use std::collections::{HashMap, HashSet};

use crate::{
    parser::{Declaration, Expr, Literal, SimpleType, Type, XenoType as AstType},
    semantic::{
        is_type_compatible, AnalyzerListener, ScopeInfo, XenoAnnotation, XenoParameterType,
        XenoType as SemanticType, BUILTIN_ANNOTATIONS, BUILTIN_TYPES,
    },
    TokenData, XenoDiagnostic,
};

#[derive(Clone)]
enum TypeHint {
    Builtin(&'static SemanticType),
    Alias(String),
}

pub struct AnnotationValidator {
    scope: ScopeInfo,
    type_aliases: HashMap<String, Vec<TypeHint>>,
    type_stack: Vec<Vec<&'static SemanticType>>,
    annotation_depth: usize,
}

impl AnnotationValidator {
    pub fn new(scope: &ScopeInfo) -> Self {
        Self {
            scope: scope.clone(),
            type_aliases: HashMap::new(),
            type_stack: Vec::new(),
            annotation_depth: 0,
        }
    }

    fn find_annotation(name: &str) -> Option<&'static XenoAnnotation> {
        BUILTIN_ANNOTATIONS
            .iter()
            .copied()
            .find(|annotation| annotation.name == name)
    }

    fn find_builtin_type(name: &str) -> Option<&'static SemanticType> {
        BUILTIN_TYPES
            .iter()
            .copied()
            .find(|builtin_type| builtin_type.name == name)
    }

    fn current_types(&self) -> &[&'static SemanticType] {
        self.type_stack.last().map_or(&[], Vec::as_slice)
    }

    fn resolve_types(&self, ty: &AstType<'_>) -> Vec<&'static SemanticType> {
        let mut types = Vec::new();
        let mut visited_aliases = HashSet::new();
        self.collect_types(&ty.0, &mut types, &mut visited_aliases);
        types
    }

    fn collect_types(
        &self,
        ty: &Type<'_>,
        types: &mut Vec<&'static SemanticType>,
        visited_aliases: &mut HashSet<String>,
    ) {
        match ty {
            Type::Simple(simple) => self.collect_simple_types(simple, types, visited_aliases),
            Type::Sum(items) | Type::Intersection(items) => {
                for item in items {
                    self.collect_simple_types(item, types, visited_aliases);
                }
            }
            Type::Tuple(_) | Type::Set(_) => Self::collect_builtin_type("any", types),
            Type::Struct(_) => Self::collect_builtin_type("dict", types),
            Type::Enum(_) => {}
        }
    }

    fn collect_simple_types(
        &self,
        simple: &SimpleType<'_>,
        types: &mut Vec<&'static SemanticType>,
        visited_aliases: &mut HashSet<String>,
    ) {
        match simple {
            SimpleType::Identifier(identifier)
            | SimpleType::OptionalIdentifier(identifier)
            | SimpleType::Array(identifier)
            | SimpleType::OptionalArray(identifier) => {
                if let Some(builtin_type) = Self::find_builtin_type(identifier.v) {
                    types.push(builtin_type);
                } else {
                    self.collect_alias_types(identifier.v, types, visited_aliases);
                }
            }
            SimpleType::Literal(literal) | SimpleType::OptionalLiteral(literal) => {
                Self::collect_literal_type(literal, types)
            }
        }
    }

    fn collect_alias_types(
        &self,
        alias: &str,
        types: &mut Vec<&'static SemanticType>,
        visited_aliases: &mut HashSet<String>,
    ) {
        if !visited_aliases.insert(alias.to_string()) {
            return;
        }

        if let Some(hints) = self.type_aliases.get(alias) {
            for hint in hints {
                match hint {
                    TypeHint::Builtin(builtin_type) => types.push(builtin_type),
                    TypeHint::Alias(next_alias) => {
                        self.collect_alias_types(next_alias, types, visited_aliases)
                    }
                }
            }
        }

        visited_aliases.remove(alias);
    }

    fn collect_literal_type(literal: &Literal<'_>, types: &mut Vec<&'static SemanticType>) {
        match literal {
            Literal::Int(_, _) | Literal::Float(_, _) => {
                Self::collect_builtin_type("number", types)
            }
            Literal::String(_, _) => Self::collect_builtin_type("string", types),
            Literal::Boolean(_, _) => Self::collect_builtin_type("bool", types),
        }
    }

    fn collect_builtin_type(name: &str, types: &mut Vec<&'static SemanticType>) {
        if let Some(builtin_type) = Self::find_builtin_type(name) {
            types.push(builtin_type);
        }
    }

    fn collect_type_hints(&self, ty: &AstType<'_>) -> Vec<TypeHint> {
        let mut hints = Vec::new();
        self.collect_type_hint(&ty.0, &mut hints);
        hints
    }

    fn collect_type_hint(&self, ty: &Type<'_>, hints: &mut Vec<TypeHint>) {
        match ty {
            Type::Simple(simple) => self.collect_simple_type_hint(simple, hints),
            Type::Sum(items) | Type::Intersection(items) => {
                for item in items {
                    self.collect_simple_type_hint(item, hints);
                }
            }
            Type::Tuple(_) | Type::Set(_) => Self::push_builtin_hint("any", hints),
            Type::Struct(_) => Self::push_builtin_hint("dict", hints),
            Type::Enum(_) => {}
        }
    }

    fn collect_simple_type_hint(&self, simple: &SimpleType<'_>, hints: &mut Vec<TypeHint>) {
        match simple {
            SimpleType::Identifier(identifier)
            | SimpleType::OptionalIdentifier(identifier)
            | SimpleType::Array(identifier)
            | SimpleType::OptionalArray(identifier) => {
                if let Some(builtin_type) = Self::find_builtin_type(identifier.v) {
                    hints.push(TypeHint::Builtin(builtin_type));
                } else {
                    hints.push(TypeHint::Alias(identifier.v.to_string()));
                }
            }
            SimpleType::Literal(literal) | SimpleType::OptionalLiteral(literal) => match literal {
                Literal::Int(_, _) | Literal::Float(_, _) => {
                    Self::push_builtin_hint("number", hints)
                }
                Literal::String(_, _) => Self::push_builtin_hint("string", hints),
                Literal::Boolean(_, _) => Self::push_builtin_hint("bool", hints),
            },
        }
    }

    fn push_builtin_hint(name: &str, hints: &mut Vec<TypeHint>) {
        if let Some(builtin_type) = Self::find_builtin_type(name) {
            hints.push(TypeHint::Builtin(builtin_type));
        }
    }

    fn validate_applicability<'src>(
        &self,
        annotation: &XenoAnnotation,
        name: &TokenData<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if self.annotation_depth > 0 {
            return;
        }

        let Some(applicable_to) = annotation.applicable_to else {
            return;
        };

        for candidate in self.current_types() {
            let compatible = applicable_to.iter().any(|target| {
                let mut visited = HashSet::new();
                is_type_compatible(candidate, target, &mut visited)
            });

            if !compatible {
                errors.push(XenoDiagnostic {
                    location: (*name).clone(),
                    message: format!(
                        "Annotation '@{}' is not applicable to type '{}'. Expected one of: {}.",
                        annotation.name,
                        candidate.name,
                        Self::format_types(applicable_to)
                    ),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }
    }

    fn validate_args<'src>(
        &self,
        annotation: &XenoAnnotation,
        name: &TokenData<'src>,
        args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        let expected_params = annotation.params.unwrap_or(&[]);
        if args.len() != expected_params.len() {
            errors.push(XenoDiagnostic {
                severity: crate::XenoDiagSeverity::Err,
                location: (*name).clone(),
                message: format!(
                    "Annotation '@{}' expects {} argument(s), got {}.",
                    annotation.name,
                    expected_params.len(),
                    args.len()
                ),
            });
            return;
        }

        for (arg, param) in args.iter().zip(expected_params.iter()) {
            if !self.arg_matches(arg, param.param_type) {
                errors.push(XenoDiagnostic {
                    severity: crate::XenoDiagSeverity::Err,
                    location: Self::expr_location(arg),
                    message: format!(
                        "Annotation '@{}' argument '{}' expects {}, got {}.",
                        annotation.name,
                        param.name,
                        Self::param_type_name(param.param_type),
                        Self::arg_type_name(arg)
                    ),
                });
            }
        }
    }

    fn arg_matches(&self, arg: &Expr<'_>, expected: XenoParameterType) -> bool {
        match expected {
            XenoParameterType::None => false,
            XenoParameterType::Expression => true,
            XenoParameterType::Identifier => {
                matches!(arg, Expr::Type(Type::Simple(SimpleType::Identifier(_))))
            }
            XenoParameterType::Type => match arg {
                Expr::Type(Type::Simple(SimpleType::Identifier(identifier))) => {
                    self.scope.has_type(identifier.v)
                }
                _ => false,
            },
            XenoParameterType::Annotation => matches!(arg, Expr::Annotation(_)),
            XenoParameterType::FieldReference => false,
            XenoParameterType::NumberLiteral => matches!(
                arg,
                Expr::Type(Type::Simple(SimpleType::Literal(
                    Literal::Int(_, _) | Literal::Float(_, _)
                )))
            ),
            XenoParameterType::IntegerLiteral => matches!(
                arg,
                Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(_, _))))
            ),
            XenoParameterType::StringLiteral => matches!(
                arg,
                Expr::Type(Type::Simple(SimpleType::Literal(Literal::String(_, _))))
            ),
            XenoParameterType::BoolLiteral => matches!(
                arg,
                Expr::Type(Type::Simple(SimpleType::Literal(Literal::Boolean(_, _))))
            ),
            XenoParameterType::AnyLiteral => matches!(
                arg,
                Expr::Type(Type::Simple(SimpleType::Literal(_))) | Expr::Regex(_)
            ),
            XenoParameterType::List(item_types) => match arg {
                Expr::Type(Type::Tuple(items)) => {
                    items.len() == item_types.len()
                        && items
                            .iter()
                            .zip(item_types.iter())
                            .all(|(item, item_type)| self.simple_arg_matches(item, *item_type))
                }
                _ => false,
            },
        }
    }

    fn simple_arg_matches(&self, arg: &SimpleType<'_>, expected: XenoParameterType) -> bool {
        match expected {
            XenoParameterType::Expression => true,
            XenoParameterType::Identifier => matches!(arg, SimpleType::Identifier(_)),
            XenoParameterType::Type => {
                matches!(arg, SimpleType::Identifier(identifier) if self.scope.has_type(identifier.v))
            }
            XenoParameterType::NumberLiteral => matches!(
                arg,
                SimpleType::Literal(Literal::Int(_, _) | Literal::Float(_, _))
            ),
            XenoParameterType::IntegerLiteral => {
                matches!(arg, SimpleType::Literal(Literal::Int(_, _)))
            }
            XenoParameterType::StringLiteral => {
                matches!(arg, SimpleType::Literal(Literal::String(_, _)))
            }
            XenoParameterType::BoolLiteral => {
                matches!(arg, SimpleType::Literal(Literal::Boolean(_, _)))
            }
            XenoParameterType::AnyLiteral => matches!(arg, SimpleType::Literal(_)),
            XenoParameterType::None
            | XenoParameterType::FieldReference
            | XenoParameterType::Annotation
            | XenoParameterType::List(_) => false,
        }
    }

    fn expr_location<'src>(expr: &Expr<'src>) -> TokenData<'src> {
        match expr {
            Expr::Regex(token) => (*token).clone(),
            Expr::Annotation(annotation) => (*annotation.ident).clone(),
            Expr::Type(ty) => Self::type_location(ty),
        }
    }

    fn type_location<'src>(ty: &Type<'src>) -> TokenData<'src> {
        match ty {
            Type::Simple(simple) => simple.get_last_token().clone(),
            Type::Tuple(items)
            | Type::Set(items)
            | Type::Sum(items)
            | Type::Intersection(items) => items
                .first()
                .map(|item| item.get_last_token().clone())
                .unwrap_or_default(),
            Type::Struct(fields) | Type::Enum(fields) => fields
                .first()
                .map(|(key, _, _)| (*key).clone())
                .unwrap_or_default(),
        }
    }

    fn arg_type_name(arg: &Expr<'_>) -> &'static str {
        match arg {
            Expr::Regex(_) => "regex literal",
            Expr::Annotation(_) => "annotation",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(_, _)))) => "integer literal",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Float(_, _)))) => "number literal",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::String(_, _)))) => {
                "string literal"
            }
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Boolean(_, _)))) => {
                "boolean literal"
            }
            Expr::Type(Type::Simple(SimpleType::OptionalLiteral(_))) => "optional literal",
            Expr::Type(Type::Simple(SimpleType::Identifier(_))) => "identifier",
            Expr::Type(Type::Simple(SimpleType::OptionalIdentifier(_))) => "optional identifier",
            Expr::Type(Type::Simple(SimpleType::Array(_) | SimpleType::OptionalArray(_))) => {
                "array"
            }
            Expr::Type(Type::Tuple(_)) => "list",
            Expr::Type(Type::Set(_)) => "set",
            Expr::Type(Type::Struct(_)) => "struct",
            Expr::Type(Type::Enum(_)) => "enum",
            Expr::Type(Type::Sum(_) | Type::Intersection(_)) => "compound expression",
        }
    }

    fn param_type_name(param_type: XenoParameterType) -> &'static str {
        match param_type {
            XenoParameterType::None => "no argument",
            XenoParameterType::NumberLiteral => "number literal",
            XenoParameterType::IntegerLiteral => "integer literal",
            XenoParameterType::StringLiteral => "string literal",
            XenoParameterType::BoolLiteral => "boolean literal",
            XenoParameterType::FieldReference => "field reference",
            XenoParameterType::AnyLiteral => "literal",
            XenoParameterType::Expression => "expression",
            XenoParameterType::Identifier => "identifier",
            XenoParameterType::Type => "type",
            XenoParameterType::Annotation => "annotation",
            XenoParameterType::List(_) => "list",
        }
    }

    fn format_types(types: &[&SemanticType]) -> String {
        types
            .iter()
            .map(|xeno_type| xeno_type.name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl<'src> AnalyzerListener<'src> for AnnotationValidator {
    fn on_before_ast(
        &mut self,
        ast: &[Declaration<'src>],
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.type_aliases.clear();

        for declaration in ast {
            if let Declaration::Type { name, ty, .. } = declaration {
                let hints = self.collect_type_hints(ty);
                self.type_aliases.insert(name.v.to_string(), hints);
            }
        }
    }

    fn on_before_type(&mut self, ty: &AstType<'src>, _errors: &mut Vec<XenoDiagnostic<'src>>) {
        self.type_stack.push(self.resolve_types(ty));
    }

    fn on_after_type(&mut self, _ty: &AstType<'src>, _errors: &mut Vec<XenoDiagnostic<'src>>) {
        self.type_stack.pop();
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if let Some(annotation) = Self::find_annotation(name.v) {
            self.validate_applicability(annotation, name, errors);
            self.validate_args(annotation, name, args, errors);
        }
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData<'src>,
        _args: &[Expr<'src>],
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use num_bigint::BigInt;

    use super::*;
    use crate::parser::Annotation;

    fn scope() -> ScopeInfo {
        ScopeInfo {
            module_path: "test".to_string(),
            abs_path: PathBuf::new(),
            own_types: vec!["A".to_string(), "B".to_string()],
            imported_types: HashMap::new(),
            builtin_types: BUILTIN_TYPES
                .iter()
                .map(|builtin_type| builtin_type.name.to_string())
                .collect(),
            known_annotations: BUILTIN_ANNOTATIONS
                .iter()
                .map(|annotation| annotation.name.to_string())
                .collect(),
        }
    }

    fn int_arg<'src>(value: &'src TokenData<'src>) -> Expr<'src> {
        Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(
            BigInt::from(5),
            value,
        ))))
    }

    #[test]
    fn annotation_applicability_resolves_custom_literal_aliases() {
        let a_name = TokenData { v: "A", l: 0, c: 5 };
        let b_name = TokenData { v: "B", l: 1, c: 5 };
        let string_literal = TokenData {
            v: "\"literal\"",
            l: 0,
            c: 9,
        };
        let a_reference = TokenData { v: "A", l: 1, c: 9 };
        let max = TokenData {
            v: "max",
            l: 1,
            c: 12,
        };
        let value = TokenData {
            v: "5",
            l: 1,
            c: 16,
        };
        let max_args = vec![int_arg(&value)];
        let b_type = (
            Type::Simple(SimpleType::Identifier(&a_reference)),
            vec![Annotation {
                ident: &max,
                params: max_args.clone(),
            }],
        );
        let from = TokenData::default();
        let ast = vec![
            Declaration::Type {
                docs: None,
                name: &a_name,
                generics: None,
                ty: (
                    Type::Simple(SimpleType::Literal(Literal::String(
                        "literal".to_string(),
                        &string_literal,
                    ))),
                    vec![],
                ),
                from: &from,
                to: &from,
            },
            Declaration::Type {
                docs: None,
                name: &b_name,
                generics: None,
                ty: b_type.clone(),
                from: &from,
                to: &from,
            },
        ];
        let mut validator = AnnotationValidator::new(&scope());
        let mut errors = Vec::new();

        validator.on_before_ast(&ast, &mut errors);
        validator.on_before_type(&b_type, &mut errors);
        validator.on_before_annotation(&max, &max_args, &mut errors);

        assert_eq!(errors.len(), 1);
        assert!(errors[0]
            .message
            .contains("Annotation '@max' is not applicable to type 'string'"));
    }

    #[test]
    fn annotation_applicability_accepts_custom_numeric_aliases() {
        let a_name = TokenData { v: "A", l: 0, c: 5 };
        let b_name = TokenData { v: "B", l: 1, c: 5 };
        let u8_type = TokenData {
            v: "u8",
            l: 0,
            c: 9,
        };
        let a_reference = TokenData { v: "A", l: 1, c: 9 };
        let max = TokenData {
            v: "max",
            l: 1,
            c: 12,
        };
        let value = TokenData {
            v: "5",
            l: 1,
            c: 16,
        };
        let max_args = vec![int_arg(&value)];
        let b_type = (
            Type::Simple(SimpleType::Identifier(&a_reference)),
            vec![Annotation {
                ident: &max,
                params: max_args.clone(),
            }],
        );
        let from = TokenData::default();
        let ast = vec![
            Declaration::Type {
                docs: None,
                name: &a_name,
                generics: None,
                ty: (Type::Simple(SimpleType::Identifier(&u8_type)), vec![]),
                from: &from,
                to: &from,
            },
            Declaration::Type {
                docs: None,
                name: &b_name,
                generics: None,
                ty: b_type.clone(),
                from: &from,
                to: &from,
            },
        ];
        let mut validator = AnnotationValidator::new(&scope());
        let mut errors = Vec::new();

        validator.on_before_ast(&ast, &mut errors);
        validator.on_before_type(&b_type, &mut errors);
        validator.on_before_annotation(&max, &max_args, &mut errors);

        assert!(errors.is_empty());
    }
}
