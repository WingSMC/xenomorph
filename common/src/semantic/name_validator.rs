use crate::{
    parser::{Declaration, Expr, SimpleType},
    semantic::{simple_to_owned_type, AnalyzerListener, OwnedType, ScopeInfo},
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
            OwnedType::Generic { .. } => candidate,
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
        match ty {
            SimpleType::Identifier(id, arguments)
            | SimpleType::OptionalIdentifier(id, arguments)
            | SimpleType::Array(id, arguments)
            | SimpleType::OptionalArray(id, arguments) => {
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
            SimpleType::Literal(_) | SimpleType::OptionalLiteral(_) => {}
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
