use std::collections::{HashMap, HashSet};

use crate::{
    parser::{AnonymType, BinaryExpr, Declaration, Expr, Literal, NumberType, TypeList},
    semantic::{
        is_type_compatible, AnalyzerListener, ScopeInfo, XenoAnnotation, XenoParameterType,
        XenoType, BUILTIN_ANNOTATIONS, BUILTIN_TYPES,
    },
    TokenData, XenoError,
};

#[derive(Clone)]
enum TypeHint {
    Builtin(&'static XenoType),
    Alias(String),
}

pub struct AnnotationValidator {
    scope: ScopeInfo,
    type_aliases: HashMap<String, Vec<TypeHint>>,
    type_stack: Vec<Vec<&'static XenoType>>,
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

    fn find_annotation(&self, name: &str) -> Option<&'static XenoAnnotation> {
        BUILTIN_ANNOTATIONS
            .iter()
            .copied()
            .find(|annotation| annotation.name == name)
    }

    fn find_builtin_type(&self, name: &str) -> Option<&'static XenoType> {
        BUILTIN_TYPES
            .iter()
            .copied()
            .find(|builtin_type| builtin_type.name == name)
    }

    fn current_types(&self) -> &[&'static XenoType] {
        self.type_stack.last().map_or(&[], Vec::as_slice)
    }

    fn resolve_types(&self, exprs: &AnonymType<'_>) -> Vec<&'static XenoType> {
        let mut types = Vec::new();
        let mut visited_aliases = HashSet::new();
        for expr in exprs {
            self.collect_types(expr, &mut types, &mut visited_aliases);
        }
        types
    }

    fn collect_types(
        &self,
        expr: &Expr<'_>,
        types: &mut Vec<&'static XenoType>,
        visited_aliases: &mut HashSet<String>,
    ) {
        match expr {
            Expr::Identifier(identifier) => {
                if let Some(builtin_type) = self.find_builtin_type(identifier.v) {
                    types.push(builtin_type);
                } else {
                    self.collect_alias_types(identifier.v, types, visited_aliases);
                }
            }
            Expr::BinaryExpr(_, pair) => {
                self.collect_binary_types(pair, types, visited_aliases);
            }
            Expr::Not(inner) => self.collect_types(inner, types, visited_aliases),
            Expr::Literal(literal) => self.collect_literal_type(literal, types),
            Expr::Regex(_) => self.collect_builtin_type("regex", types),
            Expr::List(_) => self.collect_builtin_type("any", types),
            Expr::Set(_) => self.collect_builtin_type("any", types),
            Expr::Struct(_) => self.collect_builtin_type("dict", types),
            Expr::Annotation(_, _) | Expr::FieldAccess(_) | Expr::Enum(_) => {}
        }
    }

    fn collect_binary_types(
        &self,
        pair: &BinaryExpr<'_>,
        types: &mut Vec<&'static XenoType>,
        visited_aliases: &mut HashSet<String>,
    ) {
        self.collect_types(&pair.0, types, visited_aliases);
        self.collect_types(&pair.1, types, visited_aliases);
    }

    fn collect_alias_types(
        &self,
        alias: &str,
        types: &mut Vec<&'static XenoType>,
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

    fn collect_literal_type(&self, literal: &Literal<'_>, types: &mut Vec<&'static XenoType>) {
        match literal {
            Literal::Number(_) => self.collect_builtin_type("number", types),
            Literal::String(_, _) => self.collect_builtin_type("string", types),
            Literal::Boolean(_, _) => self.collect_builtin_type("bool", types),
        }
    }

    fn collect_builtin_type(&self, name: &str, types: &mut Vec<&'static XenoType>) {
        if let Some(builtin_type) = self.find_builtin_type(name) {
            types.push(builtin_type);
        }
    }

    fn collect_type_hints(&self, exprs: &AnonymType<'_>) -> Vec<TypeHint> {
        let mut hints = Vec::new();
        for expr in exprs {
            self.collect_type_hint(expr, &mut hints);
        }
        hints
    }

    fn collect_type_hint(&self, expr: &Expr<'_>, hints: &mut Vec<TypeHint>) {
        match expr {
            Expr::Identifier(identifier) => {
                if let Some(builtin_type) = self.find_builtin_type(identifier.v) {
                    hints.push(TypeHint::Builtin(builtin_type));
                } else {
                    hints.push(TypeHint::Alias(identifier.v.to_string()));
                }
            }
            Expr::Literal(literal) => match literal {
                Literal::Number(_) => self.push_builtin_hint("number", hints),
                Literal::String(_, _) => self.push_builtin_hint("string", hints),
                Literal::Boolean(_, _) => self.push_builtin_hint("bool", hints),
            },
            Expr::Regex(_) => self.push_builtin_hint("regex", hints),
            Expr::BinaryExpr(_, pair) => {
                self.collect_type_hint(&pair.0, hints);
                self.collect_type_hint(&pair.1, hints);
            }
            Expr::Not(inner) => self.collect_type_hint(inner, hints),
            Expr::List(_) | Expr::Set(_) => self.push_builtin_hint("any", hints),
            Expr::Struct(_) => self.push_builtin_hint("dict", hints),
            Expr::Annotation(_, _) | Expr::FieldAccess(_) | Expr::Enum(_) => {}
        }
    }

    fn push_builtin_hint(&self, name: &str, hints: &mut Vec<TypeHint>) {
        if let Some(builtin_type) = self.find_builtin_type(name) {
            hints.push(TypeHint::Builtin(builtin_type));
        }
    }

    fn validate_applicability<'src>(
        &self,
        annotation: &XenoAnnotation,
        name: &TokenData<'src>,
        errors: &mut Vec<XenoError<'src>>,
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
                errors.push(XenoError {
                    location: (*name).clone(),
                    message: format!(
                        "Annotation '@{}' is not applicable to type '{}'. Expected one of: {}.",
                        annotation.name,
                        candidate.name,
                        Self::format_types(applicable_to)
                    ),
                });
            }
        }
    }

    fn validate_args<'src>(
        &self,
        annotation: &XenoAnnotation,
        name: &TokenData<'src>,
        args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
        let expected_params = annotation.params.unwrap_or(&[]);
        if args.len() != expected_params.len() {
            errors.push(XenoError {
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
                errors.push(XenoError {
                    location: Self::arg_location(arg).unwrap_or_else(|| (*name).clone()),
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

    fn arg_matches(&self, arg: &AnonymType<'_>, expected: XenoParameterType) -> bool {
        match expected {
            XenoParameterType::None => arg.is_empty(),
            XenoParameterType::Expression => !arg.is_empty(),
            XenoParameterType::Identifier => {
                matches!(arg.as_slice(), [Expr::Identifier(_)])
            }
            XenoParameterType::Type => match arg.as_slice() {
                [Expr::Identifier(identifier)] => self.scope.has_type(identifier.v),
                _ => false,
            },
            XenoParameterType::Annotation => {
                matches!(arg.as_slice(), [Expr::Annotation(_, _)])
            }
            XenoParameterType::FieldReference => {
                matches!(arg.as_slice(), [Expr::FieldAccess(_)])
            }
            XenoParameterType::NumberLiteral => matches!(
                arg.as_slice(),
                [Expr::Literal(Literal::Number(
                    NumberType::Int(_, _) | NumberType::Float(_, _)
                ))]
            ),
            XenoParameterType::IntegerLiteral => {
                matches!(
                    arg.as_slice(),
                    [Expr::Literal(Literal::Number(NumberType::Int(_, _)))]
                )
            }
            XenoParameterType::StringLiteral => {
                matches!(arg.as_slice(), [Expr::Literal(Literal::String(_, _))])
            }
            XenoParameterType::BoolLiteral => {
                matches!(arg.as_slice(), [Expr::Literal(Literal::Boolean(_, _))])
            }
            XenoParameterType::AnyLiteral => {
                matches!(arg.as_slice(), [Expr::Literal(_) | Expr::Regex(_)])
            }
            XenoParameterType::List(item_types) => match arg.as_slice() {
                [Expr::List(items)] => {
                    items.len() == item_types.len()
                        && items
                            .iter()
                            .zip(item_types.iter())
                            .all(|(item, item_type)| self.arg_matches(item, *item_type))
                }
                _ => false,
            },
        }
    }

    fn arg_location<'src>(arg: &AnonymType<'src>) -> Option<TokenData<'src>> {
        arg.first().map(Self::expr_location)
    }

    fn expr_location<'src>(expr: &Expr<'src>) -> TokenData<'src> {
        match expr {
            Expr::Identifier(token)
            | Expr::Regex(token)
            | Expr::Annotation(token, _)
            | Expr::FieldAccess(token) => (*token).clone(),
            Expr::Literal(Literal::Number(
                NumberType::Int(_, token) | NumberType::Float(_, token),
            ))
            | Expr::Literal(Literal::String(_, token))
            | Expr::Literal(Literal::Boolean(_, token)) => (*token).clone(),
            Expr::Not(inner) => Self::expr_location(inner),
            Expr::BinaryExpr(_, pair) => Self::expr_location(&pair.0),
            Expr::List(items) | Expr::Set(items) => items
                .first()
                .and_then(Self::arg_location)
                .unwrap_or(TokenData { v: "", l: 0, c: 0 }),
            Expr::Struct(fields) | Expr::Enum(fields) => fields
                .first()
                .map(|(key, _)| (*key).clone())
                .unwrap_or(TokenData { v: "", l: 0, c: 0 }),
        }
    }

    fn arg_type_name(arg: &AnonymType<'_>) -> &'static str {
        match arg.as_slice() {
            [] => "no argument",
            [Expr::Literal(Literal::Number(NumberType::Int(_, _)))] => "integer literal",
            [Expr::Literal(Literal::Number(NumberType::Float(_, _)))] => "number literal",
            [Expr::Literal(Literal::String(_, _))] => "string literal",
            [Expr::Literal(Literal::Boolean(_, _))] => "boolean literal",
            [Expr::Regex(_)] => "regex literal",
            [Expr::FieldAccess(_)] => "field reference",
            [Expr::Identifier(_)] => "identifier",
            [Expr::Annotation(_, _)] => "annotation",
            [Expr::List(_)] => "list",
            [Expr::Set(_)] => "set",
            [Expr::Struct(_)] => "struct",
            [Expr::Enum(_)] => "enum",
            [Expr::Not(_)] | [Expr::BinaryExpr(_, _)] => "expression",
            _ => "compound expression",
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

    fn format_types(types: &[&XenoType]) -> String {
        types
            .iter()
            .map(|xeno_type| xeno_type.name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl<'src> AnalyzerListener<'src> for AnnotationValidator {
    fn on_before_ast(&mut self, ast: &[Declaration<'src>], _errors: &mut Vec<XenoError<'src>>) {
        self.type_aliases.clear();

        for declaration in ast {
            if let Declaration::TypeDecl { name, t, .. } = declaration {
                let hints = self.collect_type_hints(t);
                self.type_aliases.insert(name.v.to_string(), hints);
            }
        }
    }

    fn on_before_type(&mut self, exprs: &AnonymType<'src>, _errors: &mut Vec<XenoError<'src>>) {
        self.type_stack.push(self.resolve_types(exprs));
    }

    fn on_after_type(&mut self, _exprs: &AnonymType<'src>, _errors: &mut Vec<XenoError<'src>>) {
        self.type_stack.pop();
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
        if let Some(annotation) = self.find_annotation(name.v) {
            self.validate_applicability(annotation, name, errors);
            self.validate_args(annotation, name, args, errors);
        }
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData<'src>,
        _args: &TypeList<'src>,
        _errors: &mut Vec<XenoError<'src>>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;
    use crate::parser::NumberType;

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
        let max_args = vec![vec![Expr::Literal(Literal::Number(NumberType::Int(
            5, &value,
        )))]];
        let b_type = vec![
            Expr::Identifier(&a_reference),
            Expr::Annotation(&max, max_args.clone()),
        ];
        let ast = vec![
            Declaration::TypeDecl {
                docs: None,
                name: &a_name,
                t: vec![Expr::Literal(Literal::String(
                    "literal".to_string(),
                    &string_literal,
                ))],
            },
            Declaration::TypeDecl {
                docs: None,
                name: &b_name,
                t: b_type.clone(),
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
        let max_args = vec![vec![Expr::Literal(Literal::Number(NumberType::Int(
            5, &value,
        )))]];
        let b_type = vec![
            Expr::Identifier(&a_reference),
            Expr::Annotation(&max, max_args.clone()),
        ];
        let ast = vec![
            Declaration::TypeDecl {
                docs: None,
                name: &a_name,
                t: vec![Expr::Identifier(&u8_type)],
            },
            Declaration::TypeDecl {
                docs: None,
                name: &b_name,
                t: b_type.clone(),
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
