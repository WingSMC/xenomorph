use crate::{
    parser::{Declaration, Expr, SetType, SimpleType},
    semantic::{literal_to_owned, simple_to_owned_type, AnalyzerListener, OwnedType, ScopeInfo},
    TokenData, XenoDiagnostic,
};
use std::collections::HashMap;

/// Reports unknown type identifiers and unknown annotation names.
pub struct NameValidator {
    scope: ScopeInfo,
    generic_params: HashMap<String, Option<String>>,
}

impl NameValidator {
    pub fn new(scope: &ScopeInfo) -> Self {
        Self {
            scope: scope.clone(),
            generic_params: HashMap::new(),
        }
    }

    fn validate_known_type<'src>(
        &self,
        name: &TokenData<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) -> bool {
        if self.scope.has_type(name.v) || self.generic_params.contains_key(name.v) {
            true
        } else {
            errors.push(XenoDiagnostic {
                location: (*name).clone(),
                message: format!("Unknown type '{}'", name.v),
                severity: crate::XenoDiagSeverity::Err,
            });
            false
        }
    }

    fn validate_known_constraint<'src>(
        &self,
        name: &TokenData<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) -> bool {
        if self.scope.has_constraint(name.v) {
            true
        } else {
            errors.push(XenoDiagnostic {
                location: (*name).clone(),
                message: format!("Unknown type or trait '{}'", name.v),
                severity: crate::XenoDiagSeverity::Err,
            });
            false
        }
    }

    fn resolved_argument(&self, argument: &SimpleType<'_>) -> OwnedType {
        self.resolve_generic_references(simple_to_owned_type(argument))
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

    fn validate_specialization<'src>(
        &self,
        name: &TokenData<'src>,
        arguments: Option<&[SimpleType<'src>]>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if !self.validate_known_type(name, errors) {
            return;
        }

        let Some(parameters) = self.scope.generic_parameters(name.v) else {
            return;
        };
        let arguments = arguments.unwrap_or(&[]);
        if parameters.len() != arguments.len() {
            errors.push(XenoDiagnostic {
                location: (*name).clone(),
                message: format!(
                    "Type '{}' expects {} generic argument(s), got {}.",
                    name.v,
                    parameters.len(),
                    arguments.len()
                ),
                severity: crate::XenoDiagSeverity::Err,
            });
            return;
        }

        let mut constraints_are_valid = true;
        for (parameter, argument) in parameters.iter().zip(arguments) {
            let Some(constraint) = parameter.constraint.as_deref() else {
                continue;
            };
            let candidate = self.resolved_argument(argument);
            if !self.scope.satisfies_constraint(
                &candidate,
                constraint,
                parameter.constraint_scope.as_deref(),
            ) {
                constraints_are_valid = false;
                errors.push(XenoDiagnostic {
                    location: argument.get_last_token().clone(),
                    message: format!(
                        "Generic argument '{}' for '{}.{}' does not satisfy constraint '{}'.",
                        candidate.display_name(),
                        name.v,
                        parameter.name,
                        constraint
                    ),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }

        if constraints_are_valid {
            let candidate = OwnedType::Named {
                name: name.v.to_string(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.resolved_argument(argument))
                    .collect(),
            };
            if let Err(error) = self.scope.type_hierarchy.evaluate_type(&candidate) {
                errors.push(XenoDiagnostic {
                    location: (*name).clone(),
                    message: error.to_string(),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }
    }
}

impl<'src> AnalyzerListener<'src> for NameValidator {
    fn on_before_decl(
        &mut self,
        declaration: &Declaration<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.generic_params.clear();
        if let Declaration::Type { generics, .. } = declaration {
            for (name, constraint) in generics.as_deref().unwrap_or(&[]) {
                if let Some(constraint) = constraint {
                    self.validate_known_constraint(constraint, errors);
                }
                self.generic_params.insert(
                    name.v.to_string(),
                    constraint.map(|constraint| constraint.v.to_string()),
                );
            }
        }
    }

    fn on_after_decl(
        &mut self,
        _declaration: &Declaration<'src>,
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.generic_params.clear();
    }

    fn on_simple_type(&mut self, ty: &SimpleType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        match ty.inner() {
            SimpleType::Identifier(id, arguments) | SimpleType::Array(id, arguments) => {
                if !self.validate_known_type(id, errors) {
                    return;
                }

                if self.generic_params.contains_key(id.v) {
                    if arguments.is_some() {
                        errors.push(XenoDiagnostic {
                            location: (*id).clone(),
                            message: format!("Generic parameter '{}' cannot take arguments.", id.v),
                            severity: crate::XenoDiagSeverity::Err,
                        });
                    }
                } else {
                    self.validate_specialization(id, arguments.as_deref(), errors);
                }
            }
            SimpleType::Literal(_) | SimpleType::Optional(_) => {}
        }
    }

    fn on_before_set(&mut self, set: &SetType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        let values = set.values.as_deref().unwrap_or_default();
        // An untyped prefill adopts its first value's narrowest type, so a
        // wider member forces an explicit `set<T>`.
        let target = match &set.element_type {
            Some(element_type) => {
                Some(self.resolve_generic_references(simple_to_owned_type(element_type)))
            }
            None => values
                .first()
                .map(|literal| OwnedType::named(literal_to_owned(literal).narrowest_type_name())),
        };

        let mut seen = Vec::new();
        for literal in values {
            let value = literal_to_owned(literal);
            if seen.iter().any(|previous: &crate::semantic::OwnedLiteral| {
                previous.same_constant_value(&value)
            }) {
                errors.push(XenoDiagnostic {
                    location: literal.token().clone(),
                    message: format!(
                        "Duplicate set value '{}'. Set values must be unique.",
                        literal.source_text()
                    ),
                    severity: crate::XenoDiagSeverity::Err,
                });
            } else {
                seen.push(value.clone());
            }

            let Some(target) = &target else {
                continue;
            };
            let narrowest = OwnedType::named(value.narrowest_type_name());
            let hierarchy = &self.scope.type_hierarchy;
            if !hierarchy.is_assignable_to(&OwnedType::Literal(value), target)
                && !hierarchy.is_assignable_to(&narrowest, target)
            {
                errors.push(XenoDiagnostic {
                    location: literal.get_last_token().clone(),
                    message: format!(
                        "Set value '{}' is not compatible with element type '{}'.",
                        literal.source_text(),
                        target.display_name()
                    ),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        _args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if !self.scope.has_annotation(name.v) {
            errors.push(XenoDiagnostic {
                location: (*name).clone(),
                message: format!("Unknown annotation '@{}'", name.v),
                severity: crate::XenoDiagSeverity::Warn,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;
    use crate::parser::{IntLiteral, IntegerRepresentation, IntegerSize, Literal, SetType};
    use crate::semantic::{TypeHierarchy, BUILTIN_TYPES};

    fn scope() -> ScopeInfo {
        let mut type_hierarchy = TypeHierarchy::default();
        type_hierarchy.set_current_module("test");
        type_hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            type_hierarchy.insert_semantic_type(semantic_type);
        }
        ScopeInfo {
            module_path: "test".to_string(),
            abs_path: PathBuf::new(),
            own_types: Vec::new(),
            imported_types: HashMap::new(),
            builtin_types: BUILTIN_TYPES
                .iter()
                .map(|semantic_type| semantic_type.name.to_string())
                .collect(),
            known_annotations: HashSet::new(),
            type_hierarchy,
            annotations: HashMap::new(),
        }
    }

    fn token(value: &str) -> TokenData<'_> {
        TokenData {
            v: value,
            l: 0,
            c: 0,
        }
    }

    fn integer<'src>(
        value: i64,
        bits: u64,
        token: &'src TokenData<'src>,
        cast: Option<&'src TokenData<'src>>,
    ) -> Literal<'src> {
        Literal::Int(IntLiteral {
            value: value.into(),
            representation: IntegerRepresentation {
                signed: false,
                size: IntegerSize::Bits(bits),
            },
            token,
            cast,
        })
    }

    fn validate<'src>(set: &SetType<'src>) -> Vec<XenoDiagnostic<'src>> {
        let mut errors = Vec::new();
        NameValidator::new(&scope()).on_before_set(set, &mut errors);
        errors
    }

    #[test]
    fn duplicate_values_and_element_type_violations_are_both_reported() {
        let keyword = token("set");
        let element = token("string");
        let first = token("\"a\"");
        let duplicate = token("\"a\"");
        let number = token("123");
        let closing = token("]");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: Some(SimpleType::Identifier(&element, None)),
            values: Some(vec![
                Literal::String("a".to_string(), &first),
                Literal::String("a".to_string(), &duplicate),
                integer(123, 7, &number, None),
            ]),
            last_token: &closing,
        });

        assert_eq!(errors.len(), 2, "{errors:#?}");
        assert!(errors[0].message.contains("Duplicate set value '\"a\"'"));
        assert!(errors[1]
            .message
            .contains("is not compatible with element type 'string'"));
        assert_eq!(errors[1].location.v, "123");
    }

    #[test]
    fn unique_values_matching_the_element_type_are_accepted() {
        let keyword = token("set");
        let element = token("string");
        let first = token("\"a\"");
        let second = token("\"b\"");
        let closing = token("]");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: Some(SimpleType::Identifier(&element, None)),
            values: Some(vec![
                Literal::String("a".to_string(), &first),
                Literal::String("b".to_string(), &second),
            ]),
            last_token: &closing,
        });

        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn casts_and_representations_do_not_bypass_uniqueness() {
        let keyword = token("set");
        let element = token("u8");
        let plain = token("1");
        let cast_value = token("1");
        let cast = token("u8");
        let closing = token("]");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: Some(SimpleType::Identifier(&element, None)),
            values: Some(vec![
                integer(1, 1, &plain, None),
                integer(1, 8, &cast_value, Some(&cast)),
            ]),
            last_token: &closing,
        });

        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert!(errors[0].message.contains("Duplicate set value '1 as u8'"));
    }

    #[test]
    fn untyped_prefills_adopt_the_first_value_type_and_reject_wider_members() {
        let keyword = token("set");
        let first = token("1");
        let wider = token("16");
        let closing = token("]");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: None,
            values: Some(vec![
                integer(1, 1, &first, None),
                integer(16, 5, &wider, None),
            ]),
            last_token: &closing,
        });

        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert!(errors[0]
            .message
            .contains("Set value '16' is not compatible with element type 'u4'"));
    }

    #[test]
    fn untyped_prefills_accept_members_narrower_than_the_first() {
        let keyword = token("set");
        let first = token("16");
        let narrower = token("1");
        let closing = token("]");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: None,
            values: Some(vec![
                integer(16, 5, &first, None),
                integer(1, 1, &narrower, None),
            ]),
            last_token: &closing,
        });

        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn sets_without_a_prefill_have_nothing_to_validate() {
        let keyword = token("set");
        let element = token("string");
        let closing = token(">");

        let errors = validate(&SetType {
            keyword: &keyword,
            element_type: Some(SimpleType::Identifier(&element, None)),
            values: None,
            last_token: &closing,
        });

        assert!(errors.is_empty(), "{errors:#?}");
    }
}
