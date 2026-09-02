use std::collections::HashMap;

use crate::{
    parser::{Declaration, Type, XenoType},
    semantic::{simple_to_owned_type, AnalyzerListener, OwnedType, ScopeInfo},
    XenoDiagSeverity, XenoDiagnostic,
};

/// Rejects source intersections whose member domains are provably disjoint.
/// The hierarchy also normalizes these expressions to `NEVER`, so downstream
/// semantic queries retain correct bottom-type behavior after the diagnostic.
pub struct IntersectionValidator {
    scope: ScopeInfo,
    generic_params: HashMap<String, Option<String>>,
}

impl IntersectionValidator {
    pub fn new(scope: &ScopeInfo) -> Self {
        Self {
            scope: scope.clone(),
            generic_params: HashMap::new(),
        }
    }

    fn resolve_generic(&self, candidate: OwnedType) -> OwnedType {
        match candidate {
            OwnedType::Named { name, arguments } if arguments.is_empty() => {
                match self.generic_params.get(&name) {
                    Some(constraint) => OwnedType::Generic {
                        name,
                        constraint: constraint.clone(),
                        constraint_scope: Some(self.scope.module_path.clone()),
                    },
                    None => OwnedType::named(name),
                }
            }
            OwnedType::Named { name, arguments } => OwnedType::Named {
                name,
                arguments: arguments
                    .into_iter()
                    .map(|argument| self.resolve_generic(argument))
                    .collect(),
            },
            OwnedType::Array(inner) => OwnedType::Array(Box::new(self.resolve_generic(*inner))),
            OwnedType::Optional(inner) => {
                OwnedType::Optional(Box::new(self.resolve_generic(*inner)))
            }
            _ => candidate,
        }
    }
}

impl<'src> AnalyzerListener<'src> for IntersectionValidator {
    fn on_before_decl(
        &mut self,
        declaration: &Declaration<'src>,
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.generic_params.clear();
        if let Declaration::Type { generics, .. } = declaration {
            self.generic_params
                .extend(generics.as_deref().unwrap_or_default().iter().map(
                    |(name, constraint)| {
                        (
                            name.v.to_string(),
                            constraint.map(|constraint| constraint.v.to_string()),
                        )
                    },
                ));
        }
    }

    fn on_before_type(&mut self, (ty, _): &XenoType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        let Type::Intersection(items) = ty else {
            return;
        };
        let resolved = items
            .iter()
            .map(simple_to_owned_type)
            .map(|item| self.resolve_generic(item))
            .collect::<Vec<_>>();
        if !self.scope.type_hierarchy.intersection_is_never(&resolved) {
            return;
        }

        let (location, message) = self
            .scope
            .type_hierarchy
            .first_disjoint_intersection_pair(&resolved)
            .map_or_else(
                || {
                    (
                        items
                            .last()
                            .map(|item| item.get_last_token().clone())
                            .unwrap_or_default(),
                        format!(
                            "Types '{}' cannot be intersected because their intersection is NEVER.",
                            resolved
                                .iter()
                                .map(OwnedType::display_name)
                                .collect::<Vec<_>>()
                                .join("', '")
                        ),
                    )
                },
                |(left, right)| {
                    (
                        items[right].get_last_token().clone(),
                        format!(
                            "Types '{}' and '{}' cannot be intersected because their intersection is NEVER.",
                            resolved[left].display_name(),
                            resolved[right].display_name(),
                        ),
                    )
                },
            );
        errors.push(XenoDiagnostic {
            location,
            message,
            severity: XenoDiagSeverity::Err,
        });
    }

    fn on_after_decl(
        &mut self,
        _declaration: &Declaration<'src>,
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.generic_params.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use super::*;
    use crate::parser::SimpleType;
    use crate::semantic::{TypeHierarchy, BUILTIN_TYPES};
    use crate::TokenData;

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

    #[test]
    fn rejects_provably_disjoint_builtin_types() {
        let left = token("u8");
        let right = token("string");
        let ty = (
            Type::Intersection(vec![
                SimpleType::Identifier(&left, None),
                SimpleType::Identifier(&right, None),
            ]),
            Vec::new(),
        );
        let mut diagnostics = Vec::new();

        IntersectionValidator::new(&scope()).on_before_type(&ty, &mut diagnostics);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "Types 'u8' and 'string' cannot be intersected because their intersection is NEVER."
        );
        assert_eq!(diagnostics[0].location.v, "string");
    }

    #[test]
    fn allows_builtin_types_with_an_overlapping_domain() {
        let left = token("u8");
        let right = token("number");
        let ty = (
            Type::Intersection(vec![
                SimpleType::Identifier(&left, None),
                SimpleType::Identifier(&right, None),
            ]),
            Vec::new(),
        );
        let mut diagnostics = Vec::new();

        IntersectionValidator::new(&scope()).on_before_type(&ty, &mut diagnostics);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn optional_disjoint_bases_overlap_at_the_absent_value() {
        let left = token("u8");
        let right = token("string");
        let ty = (
            Type::Intersection(vec![
                SimpleType::Optional(Box::new(SimpleType::Identifier(&left, None))),
                SimpleType::Optional(Box::new(SimpleType::Identifier(&right, None))),
            ]),
            Vec::new(),
        );
        let mut diagnostics = Vec::new();

        IntersectionValidator::new(&scope()).on_before_type(&ty, &mut diagnostics);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn optional_and_required_disjoint_types_have_no_common_value() {
        let left = token("u8");
        let right = token("string");
        let ty = (
            Type::Intersection(vec![
                SimpleType::Optional(Box::new(SimpleType::Identifier(&left, None))),
                SimpleType::Identifier(&right, None),
            ]),
            Vec::new(),
        );
        let mut diagnostics = Vec::new();

        IntersectionValidator::new(&scope()).on_before_type(&ty, &mut diagnostics);

        assert_eq!(diagnostics.len(), 1);
    }
}
