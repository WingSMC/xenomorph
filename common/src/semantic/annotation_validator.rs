use std::collections::HashMap;

use crate::{
    parser::{Declaration, Expr, Literal, SimpleType, Type, XenoType as AstType},
    semantic::{
        simple_to_owned_type, type_to_owned_type, AnalyzerListener, OwnedAnnotation,
        OwnedConstraint, OwnedTrait, OwnedTraitKind, OwnedType, ScopeInfo,
    },
    TokenData, XenoDiagnostic,
};

pub struct AnnotationValidator<'context> {
    scope: &'context ScopeInfo,
    source: &'context str,
    type_stack: Vec<Vec<OwnedType>>,
    generic_params: HashMap<String, Option<String>>,
    annotation_depth: usize,
}

impl<'context> AnnotationValidator<'context> {
    pub fn new(scope: &'context ScopeInfo, source: &'context str) -> Self {
        Self {
            scope,
            source,
            type_stack: Vec::new(),
            generic_params: HashMap::new(),
            annotation_depth: 0,
        }
    }

    fn current_types(&self) -> &[OwnedType] {
        self.type_stack.last().map_or(&[], Vec::as_slice)
    }

    fn resolve_types(&self, ty: &AstType) -> Vec<OwnedType> {
        let candidate = match &ty.0 {
            // Validators on a standalone optional type apply to its present
            // value, preserving the established source behavior. Composite
            // types retain optionality because it changes what `|` and `&`
            // can guarantee as a whole.
            Type::Simple(simple) => simple_to_owned_type(simple.inner(), self.source),
            ty => type_to_owned_type(ty, self.source),
        };
        vec![self.resolve_generic_references(candidate)]
    }

    fn resolve_generic_references(&self, candidate: OwnedType) -> OwnedType {
        match candidate {
            OwnedType::Named { name, arguments } if arguments.is_empty() => {
                if let Some(constraint) = self.generic_params.get(&name) {
                    OwnedType::Generic {
                        name,
                        constraint: constraint.clone(),
                        constraint_scope: Some(self.scope.module_path.clone()),
                    }
                } else {
                    OwnedType::named(name)
                }
            }
            OwnedType::Named { name, arguments } => OwnedType::Named {
                name,
                arguments: arguments
                    .into_iter()
                    .map(|argument| self.resolve_generic_references(argument))
                    .collect(),
            },
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => OwnedType::Qualified {
                module_path,
                name,
                arguments: arguments
                    .into_iter()
                    .map(|argument| self.resolve_generic_references(argument))
                    .collect(),
            },
            OwnedType::Array(inner) => {
                OwnedType::Array(Box::new(self.resolve_generic_references(*inner)))
            }
            OwnedType::Optional(inner) => {
                OwnedType::Optional(Box::new(self.resolve_generic_references(*inner)))
            }
            OwnedType::Tuple(items) => OwnedType::Tuple(
                items
                    .into_iter()
                    .map(|item| self.resolve_generic_references(item))
                    .collect(),
            ),
            OwnedType::Set(mut set) => {
                set.element_type = set
                    .element_type
                    .map(|element| Box::new(self.resolve_generic_references(*element)));
                OwnedType::Set(set)
            }
            OwnedType::Struct(fields) => OwnedType::Struct(
                fields
                    .into_iter()
                    .map(|mut field| {
                        field.ty = self.resolve_generic_references(field.ty);
                        field
                    })
                    .collect(),
            ),
            OwnedType::Enum(variants) => OwnedType::Enum(
                variants
                    .into_iter()
                    .map(|mut variant| {
                        variant.ty = self.resolve_generic_references(variant.ty);
                        variant
                    })
                    .collect(),
            ),
            OwnedType::Sum(items) => OwnedType::Sum(
                items
                    .into_iter()
                    .map(|item| self.resolve_generic_references(item))
                    .collect(),
            ),
            OwnedType::Intersection(items) => OwnedType::Intersection(
                items
                    .into_iter()
                    .map(|item| self.resolve_generic_references(item))
                    .collect(),
            ),
            OwnedType::Literal(_) | OwnedType::Generic { .. } => candidate,
        }
    }

    fn validate_applicability(
        &self,
        annotation: &OwnedAnnotation,
        name: &TokenData,
        errors: &mut Vec<XenoDiagnostic>,
    ) {
        if self.annotation_depth > 0 {
            return;
        }

        let Some(target) = annotation.target_parameter() else {
            return;
        };

        for candidate in self.current_types() {
            if !self.type_matches_constraint(candidate, &target.constraint) {
                errors.push(XenoDiagnostic {
                    tags: vec![],
                    location: (*name),
                    message: format!(
                        "Annotation '@{}' is not applicable to type '{}'. Required {}(s): {}.",
                        annotation.name,
                        candidate.display_name(),
                        Self::constraint_kind(&target.constraint),
                        target.constraint.name()
                    ),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }
    }

    fn validate_args(
        &self,
        annotation: &OwnedAnnotation,
        name: &TokenData,
        args: &[Expr],
        errors: &mut Vec<XenoDiagnostic>,
    ) {
        let expected_params = annotation.explicit_parameters();
        let invalid_count = if annotation.variadic {
            args.len() < expected_params.len()
        } else {
            args.len() != expected_params.len()
        };
        if invalid_count {
            errors.push(XenoDiagnostic {
                tags: vec![],
                severity: crate::XenoDiagSeverity::Err,
                location: (*name),
                message: format!(
                    "Annotation '@{}' expects {}{} argument(s), got {}.",
                    annotation.name,
                    expected_params.len(),
                    if annotation.variadic { " or more" } else { "" },
                    args.len()
                ),
            });
            return;
        }

        for (index, arg) in args.iter().enumerate() {
            let Some(param) = annotation.parameter_at(index) else {
                continue;
            };
            if !self.arg_matches_constraint(arg, &param.constraint) {
                errors.push(XenoDiagnostic {
                    tags: vec![],
                    severity: crate::XenoDiagSeverity::Err,
                    location: Self::expr_location(arg),
                    message: format!(
                        "Annotation '@{}' argument '{}' expects {}, got {}.",
                        annotation.name,
                        param.name,
                        param.constraint.name(),
                        Self::arg_type_name(arg)
                    ),
                });
            }
        }
    }

    fn type_matches_constraint(&self, candidate: &OwnedType, required: &OwnedConstraint) -> bool {
        match required {
            OwnedConstraint::Type(required) => {
                self.scope.descends_from_semantic_type(candidate, required)
            }
            OwnedConstraint::Trait(required) => self.scope.implements_trait(candidate, required),
        }
    }

    fn arg_matches_constraint(&self, arg: &Expr, required: &OwnedConstraint) -> bool {
        match required {
            OwnedConstraint::Type(required) => match arg {
                Expr::Type(Type::Simple(simple)) => self.scope.descends_from_semantic_type(
                    &simple_to_owned_type(simple, self.source),
                    required,
                ),
                _ => false,
            },
            OwnedConstraint::Trait(required) => self
                .scope
                .type_hierarchy
                .get_trait(required)
                .is_some_and(|required| self.arg_matches_trait(arg, required)),
        }
    }

    fn constraint_kind(constraint: &OwnedConstraint) -> &'static str {
        match constraint {
            OwnedConstraint::Type(_) => "type",
            OwnedConstraint::Trait(_) => "trait",
        }
    }

    fn arg_matches_trait(&self, arg: &Expr, required: &OwnedTrait) -> bool {
        match &required.kind {
            OwnedTraitKind::Expression => true,
            OwnedTraitKind::Literal => matches!(
                arg,
                Expr::Regex(_) | Expr::Type(Type::Simple(SimpleType::Literal(_)))
            ),
            OwnedTraitKind::LiteralType => match arg {
                Expr::Type(Type::Simple(SimpleType::Literal(literal))) => {
                    let candidate = match literal {
                        Literal::Int(_) | Literal::Float(_) => {
                            OwnedType::named(literal.semantic_type_name(self.source))
                        }
                        Literal::String(_, _) => OwnedType::named("string"),
                        Literal::Boolean(_, _) => OwnedType::named("bool"),
                    };
                    self.scope.implements_trait(&candidate, &required.name)
                }
                _ => false,
            },
            OwnedTraitKind::RegexLiteral => matches!(arg, Expr::Regex(_)),
            OwnedTraitKind::Identifier => {
                matches!(arg, Expr::Type(Type::Simple(SimpleType::Identifier(_, _))))
            }
            OwnedTraitKind::Type => match arg {
                Expr::Type(Type::Simple(SimpleType::Identifier(identifier, _))) => {
                    self.scope.has_type(identifier.text(self.source))
                }
                _ => false,
            },
            OwnedTraitKind::Annotation => matches!(arg, Expr::Annotation(_)),
            OwnedTraitKind::Semantic => match arg {
                Expr::Type(Type::Simple(simple)) => self
                    .scope
                    .implements_trait(&simple_to_owned_type(simple, self.source), &required.name),
                _ => false,
            },
            OwnedTraitKind::Struct => matches!(arg, Expr::Type(Type::Struct(_))),
            OwnedTraitKind::Sum(member) => match arg {
                Expr::Type(Type::Sum(items)) => self
                    .scope
                    .type_hierarchy
                    .get_trait(member)
                    .is_some_and(|member| {
                        items.iter().all(|item| {
                            self.arg_matches_trait(&Expr::Type(Type::Simple(item.clone())), member)
                        })
                    }),
                _ => false,
            },
        }
    }

    fn expr_location(expr: &Expr) -> TokenData {
        match expr {
            Expr::Regex(token) => *token,
            Expr::Annotation(annotation) => annotation.ident,
            Expr::Type(ty) => Self::type_location(ty),
        }
    }

    fn type_location(ty: &Type) -> TokenData {
        match ty {
            Type::Simple(simple) => simple.get_last_token(),
            Type::Tuple(items) | Type::Sum(items) | Type::Intersection(items) => items
                .first()
                .map(|item| item.get_last_token())
                .unwrap_or_default(),
            Type::Set(set) => set.last_token,
            Type::Struct(fields) | Type::Enum(fields) => {
                fields.first().map(|(key, _, _)| *key).unwrap_or_default()
            }
        }
    }

    fn arg_type_name(arg: &Expr) -> &'static str {
        match arg {
            Expr::Regex(_) => "regex literal",
            Expr::Annotation(_) => "annotation",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(_)))) => "integer literal",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Float(_)))) => "number literal",
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::String(_, _)))) => {
                "string literal"
            }
            Expr::Type(Type::Simple(SimpleType::Literal(Literal::Boolean(_, _)))) => {
                "boolean literal"
            }
            Expr::Type(Type::Simple(SimpleType::Optional(_))) => "optional type",
            Expr::Type(Type::Simple(SimpleType::Identifier(_, _))) => "identifier",
            Expr::Type(Type::Simple(SimpleType::Array(_, _))) => "array",
            Expr::Type(Type::Tuple(_)) => "list",
            Expr::Type(Type::Set(_)) => "set",
            Expr::Type(Type::Struct(_)) => "struct",
            Expr::Type(Type::Enum(_)) => "enum",
            Expr::Type(Type::Sum(_) | Type::Intersection(_)) => "compound expression",
        }
    }
}

impl<'src, 'context> AnalyzerListener<'src> for AnnotationValidator<'context> {
    fn on_before_type(&mut self, ty: &AstType, _errors: &mut Vec<XenoDiagnostic>) {
        self.type_stack.push(self.resolve_types(ty));
    }

    fn on_before_decl(&mut self, declaration: &Declaration, _errors: &mut Vec<XenoDiagnostic>) {
        self.generic_params.clear();
        if let Declaration::Type { generics, .. } = declaration {
            self.generic_params
                .extend(
                    generics
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .map(|(name, constraint)| {
                            (
                                name.text(self.source).to_string(),
                                constraint
                                    .map(|constraint| constraint.text(self.source).to_string()),
                            )
                        }),
                );
        }
    }

    fn on_after_decl(&mut self, _declaration: &Declaration, _errors: &mut Vec<XenoDiagnostic>) {
        self.generic_params.clear();
    }

    fn on_after_type(&mut self, _ty: &AstType, _errors: &mut Vec<XenoDiagnostic>) {
        self.type_stack.pop();
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData,
        args: &[Expr],
        errors: &mut Vec<XenoDiagnostic>,
    ) {
        if let Some(annotation) = self.scope.find_annotation(name.text(self.source)) {
            self.validate_applicability(annotation, name, errors);
            self.validate_args(annotation, name, args, errors);
        }
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData,
        _args: &[Expr],
        _errors: &mut Vec<XenoDiagnostic>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use num_bigint::BigInt;

    use super::*;
    use crate::parser::{Annotation, IntLiteral, IntegerRepresentation, IntegerSize};
    use crate::semantic::{TypeDeclarationInfo, TypeHierarchy, BUILTIN_ANNOTATIONS, BUILTIN_TYPES};

    fn scope(ast: &[Declaration], source: &str) -> ScopeInfo {
        let own_types = ast
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Type { name, .. } => Some(name.text(source).to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut type_hierarchy = TypeHierarchy::default();
        type_hierarchy.set_current_module("test");
        type_hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            type_hierarchy.insert_semantic_type(semantic_type);
        }
        for declaration in ast {
            if let Declaration::Type {
                name, generics, ty, ..
            } = declaration
            {
                type_hierarchy.insert_declaration(
                    "test",
                    name.text(source),
                    TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
                );
            }
        }
        ScopeInfo {
            module_path: "test".to_string(),
            abs_path: PathBuf::new(),
            own_types,
            imported_types: HashMap::new(),
            builtin_types: BUILTIN_TYPES
                .iter()
                .map(|builtin_type| builtin_type.name.to_string())
                .collect(),
            known_annotations: BUILTIN_ANNOTATIONS
                .iter()
                .map(|annotation| annotation.name.to_string())
                .collect(),
            type_hierarchy,
            annotations: crate::semantic::MetadataGraph::from_descriptors(&[], BUILTIN_ANNOTATIONS)
                .annotations,
        }
    }

    fn int_arg(value: &TokenData) -> Expr {
        Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(
            IntLiteral {
                value: BigInt::from(5),
                representation: IntegerRepresentation {
                    signed: false,
                    size: IntegerSize::Bits(3),
                },
                token: *value,
                cast: None,
            },
        ))))
    }

    #[test]
    fn annotation_applicability_resolves_custom_literal_aliases() {
        let source = "A B \"literal\" A max 5";
        let a_name = TokenData::new(0, 1);
        let b_name = TokenData::new(2, 3);
        let string_literal = TokenData::new(4, 13);
        let a_reference = TokenData::new(14, 15);
        let max = TokenData::new(16, 19);
        let value = TokenData::new(20, 21);
        let max_args = vec![int_arg(&value)];
        let b_type = (
            Type::Simple(SimpleType::Identifier(a_reference, None)),
            vec![Annotation {
                ident: max,
                params: max_args.clone(),
            }],
        );
        let from = TokenData::default();
        let ast = vec![
            Declaration::Type {
                docs: None,
                public: None,
                name: a_name,
                generics: None,
                ty: (
                    Type::Simple(SimpleType::Literal(Literal::String(
                        "literal".to_string(),
                        string_literal,
                    ))),
                    vec![],
                ),
                from,
                to: from,
            },
            Declaration::Type {
                docs: None,
                public: None,
                name: b_name,
                generics: None,
                ty: b_type.clone(),
                from,
                to: from,
            },
        ];
        let scope = scope(&ast, source);
        let mut validator = AnnotationValidator::new(&scope, source);
        let mut errors = Vec::new();

        validator.on_before_ast(&ast, &mut errors);
        validator.on_before_type(&b_type, &mut errors);
        validator.on_before_annotation(&max, &max_args, &mut errors);

        assert_eq!(errors.len(), 1);
        assert!(errors[0]
            .message
            .contains("Annotation '@max' is not applicable to type 'A'"));
    }

    #[test]
    fn annotation_applicability_accepts_custom_numeric_aliases() {
        let source = "A B u8 A max 5";
        let a_name = TokenData::new(0, 1);
        let b_name = TokenData::new(2, 3);
        let u8_type = TokenData::new(4, 6);
        let a_reference = TokenData::new(7, 8);
        let max = TokenData::new(9, 12);
        let value = TokenData::new(13, 14);
        let max_args = vec![int_arg(&value)];
        let b_type = (
            Type::Simple(SimpleType::Identifier(a_reference, None)),
            vec![Annotation {
                ident: max,
                params: max_args.clone(),
            }],
        );
        let from = TokenData::default();
        let ast = vec![
            Declaration::Type {
                docs: None,
                public: None,
                name: a_name,
                generics: None,
                ty: (Type::Simple(SimpleType::Identifier(u8_type, None)), vec![]),
                from,
                to: from,
            },
            Declaration::Type {
                docs: None,
                public: None,
                name: b_name,
                generics: None,
                ty: b_type.clone(),
                from,
                to: from,
            },
        ];
        let scope = scope(&ast, source);
        let mut validator = AnnotationValidator::new(&scope, source);
        let mut errors = Vec::new();

        validator.on_before_ast(&ast, &mut errors);
        validator.on_before_type(&b_type, &mut errors);
        validator.on_before_annotation(&max, &max_args, &mut errors);

        assert!(errors.is_empty());
    }
}
