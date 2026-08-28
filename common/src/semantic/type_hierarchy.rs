use std::collections::{HashMap, HashSet};

use crate::parser::{Literal, SimpleType, Type};

use super::{XenoConstraint, XenoParent, XenoTrait, XenoType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParameterInfo {
    pub name: String,
    /// Name of either a type or a trait that constrains this parameter.
    pub constraint: Option<String>,
    /// Module in which an unqualified type constraint must be resolved.
    pub constraint_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDeclarationInfo {
    pub generic_params: Vec<GenericParameterInfo>,
    /// Immediate type parents. Intersections contribute multiple parents.
    pub parents: Vec<OwnedType>,
    /// Whether this declaration is a transparent `type Alias = Parent`
    /// declaration whose parent can be substituted at use sites.
    pub transparent_alias: bool,
}

impl TypeDeclarationInfo {
    pub fn from_ast(
        generics: Option<&[(&crate::TokenData<'_>, Option<&crate::TokenData<'_>>)]>,
        ty: &Type<'_>,
    ) -> Self {
        Self {
            generic_params: generics
                .unwrap_or(&[])
                .iter()
                .map(|(name, constraint)| GenericParameterInfo {
                    name: name.v.to_string(),
                    constraint: constraint.map(|constraint| constraint.v.to_string()),
                    constraint_scope: None,
                })
                .collect(),
            parents: type_parents(ty),
            transparent_alias: matches!(ty, Type::Simple(_)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedType {
    Named {
        name: String,
        arguments: Vec<OwnedType>,
    },
    Qualified {
        module_path: Option<String>,
        name: String,
        arguments: Vec<OwnedType>,
    },
    Array(Box<OwnedType>),
    /// A generic parameter as used while validating its declaration body.
    Generic {
        name: String,
        constraint: Option<String>,
        constraint_scope: Option<String>,
    },
}

impl OwnedType {
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named {
            name: name.into(),
            arguments: Vec::new(),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Named { name, arguments } if arguments.is_empty() => name.clone(),
            Self::Named { name, arguments } => format!(
                "{}<{}>",
                name,
                arguments
                    .iter()
                    .map(OwnedType::display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Qualified {
                module_path,
                name,
                arguments,
            } if arguments.is_empty() => module_path
                .as_deref()
                .map(|module_path| format!("{module_path}::{name}"))
                .unwrap_or_else(|| name.clone()),
            Self::Qualified {
                module_path,
                name,
                arguments,
            } => format!(
                "{}<{}>",
                module_path
                    .as_deref()
                    .map(|module_path| format!("{module_path}::{name}"))
                    .unwrap_or_else(|| name.clone()),
                arguments
                    .iter()
                    .map(OwnedType::display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Array(inner) => format!("{}[]", inner.display_name()),
            Self::Generic { name, .. } => name.clone(),
        }
    }
}

/// One node in the runtime hierarchy. Static builtin/plugin types and loaded
/// source declarations use the same representation after pass one.
#[derive(Debug, Clone)]
pub struct TypeDefinition {
    pub name: String,
    /// `None` for builtin/plugin types, otherwise the declaring module path.
    pub module_path: Option<String>,
    pub generic_params: Vec<GenericParameterInfo>,
    pub parents: Vec<OwnedType>,
    pub traits: Vec<&'static XenoTrait>,
    /// Source-level simple aliases are transparent to target generators.
    pub transparent_alias: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TypeKey {
    module_path: Option<String>,
    name: String,
}

#[derive(Debug, Clone, Default)]
pub struct TypeHierarchy {
    types: HashMap<TypeKey, TypeDefinition>,
    traits: HashMap<String, &'static XenoTrait>,
    module_imports: HashMap<String, Vec<String>>,
    current_module: Option<String>,
}

impl TypeHierarchy {
    pub fn set_current_module(&mut self, module_path: impl Into<String>) {
        self.current_module = Some(module_path.into());
    }

    pub fn register_module(&mut self, module_path: impl Into<String>, imports: Vec<String>) {
        self.module_imports.insert(module_path.into(), imports);
    }

    pub fn insert_trait(&mut self, xeno_trait: &'static XenoTrait) {
        for parent in xeno_trait.parents.unwrap_or(&[]) {
            self.insert_trait(parent);
        }
        self.traits.insert(xeno_trait.name.to_string(), xeno_trait);
    }

    pub fn insert_semantic_type(&mut self, semantic_type: &'static XenoType) {
        self.insert_semantic_type_inner(semantic_type, &mut HashSet::new());
    }

    fn insert_semantic_type_inner(
        &mut self,
        semantic_type: &'static XenoType,
        visited: &mut HashSet<*const XenoType>,
    ) {
        if !visited.insert(semantic_type as *const XenoType) {
            return;
        }

        let mut parents = Vec::new();
        let mut traits = Vec::new();
        let mut generic_params = Vec::new();
        for parent in semantic_type.parents.unwrap_or(&[]) {
            match parent {
                XenoParent::Type(parent_type) => {
                    self.insert_semantic_type_inner(parent_type, visited);
                    parents.push(OwnedType::named(parent_type.name));
                }
                XenoParent::Trait(parent_trait) => {
                    self.insert_trait(parent_trait);
                    traits.push(*parent_trait);
                }
            }
        }
        for parameter in semantic_type.generic_params.unwrap_or(&[]) {
            let constraint = parameter.constraint.map(|constraint| match constraint {
                XenoConstraint::Type(required_type) => {
                    self.insert_semantic_type_inner(required_type, visited);
                    required_type.name.to_string()
                }
                XenoConstraint::Trait(required_trait) => {
                    self.insert_trait(required_trait);
                    required_trait.name.to_string()
                }
            });
            generic_params.push(GenericParameterInfo {
                name: parameter.name.to_string(),
                constraint,
                constraint_scope: None,
            });
        }

        self.types.insert(
            TypeKey {
                module_path: None,
                name: semantic_type.name.to_string(),
            },
            TypeDefinition {
                name: semantic_type.name.to_string(),
                module_path: None,
                generic_params,
                parents,
                traits,
                transparent_alias: false,
            },
        );
        visited.remove(&(semantic_type as *const XenoType));
    }

    pub fn insert_declaration(
        &mut self,
        module_path: impl Into<String>,
        name: impl Into<String>,
        declaration: TypeDeclarationInfo,
    ) {
        let module_path = module_path.into();
        let name = name.into();
        let generic_params = declaration
            .generic_params
            .into_iter()
            .map(|mut parameter| {
                parameter.constraint_scope = Some(module_path.clone());
                parameter
            })
            .collect();
        self.types.insert(
            TypeKey {
                module_path: Some(module_path.clone()),
                name: name.clone(),
            },
            TypeDefinition {
                name,
                module_path: Some(module_path),
                generic_params,
                parents: declaration.parents,
                traits: Vec::new(),
                transparent_alias: declaration.transparent_alias,
            },
        );
    }

    pub fn get_type(&self, name: &str) -> Option<&TypeDefinition> {
        self.resolve_type(name, self.current_module.as_deref())
    }

    pub fn get_type_in(&self, module_path: Option<&str>, name: &str) -> Option<&TypeDefinition> {
        self.types.get(&TypeKey {
            module_path: module_path.map(str::to_string),
            name: name.to_string(),
        })
    }

    pub fn get_trait(&self, name: &str) -> Option<&'static XenoTrait> {
        self.traits.get(name).copied()
    }

    pub fn has_type(&self, name: &str) -> bool {
        self.get_type(name).is_some()
    }

    pub fn has_trait(&self, name: &str) -> bool {
        self.traits.contains_key(name)
    }

    pub fn generic_parameters(&self, name: &str) -> Option<Vec<GenericParameterInfo>> {
        self.types
            .get(&self.resolve_key(name, self.current_module.as_deref())?)
            .map(|definition| definition.generic_params.clone())
    }

    /// Expands source-level simple aliases and substitutes their generic
    /// parameters while retaining structs, enums, builtins, and plugin types
    /// as named target-language types.
    pub fn resolve_transparent_aliases(&self, candidate: &OwnedType) -> OwnedType {
        self.resolve_transparent_aliases_inner(
            candidate,
            self.current_module.as_deref(),
            &mut HashSet::new(),
        )
    }

    fn resolve_transparent_aliases_inner(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<TypeKey>,
    ) -> OwnedType {
        match candidate {
            OwnedType::Array(inner) => OwnedType::Array(Box::new(
                self.resolve_transparent_aliases_inner(inner, context, resolving),
            )),
            OwnedType::Generic { .. } => candidate.clone(),
            OwnedType::Named { name, arguments } => {
                let Some(key) = self.resolve_key(name, context) else {
                    return OwnedType::Named {
                        name: name.clone(),
                        arguments: arguments
                            .iter()
                            .map(|argument| {
                                self.resolve_transparent_aliases_inner(argument, context, resolving)
                            })
                            .collect(),
                    };
                };
                self.resolve_named_alias(key, arguments, context, resolving)
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => self.resolve_named_alias(
                TypeKey {
                    module_path: module_path.clone(),
                    name: name.clone(),
                },
                arguments,
                context,
                resolving,
            ),
        }
    }

    fn resolve_named_alias(
        &self,
        key: TypeKey,
        arguments: &[OwnedType],
        argument_context: Option<&str>,
        resolving: &mut HashSet<TypeKey>,
    ) -> OwnedType {
        let qualified_arguments = arguments
            .iter()
            .map(|argument| self.qualify_type(argument, argument_context))
            .collect::<Vec<_>>();
        let Some(definition) = self.types.get(&key) else {
            return OwnedType::Qualified {
                module_path: key.module_path,
                name: key.name,
                arguments: qualified_arguments,
            };
        };

        if definition.transparent_alias && resolving.insert(key.clone()) {
            let substitutions = definition
                .generic_params
                .iter()
                .zip(&qualified_arguments)
                .map(|(parameter, argument)| (parameter.name.as_str(), argument))
                .collect::<HashMap<_, _>>();
            let expanded = definition
                .parents
                .first()
                .map(|parent| substitute_owned_type(parent, &substitutions))
                .map(|parent| {
                    self.resolve_transparent_aliases_inner(
                        &parent,
                        definition.module_path.as_deref(),
                        resolving,
                    )
                });
            resolving.remove(&key);
            if let Some(expanded) = expanded {
                return expanded;
            }
        }

        OwnedType::Qualified {
            module_path: key.module_path,
            name: key.name,
            arguments: qualified_arguments
                .iter()
                .map(|argument| {
                    self.resolve_transparent_aliases_inner(argument, argument_context, resolving)
                })
                .collect(),
        }
    }

    pub fn type_implements_trait(&self, candidate: &OwnedType, required: &XenoTrait) -> bool {
        self.type_implements_trait_inner(
            candidate,
            self.current_module.as_deref(),
            required,
            &mut HashSet::new(),
        )
    }

    fn type_implements_trait_inner(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        required: &XenoTrait,
        visited: &mut HashSet<String>,
    ) -> bool {
        let (key, arguments) = match candidate {
            OwnedType::Named { name, arguments } => {
                let Some(key) = self.resolve_key(name, context) else {
                    return false;
                };
                (key, arguments.as_slice())
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => (
                TypeKey {
                    module_path: module_path.clone(),
                    name: name.clone(),
                },
                arguments.as_slice(),
            ),
            OwnedType::Array(_) => {
                let Some(key) = self.resolve_key("array", None) else {
                    return false;
                };
                (key, &[][..])
            }
            OwnedType::Generic {
                constraint,
                constraint_scope,
                ..
            } => {
                return constraint.as_deref().is_some_and(|constraint| {
                    self.constraint_implies_trait(constraint, constraint_scope.as_deref(), required)
                });
            }
        };

        let visit_key = format!("{:?}:{}", key.module_path, candidate.display_name());
        if !visited.insert(visit_key) {
            return false;
        }
        let Some(definition) = self.types.get(&key) else {
            return false;
        };
        if definition
            .traits
            .iter()
            .any(|candidate_trait| candidate_trait.is_or_inherits(required))
        {
            return true;
        }

        let qualified_arguments = arguments
            .iter()
            .map(|argument| self.qualify_type(argument, context))
            .collect::<Vec<_>>();
        let substitutions = definition
            .generic_params
            .iter()
            .zip(&qualified_arguments)
            .map(|(parameter, argument)| (parameter.name.as_str(), argument))
            .collect::<HashMap<_, _>>();
        definition
            .parents
            .iter()
            .map(|parent| substitute_owned_type(parent, &substitutions))
            .any(|parent| {
                self.type_implements_trait_inner(
                    &parent,
                    definition.module_path.as_deref(),
                    required,
                    visited,
                )
            })
    }

    pub fn is_type_compatible(&self, candidate: &OwnedType, target: &str) -> bool {
        self.is_type_compatible_in(
            candidate,
            self.current_module.as_deref(),
            target,
            self.current_module.as_deref(),
        )
    }

    pub fn descends_from_static_type(&self, candidate: &OwnedType, target: &XenoType) -> bool {
        let target_key = TypeKey {
            module_path: None,
            name: target.name.to_string(),
        };
        self.types.contains_key(&target_key)
            && self.is_type_compatible_inner(
                candidate,
                self.current_module.as_deref(),
                &target_key,
                &mut HashSet::new(),
            )
    }

    fn is_type_compatible_in(
        &self,
        candidate: &OwnedType,
        candidate_context: Option<&str>,
        target: &str,
        target_context: Option<&str>,
    ) -> bool {
        if target == "any" && self.resolve_key(target, target_context).is_some() {
            return true;
        }
        let Some(target_key) = self.resolve_key(target, target_context) else {
            return false;
        };
        self.is_type_compatible_inner(
            candidate,
            candidate_context,
            &target_key,
            &mut HashSet::new(),
        )
    }

    fn is_type_compatible_inner(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        target: &TypeKey,
        visited: &mut HashSet<String>,
    ) -> bool {
        if target.module_path.is_none() && target.name == "any" {
            return true;
        }

        let (key, arguments) = match candidate {
            OwnedType::Named { name, arguments } => {
                let Some(key) = self.resolve_key(name, context) else {
                    return false;
                };
                (key, arguments.as_slice())
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => (
                TypeKey {
                    module_path: module_path.clone(),
                    name: name.clone(),
                },
                arguments.as_slice(),
            ),
            OwnedType::Array(_) => {
                let Some(key) = self.resolve_key("array", None) else {
                    return false;
                };
                (key, &[][..])
            }
            OwnedType::Generic {
                constraint,
                constraint_scope,
                ..
            } => {
                return constraint.as_deref().is_some_and(|constraint| {
                    self.constraint_implies(constraint, constraint_scope.as_deref(), target)
                });
            }
        };

        if key == *target {
            return true;
        }
        let visit_key = format!("{:?}:{}", key.module_path, candidate.display_name());
        if key.module_path.is_none() && key.name == "any" || !visited.insert(visit_key) {
            return false;
        }
        let Some(definition) = self.types.get(&key) else {
            return false;
        };
        let qualified_arguments = arguments
            .iter()
            .map(|argument| self.qualify_type(argument, context))
            .collect::<Vec<_>>();
        let substitutions = definition
            .generic_params
            .iter()
            .zip(&qualified_arguments)
            .map(|(parameter, argument)| (parameter.name.as_str(), argument))
            .collect::<HashMap<_, _>>();
        definition
            .parents
            .iter()
            .map(|parent| substitute_owned_type(parent, &substitutions))
            .any(|parent| {
                self.is_type_compatible_inner(
                    &parent,
                    definition.module_path.as_deref(),
                    target,
                    visited,
                )
            })
    }

    /// Checks a concrete or generic candidate against a named type/trait constraint.
    pub fn satisfies_constraint(
        &self,
        candidate: &OwnedType,
        constraint: &str,
        constraint_scope: Option<&str>,
    ) -> bool {
        if let Some(required_trait) = self.get_trait(constraint) {
            self.type_implements_trait(candidate, required_trait)
        } else {
            self.is_type_compatible_in(
                candidate,
                self.current_module.as_deref(),
                constraint,
                constraint_scope,
            )
        }
    }

    fn constraint_implies(
        &self,
        candidate_constraint: &str,
        candidate_scope: Option<&str>,
        required: &TypeKey,
    ) -> bool {
        if self.get_trait(candidate_constraint).is_some() {
            return false;
        }
        let Some(candidate_key) = self.resolve_key(candidate_constraint, candidate_scope) else {
            return false;
        };
        self.is_type_compatible_inner(
            &OwnedType::Qualified {
                module_path: candidate_key.module_path,
                name: candidate_key.name,
                arguments: Vec::new(),
            },
            candidate_scope,
            required,
            &mut HashSet::new(),
        )
    }

    fn constraint_implies_trait(
        &self,
        candidate_constraint: &str,
        candidate_scope: Option<&str>,
        required: &XenoTrait,
    ) -> bool {
        if let Some(candidate_trait) = self.get_trait(candidate_constraint) {
            candidate_trait.is_or_inherits(required)
        } else {
            self.type_implements_trait_inner(
                &OwnedType::named(candidate_constraint),
                candidate_scope,
                required,
                &mut HashSet::new(),
            )
        }
    }

    fn resolve_type(&self, name: &str, context: Option<&str>) -> Option<&TypeDefinition> {
        let key = self.resolve_key(name, context)?;
        self.types.get(&key)
    }

    fn resolve_key(&self, name: &str, context: Option<&str>) -> Option<TypeKey> {
        if let Some(module_path) = context {
            let local = TypeKey {
                module_path: Some(module_path.to_string()),
                name: name.to_string(),
            };
            if self.types.contains_key(&local) {
                return Some(local);
            }
            for import in self
                .module_imports
                .get(module_path)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let imported = TypeKey {
                    module_path: Some(import.clone()),
                    name: name.to_string(),
                };
                if self.types.contains_key(&imported) {
                    return Some(imported);
                }
            }
        }

        let semantic = TypeKey {
            module_path: None,
            name: name.to_string(),
        };
        self.types.contains_key(&semantic).then_some(semantic)
    }

    fn qualify_type(&self, ty: &OwnedType, context: Option<&str>) -> OwnedType {
        match ty {
            OwnedType::Named { name, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.qualify_type(argument, context))
                    .collect();
                match self.resolve_key(name, context) {
                    Some(key) => OwnedType::Qualified {
                        module_path: key.module_path,
                        name: key.name,
                        arguments,
                    },
                    None => OwnedType::Named {
                        name: name.clone(),
                        arguments,
                    },
                }
            }
            OwnedType::Qualified { .. } | OwnedType::Generic { .. } => ty.clone(),
            OwnedType::Array(inner) => {
                OwnedType::Array(Box::new(self.qualify_type(inner, context)))
            }
        }
    }
}

pub fn type_parents(ty: &Type<'_>) -> Vec<OwnedType> {
    match ty {
        Type::Simple(simple) => vec![simple_to_owned_type(simple)],
        Type::Intersection(types) => types.iter().map(simple_to_owned_type).collect(),
        Type::Sum(_) => vec![OwnedType::named("any")],
        Type::Tuple(_) | Type::Set(_) => vec![OwnedType::named("array")],
        Type::Struct(_) => vec![OwnedType::named("dict")],
        Type::Enum(_) => vec![OwnedType::named("any")],
    }
}

pub fn simple_to_owned_type(simple: &SimpleType<'_>) -> OwnedType {
    match simple {
        SimpleType::Identifier(identifier, arguments)
        | SimpleType::OptionalIdentifier(identifier, arguments) => OwnedType::Named {
            name: identifier.v.to_string(),
            arguments: arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(simple_to_owned_type)
                .collect(),
        },
        SimpleType::Array(identifier, arguments)
        | SimpleType::OptionalArray(identifier, arguments) => {
            OwnedType::Array(Box::new(OwnedType::Named {
                name: identifier.v.to_string(),
                arguments: arguments
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .map(simple_to_owned_type)
                    .collect(),
            }))
        }
        SimpleType::Literal(literal @ (Literal::Int(_) | Literal::Float(_)))
        | SimpleType::OptionalLiteral(literal @ (Literal::Int(_) | Literal::Float(_))) => {
            OwnedType::named(literal.semantic_type_name())
        }
        SimpleType::Literal(Literal::String(_, _))
        | SimpleType::OptionalLiteral(Literal::String(_, _)) => OwnedType::named("string"),
        SimpleType::Literal(Literal::Boolean(_, _))
        | SimpleType::OptionalLiteral(Literal::Boolean(_, _)) => OwnedType::named("bool"),
    }
}

fn substitute_owned_type(ty: &OwnedType, substitutions: &HashMap<&str, &OwnedType>) -> OwnedType {
    match ty {
        OwnedType::Named { name, arguments } if arguments.is_empty() => substitutions
            .get(name.as_str())
            .map(|replacement| (*replacement).clone())
            .unwrap_or_else(|| ty.clone()),
        OwnedType::Named { name, arguments } => OwnedType::Named {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute_owned_type(argument, substitutions))
                .collect(),
        },
        OwnedType::Qualified {
            module_path,
            name,
            arguments,
        } => OwnedType::Qualified {
            module_path: module_path.clone(),
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute_owned_type(argument, substitutions))
                .collect(),
        },
        OwnedType::Array(inner) => {
            OwnedType::Array(Box::new(substitute_owned_type(inner, substitutions)))
        }
        OwnedType::Generic { .. } => ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::ANY;

    #[test]
    fn unconstrained_generics_are_compatible_with_any() {
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.insert_semantic_type(&ANY);

        let candidate = OwnedType::Generic {
            name: "T".to_string(),
            constraint: None,
            constraint_scope: Some("test".to_string()),
        };

        assert!(hierarchy.descends_from_static_type(&candidate, &ANY));
    }

    #[test]
    fn transparent_generic_aliases_expand_recursively() {
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.set_current_module("test");
        hierarchy.register_module("test", Vec::new());
        hierarchy.insert_declaration(
            "test",
            "Identity",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("T")],
                transparent_alias: true,
            },
        );
        hierarchy.insert_declaration(
            "test",
            "Items",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::Array(Box::new(OwnedType::Named {
                    name: "Identity".to_string(),
                    arguments: vec![OwnedType::named("T")],
                }))],
                transparent_alias: true,
            },
        );

        let resolved = hierarchy.resolve_transparent_aliases(&OwnedType::Named {
            name: "Items".to_string(),
            arguments: vec![OwnedType::named("u8")],
        });

        assert_eq!(
            resolved,
            OwnedType::Array(Box::new(OwnedType::Named {
                name: "u8".to_string(),
                arguments: Vec::new(),
            }))
        );
    }

    #[test]
    fn concrete_generic_declarations_are_not_expanded() {
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.set_current_module("test");
        hierarchy.register_module("test", Vec::new());
        hierarchy.insert_declaration(
            "test",
            "Box",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("dict")],
                transparent_alias: false,
            },
        );

        let resolved = hierarchy.resolve_transparent_aliases(&OwnedType::Named {
            name: "Box".to_string(),
            arguments: vec![OwnedType::named("string")],
        });

        assert_eq!(
            resolved,
            OwnedType::Qualified {
                module_path: Some("test".to_string()),
                name: "Box".to_string(),
                arguments: vec![OwnedType::Named {
                    name: "string".to_string(),
                    arguments: Vec::new(),
                }],
            }
        );
    }

    fn generic(name: &str) -> GenericParameterInfo {
        GenericParameterInfo {
            name: name.to_string(),
            constraint: None,
            constraint_scope: None,
        }
    }
}
