use std::collections::{HashMap, HashSet};
use std::fmt;

use bigdecimal::BigDecimal;
use num_bigint::BigInt;

use crate::parser::{
    FloatRepresentation, FloatSize, IntegerRepresentation, IntegerSize, Literal, SimpleType, Type,
};
use crate::utils::extract_documentation;

use super::{XenoConstraint, XenoParent, XenoTrait, XenoTraitKind, XenoType};

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
    /// Complete lifetime-free declaration body used by structural type utilities.
    pub body: OwnedType,
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
            body: type_to_owned_type(ty),
            transparent_alias: matches!(ty, Type::Simple(_)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedField {
    pub name: String,
    pub ty: OwnedType,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedLiteral {
    Integer {
        value: BigInt,
        representation: IntegerRepresentation,
        cast: Option<String>,
    },
    Float {
        value: BigDecimal,
        representation: FloatRepresentation,
        cast: Option<String>,
    },
    String(String),
    Boolean(bool),
}

impl OwnedLiteral {
    pub fn semantic_type_name(&self) -> &str {
        match self {
            Self::Integer { cast, .. } => cast.as_deref().unwrap_or("integer"),
            Self::Float { cast, .. } => cast.as_deref().unwrap_or("number"),
            Self::String(_) => "string",
            Self::Boolean(_) => "bool",
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Integer { value, cast, .. } => cast
                .as_deref()
                .map(|cast| format!("{value} as {cast}"))
                .unwrap_or_else(|| value.to_string()),
            Self::Float { value, cast, .. } => cast
                .as_deref()
                .map(|cast| format!("{value} as {cast}"))
                .unwrap_or_else(|| value.to_string()),
            Self::String(value) => format!("\"{value}\""),
            Self::Boolean(value) => value.to_string(),
        }
    }

    /// The narrowest builtin type that can hold this constant, e.g. `1` is
    /// `u4` and `1.0` is `f32`. Explicit casts win.
    pub fn narrowest_type_name(&self) -> String {
        match self {
            Self::Integer {
                representation,
                cast,
                ..
            } => cast
                .clone()
                .unwrap_or_else(|| narrowest_integer_type(*representation).to_string()),
            Self::Float {
                representation,
                cast,
                ..
            } => cast.clone().unwrap_or_else(|| {
                match representation.size {
                    FloatSize::F32 => "f32",
                    FloatSize::F64 => "f64",
                    FloatSize::Decimal => "decimal",
                }
                .to_string()
            }),
            Self::String(_) => "string".to_string(),
            Self::Boolean(_) => "bool".to_string(),
        }
    }

    /// Compares runtime constant values while ignoring source casts and
    /// storage representations. Numerically equal integers and decimals are
    /// the same set value.
    pub fn same_constant_value(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Integer { value: left, .. }, Self::Integer { value: right, .. }) => {
                left == right
            }
            (Self::Float { value: left, .. }, Self::Float { value: right, .. }) => left == right,
            (Self::Integer { value: left, .. }, Self::Float { value: right, .. }) => left
                .to_string()
                .parse::<BigDecimal>()
                .is_ok_and(|left| left == *right),
            (Self::Float { value: left, .. }, Self::Integer { value: right, .. }) => right
                .to_string()
                .parse::<BigDecimal>()
                .is_ok_and(|right| *left == right),
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Boolean(left), Self::Boolean(right)) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedSet {
    pub element_type: Option<Box<OwnedType>>,
    pub values: Option<Vec<OwnedLiteral>>,
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
    Optional(Box<OwnedType>),
    Literal(OwnedLiteral),
    Tuple(Vec<OwnedType>),
    Set(OwnedSet),
    Struct(Vec<OwnedField>),
    Enum(Vec<OwnedField>),
    Sum(Vec<OwnedType>),
    Intersection(Vec<OwnedType>),
    /// A generic parameter as used while validating its declaration body.
    Generic {
        name: String,
        constraint: Option<String>,
        constraint_scope: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeEvaluationError {
    UnknownStructKey {
        utility: &'static str,
        key: String,
        target: String,
    },
}

impl fmt::Display for TypeEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStructKey {
                utility,
                key,
                target,
            } => write!(
                formatter,
                "Type utility '{utility}' cannot select unknown field '{key}' from '{target}'."
            ),
        }
    }
}

impl std::error::Error for TypeEvaluationError {}

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
            Self::Optional(inner) => format!("?{}", inner.display_name()),
            Self::Literal(literal) => literal.display_name(),
            Self::Tuple(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(OwnedType::display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Set(set) => {
                let element_type = set
                    .element_type
                    .as_deref()
                    .map(|element| format!("<{}>", element.display_name()))
                    .unwrap_or_default();
                let values = set.values.as_ref().map_or_else(String::new, |values| {
                    format!(
                        "[{}]",
                        values
                            .iter()
                            .map(OwnedLiteral::display_name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                });
                format!("set{element_type}{values}")
            }
            Self::Struct(fields) => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, field.ty.display_name()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Enum(variants) => format!(
                "enum {{ {} }}",
                variants
                    .iter()
                    .map(|variant| format!("{}: {}", variant.name, variant.ty.display_name()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Sum(items) => items
                .iter()
                .map(OwnedType::display_name)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Intersection(items) => items
                .iter()
                .map(OwnedType::display_name)
                .collect::<Vec<_>>()
                .join(" & "),
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
    /// Source declaration body. Static semantic types do not have one.
    pub body: Option<OwnedType>,
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
        if let XenoTraitKind::Sum(member) = xeno_trait.kind {
            self.insert_trait(member);
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
                body: None,
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
        let body = declaration.body;
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
                body: Some(body),
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

    /// Evaluates transparent aliases and built-in type utilities into a shared,
    /// target-independent type expression. Concrete source declarations remain
    /// named unless a utility needs to inspect their structural body.
    pub fn evaluate_type(&self, candidate: &OwnedType) -> Result<OwnedType, TypeEvaluationError> {
        self.evaluate_type_inner(
            candidate,
            self.current_module.as_deref(),
            &mut HashSet::new(),
        )
    }

    fn evaluate_type_inner(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<String>,
    ) -> Result<OwnedType, TypeEvaluationError> {
        match candidate {
            OwnedType::Named { name, arguments } => {
                self.evaluate_named_type(name, arguments, context, None, resolving)
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => self.evaluate_named_type(
                name,
                arguments,
                context,
                Some(module_path.clone()),
                resolving,
            ),
            OwnedType::Array(inner) => Ok(OwnedType::Array(Box::new(
                self.evaluate_type_inner(inner, context, resolving)?,
            ))),
            OwnedType::Optional(inner) => Ok(optional_type(
                self.evaluate_type_inner(inner, context, resolving)?,
            )),
            OwnedType::Tuple(items) => Ok(OwnedType::Tuple(
                items
                    .iter()
                    .map(|item| self.evaluate_type_inner(item, context, resolving))
                    .collect::<Result<_, _>>()?,
            )),
            OwnedType::Set(set) => Ok(OwnedType::Set(OwnedSet {
                element_type: set
                    .element_type
                    .as_deref()
                    .map(|element| self.evaluate_type_inner(element, context, resolving))
                    .transpose()?
                    .map(Box::new),
                values: set.values.clone(),
            })),
            OwnedType::Struct(fields) => Ok(OwnedType::Struct(
                fields
                    .iter()
                    .map(|field| {
                        Ok(OwnedField {
                            name: field.name.clone(),
                            ty: self.evaluate_type_inner(&field.ty, context, resolving)?,
                            documentation: field.documentation.clone(),
                        })
                    })
                    .collect::<Result<_, TypeEvaluationError>>()?,
            )),
            OwnedType::Enum(variants) => Ok(OwnedType::Enum(
                variants
                    .iter()
                    .map(|variant| {
                        Ok(OwnedField {
                            name: variant.name.clone(),
                            ty: self.evaluate_type_inner(&variant.ty, context, resolving)?,
                            documentation: variant.documentation.clone(),
                        })
                    })
                    .collect::<Result<_, TypeEvaluationError>>()?,
            )),
            OwnedType::Sum(items) => Ok(OwnedType::Sum(
                items
                    .iter()
                    .map(|item| self.evaluate_type_inner(item, context, resolving))
                    .collect::<Result<_, _>>()?,
            )),
            OwnedType::Intersection(items) => Ok(OwnedType::Intersection(
                items
                    .iter()
                    .map(|item| self.evaluate_type_inner(item, context, resolving))
                    .collect::<Result<_, _>>()?,
            )),
            OwnedType::Literal(_) | OwnedType::Generic { .. } => Ok(candidate.clone()),
        }
    }

    fn evaluate_named_type(
        &self,
        name: &str,
        arguments: &[OwnedType],
        argument_context: Option<&str>,
        explicit_module: Option<Option<String>>,
        resolving: &mut HashSet<String>,
    ) -> Result<OwnedType, TypeEvaluationError> {
        let key = explicit_module.map_or_else(
            || self.resolve_key(name, argument_context),
            |module_path| {
                Some(TypeKey {
                    module_path,
                    name: name.to_string(),
                })
            },
        );
        let evaluated_arguments = arguments
            .iter()
            .map(|argument| self.evaluate_type_inner(argument, argument_context, resolving))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(key) = key else {
            return Ok(OwnedType::Named {
                name: name.to_string(),
                arguments: evaluated_arguments,
            });
        };

        if key.module_path.is_none() {
            if let Some(evaluated) = self.evaluate_utility(
                key.name.as_str(),
                &evaluated_arguments,
                argument_context,
                resolving,
            )? {
                return Ok(evaluated);
            }
        }

        let Some(definition) = self.types.get(&key) else {
            return Ok(OwnedType::Qualified {
                module_path: key.module_path,
                name: key.name,
                arguments: evaluated_arguments,
            });
        };
        if definition.transparent_alias {
            let visit_key = format!(
                "{:?}:{}<{}>",
                key.module_path,
                key.name,
                evaluated_arguments
                    .iter()
                    .map(OwnedType::display_name)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            if resolving.insert(visit_key.clone()) {
                let substitutions = definition
                    .generic_params
                    .iter()
                    .zip(&evaluated_arguments)
                    .map(|(parameter, argument)| (parameter.name.as_str(), argument))
                    .collect::<HashMap<_, _>>();
                let expanded = definition.body.as_ref().map(|body| {
                    let body = substitute_owned_type(body, &substitutions);
                    self.evaluate_type_inner(&body, definition.module_path.as_deref(), resolving)
                });
                resolving.remove(&visit_key);
                if let Some(expanded) = expanded {
                    return expanded;
                }
            }
        }

        if key.module_path.is_none() {
            Ok(OwnedType::Named {
                name: key.name,
                arguments: evaluated_arguments,
            })
        } else {
            Ok(OwnedType::Qualified {
                module_path: key.module_path,
                name: key.name,
                arguments: evaluated_arguments,
            })
        }
    }

    fn evaluate_utility(
        &self,
        name: &str,
        arguments: &[OwnedType],
        context: Option<&str>,
        resolving: &mut HashSet<String>,
    ) -> Result<Option<OwnedType>, TypeEvaluationError> {
        let result = match (name, arguments) {
            ("Required", [inner]) => required_type(inner.clone()),
            ("Keyof", [target]) => {
                let Some(fields) = self.resolve_struct_fields(target, context, resolving)? else {
                    return Ok(None);
                };
                OwnedType::Sum(
                    fields
                        .into_iter()
                        .map(|field| OwnedType::Literal(OwnedLiteral::String(field.name)))
                        .collect(),
                )
            }
            ("Pick", [target, keys]) => {
                let Some(fields) = self.resolve_struct_fields(target, context, resolving)? else {
                    return Ok(None);
                };
                let Some(keys) = self.resolve_string_literal_sum(keys, context, resolving)? else {
                    return Ok(None);
                };
                validate_struct_keys("Pick", target, &fields, &keys)?;
                OwnedType::Struct(
                    fields
                        .into_iter()
                        .filter(|field| keys.iter().any(|key| key == &field.name))
                        .collect(),
                )
            }
            ("Omit", [target, keys]) => {
                let Some(fields) = self.resolve_struct_fields(target, context, resolving)? else {
                    return Ok(None);
                };
                let Some(keys) = self.resolve_string_literal_sum(keys, context, resolving)? else {
                    return Ok(None);
                };
                validate_struct_keys("Omit", target, &fields, &keys)?;
                OwnedType::Struct(
                    fields
                        .into_iter()
                        .filter(|field| !keys.iter().any(|key| key == &field.name))
                        .collect(),
                )
            }
            ("Partial", [target]) => {
                let Some(mut fields) = self.resolve_struct_fields(target, context, resolving)?
                else {
                    return Ok(None);
                };
                for field in &mut fields {
                    field.ty = optional_type(field.ty.clone());
                }
                OwnedType::Struct(fields)
            }
            ("Complete", [target]) => {
                let Some(mut fields) = self.resolve_struct_fields(target, context, resolving)?
                else {
                    return Ok(None);
                };
                for field in &mut fields {
                    field.ty = required_type(field.ty.clone());
                }
                OwnedType::Struct(fields)
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn resolve_struct_fields(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<String>,
    ) -> Result<Option<Vec<OwnedField>>, TypeEvaluationError> {
        let evaluated = self.evaluate_type_inner(candidate, context, resolving)?;
        if let OwnedType::Struct(fields) = evaluated {
            return Ok(Some(fields));
        }
        self.resolve_declaration_body(&evaluated, context, resolving, |body| match body {
            OwnedType::Struct(fields) => Some(fields),
            _ => None,
        })
    }

    fn resolve_literal_sum(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<String>,
    ) -> Result<Option<Vec<OwnedType>>, TypeEvaluationError> {
        let evaluated = self.evaluate_type_inner(candidate, context, resolving)?;
        if matches!(evaluated, OwnedType::Literal(_)) {
            return Ok(Some(vec![evaluated]));
        }
        if let OwnedType::Sum(items) = evaluated {
            return Ok(items
                .iter()
                .all(|item| matches!(item, OwnedType::Literal(_)))
                .then_some(items));
        }
        self.resolve_declaration_body(&evaluated, context, resolving, |body| match body {
            literal @ OwnedType::Literal(_) => Some(vec![literal]),
            OwnedType::Sum(items)
                if items
                    .iter()
                    .all(|item| matches!(item, OwnedType::Literal(_))) =>
            {
                Some(items)
            }
            _ => None,
        })
    }

    fn resolve_string_literal_sum(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<String>,
    ) -> Result<Option<Vec<String>>, TypeEvaluationError> {
        Ok(self
            .resolve_literal_sum(candidate, context, resolving)?
            .and_then(|items| {
                items
                    .into_iter()
                    .map(|item| match item {
                        OwnedType::Literal(OwnedLiteral::String(value)) => Some(value),
                        _ => None,
                    })
                    .collect()
            }))
    }

    fn resolve_declaration_body<T>(
        &self,
        candidate: &OwnedType,
        context: Option<&str>,
        resolving: &mut HashSet<String>,
        extract: impl FnOnce(OwnedType) -> Option<T>,
    ) -> Result<Option<T>, TypeEvaluationError> {
        let (key, arguments) = match candidate {
            OwnedType::Named { name, arguments } => {
                let Some(key) = self.resolve_key(name, context) else {
                    return Ok(None);
                };
                (key, arguments)
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
                arguments,
            ),
            _ => return Ok(None),
        };
        let Some(definition) = self.types.get(&key) else {
            return Ok(None);
        };
        let Some(body) = definition.body.as_ref() else {
            return Ok(None);
        };
        let substitutions = definition
            .generic_params
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| (parameter.name.as_str(), argument))
            .collect::<HashMap<_, _>>();
        let body = substitute_owned_type(body, &substitutions);
        let body = self.evaluate_type_inner(&body, definition.module_path.as_deref(), resolving)?;
        Ok(extract(body))
    }

    /// Expands transparent aliases and evaluates type utilities so a target
    /// generator sees a concrete type. Falls back to the resolved form when a
    /// utility cannot be evaluated.
    pub fn resolve_for_target(&self, candidate: &OwnedType) -> OwnedType {
        let resolved = self.resolve_transparent_aliases(candidate);
        self.evaluate_type(&resolved).unwrap_or(resolved)
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
            OwnedType::Optional(inner) => OwnedType::Optional(Box::new(
                self.resolve_transparent_aliases_inner(inner, context, resolving),
            )),
            OwnedType::Tuple(items) => OwnedType::Tuple(
                items
                    .iter()
                    .map(|item| self.resolve_transparent_aliases_inner(item, context, resolving))
                    .collect(),
            ),
            OwnedType::Set(set) => OwnedType::Set(OwnedSet {
                element_type: set.element_type.as_deref().map(|element| {
                    Box::new(self.resolve_transparent_aliases_inner(element, context, resolving))
                }),
                values: set.values.clone(),
            }),
            OwnedType::Struct(fields) => OwnedType::Struct(
                fields
                    .iter()
                    .map(|field| OwnedField {
                        name: field.name.clone(),
                        ty: self.resolve_transparent_aliases_inner(&field.ty, context, resolving),
                        documentation: field.documentation.clone(),
                    })
                    .collect(),
            ),
            OwnedType::Enum(variants) => OwnedType::Enum(
                variants
                    .iter()
                    .map(|variant| OwnedField {
                        name: variant.name.clone(),
                        ty: self.resolve_transparent_aliases_inner(&variant.ty, context, resolving),
                        documentation: variant.documentation.clone(),
                    })
                    .collect(),
            ),
            OwnedType::Sum(items) => OwnedType::Sum(
                items
                    .iter()
                    .map(|item| self.resolve_transparent_aliases_inner(item, context, resolving))
                    .collect(),
            ),
            OwnedType::Intersection(items) => OwnedType::Intersection(
                items
                    .iter()
                    .map(|item| self.resolve_transparent_aliases_inner(item, context, resolving))
                    .collect(),
            ),
            OwnedType::Literal(_) => candidate.clone(),
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
                .body
                .as_ref()
                .map(|body| substitute_owned_type(body, &substitutions))
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
        if matches!(required.kind, XenoTraitKind::Struct | XenoTraitKind::Sum(_)) {
            if let Ok(evaluated) = self.evaluate_type(candidate) {
                if evaluated != *candidate {
                    return self.type_implements_trait_inner(
                        &evaluated,
                        self.current_module.as_deref(),
                        required,
                        &mut HashSet::new(),
                    );
                }
            }
        }
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
        match candidate {
            OwnedType::Optional(_) => return false,
            OwnedType::Literal(literal) => {
                return match required.kind {
                    XenoTraitKind::Literal => true,
                    XenoTraitKind::LiteralType | XenoTraitKind::Semantic => self
                        .type_implements_trait_inner(
                            &OwnedType::named(literal.semantic_type_name()),
                            context,
                            required,
                            visited,
                        ),
                    XenoTraitKind::Sum(member) => {
                        self.type_implements_trait_inner(candidate, context, member, visited)
                    }
                    _ => false,
                };
            }
            OwnedType::Tuple(_) | OwnedType::Set(_) => {
                return self.type_implements_trait_inner(
                    &OwnedType::named("array"),
                    context,
                    required,
                    visited,
                );
            }
            OwnedType::Struct(_) => {
                if matches!(required.kind, XenoTraitKind::Struct) {
                    return true;
                }
                return self.type_implements_trait_inner(
                    &OwnedType::named("dict"),
                    context,
                    required,
                    visited,
                );
            }
            OwnedType::Sum(items) => {
                let XenoTraitKind::Sum(member) = required.kind else {
                    return false;
                };
                return !items.is_empty()
                    && items.iter().all(|item| {
                        let Ok(item) = self.evaluate_type_inner(item, context, &mut HashSet::new())
                        else {
                            return false;
                        };
                        matches!(item, OwnedType::Literal(_))
                            && self.type_implements_trait_inner(
                                &item,
                                context,
                                member,
                                &mut visited.clone(),
                            )
                    });
            }
            OwnedType::Enum(_) | OwnedType::Intersection(_) => return false,
            OwnedType::Named { .. }
            | OwnedType::Qualified { .. }
            | OwnedType::Array(_)
            | OwnedType::Generic { .. } => {}
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
                    self.constraint_implies_trait(constraint, constraint_scope.as_deref(), required)
                });
            }
            _ => unreachable!("structural types are handled before named trait traversal"),
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
        if matches!(required.kind, XenoTraitKind::Struct | XenoTraitKind::Sum(_))
            && definition.body.as_ref().is_some_and(|body| {
                let body = substitute_owned_type(body, &substitutions);
                self.type_implements_trait_inner(
                    &body,
                    definition.module_path.as_deref(),
                    required,
                    visited,
                )
            })
        {
            return true;
        }
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

    /// Checks whether a concrete value/type can be assigned to an arbitrary
    /// target expression, including transparent aliases and constrained
    /// generic parameters.
    pub fn is_assignable_to(&self, candidate: &OwnedType, target: &OwnedType) -> bool {
        let target = self
            .evaluate_type(target)
            .unwrap_or_else(|_| target.clone());
        match target {
            OwnedType::Optional(inner) => self.is_assignable_to(candidate, &inner),
            OwnedType::Named { name, .. } => self.is_type_compatible(candidate, &name),
            OwnedType::Qualified {
                module_path, name, ..
            } => self.is_type_compatible_inner(
                candidate,
                self.current_module.as_deref(),
                &TypeKey { module_path, name },
                &mut HashSet::new(),
            ),
            OwnedType::Generic {
                constraint,
                constraint_scope,
                ..
            } => constraint.as_deref().is_some_and(|constraint| {
                self.satisfies_constraint(candidate, constraint, constraint_scope.as_deref())
            }),
            OwnedType::Literal(target) => matches!(candidate,
                OwnedType::Literal(value) if value.same_constant_value(&target)),
            OwnedType::Sum(targets) => targets
                .iter()
                .any(|target| self.is_assignable_to(candidate, target)),
            OwnedType::Intersection(targets) => targets
                .iter()
                .all(|target| self.is_assignable_to(candidate, target)),
            target => self
                .evaluate_type(candidate)
                .is_ok_and(|candidate| candidate == target),
        }
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

        match candidate {
            OwnedType::Optional(inner) => {
                return self.is_type_compatible_inner(inner, context, target, visited);
            }
            OwnedType::Literal(literal) => {
                return self.is_type_compatible_inner(
                    &OwnedType::named(literal.semantic_type_name()),
                    context,
                    target,
                    visited,
                );
            }
            OwnedType::Tuple(_) | OwnedType::Set(_) => {
                return self.is_type_compatible_inner(
                    &OwnedType::named("array"),
                    context,
                    target,
                    visited,
                );
            }
            OwnedType::Struct(_) => {
                return self.is_type_compatible_inner(
                    &OwnedType::named("dict"),
                    context,
                    target,
                    visited,
                );
            }
            OwnedType::Enum(_) | OwnedType::Sum(_) | OwnedType::Intersection(_) => return false,
            OwnedType::Named { .. }
            | OwnedType::Qualified { .. }
            | OwnedType::Array(_)
            | OwnedType::Generic { .. } => {}
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
            _ => unreachable!("structural types are handled before named compatibility traversal"),
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
            OwnedType::Qualified { .. } | OwnedType::Generic { .. } | OwnedType::Literal(_) => {
                ty.clone()
            }
            OwnedType::Array(inner) => {
                OwnedType::Array(Box::new(self.qualify_type(inner, context)))
            }
            OwnedType::Optional(inner) => {
                OwnedType::Optional(Box::new(self.qualify_type(inner, context)))
            }
            OwnedType::Tuple(items) => OwnedType::Tuple(
                items
                    .iter()
                    .map(|item| self.qualify_type(item, context))
                    .collect(),
            ),
            OwnedType::Set(set) => OwnedType::Set(OwnedSet {
                element_type: set
                    .element_type
                    .as_deref()
                    .map(|element| Box::new(self.qualify_type(element, context))),
                values: set.values.clone(),
            }),
            OwnedType::Struct(fields) => OwnedType::Struct(
                fields
                    .iter()
                    .map(|field| OwnedField {
                        name: field.name.clone(),
                        ty: self.qualify_type(&field.ty, context),
                        documentation: field.documentation.clone(),
                    })
                    .collect(),
            ),
            OwnedType::Enum(variants) => OwnedType::Enum(
                variants
                    .iter()
                    .map(|variant| OwnedField {
                        name: variant.name.clone(),
                        ty: self.qualify_type(&variant.ty, context),
                        documentation: variant.documentation.clone(),
                    })
                    .collect(),
            ),
            OwnedType::Sum(items) => OwnedType::Sum(
                items
                    .iter()
                    .map(|item| self.qualify_type(item, context))
                    .collect(),
            ),
            OwnedType::Intersection(items) => OwnedType::Intersection(
                items
                    .iter()
                    .map(|item| self.qualify_type(item, context))
                    .collect(),
            ),
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

pub fn type_to_owned_type(ty: &Type<'_>) -> OwnedType {
    match ty {
        Type::Simple(simple) => simple_to_owned_type(simple),
        Type::Tuple(items) => OwnedType::Tuple(items.iter().map(simple_to_owned_type).collect()),
        Type::Set(set) => OwnedType::Set(OwnedSet {
            element_type: set
                .element_type
                .as_ref()
                .map(simple_to_owned_type)
                .map(Box::new),
            values: set
                .values
                .as_ref()
                .map(|values| values.iter().map(literal_to_owned).collect()),
        }),
        Type::Struct(fields) => OwnedType::Struct(fields_to_owned(fields)),
        Type::Enum(variants) => OwnedType::Enum(fields_to_owned(variants)),
        Type::Sum(items) => OwnedType::Sum(items.iter().map(simple_to_owned_type).collect()),
        Type::Intersection(items) => {
            OwnedType::Intersection(items.iter().map(simple_to_owned_type).collect())
        }
    }
}

fn fields_to_owned(fields: &[crate::parser::KeyValExpr<'_>]) -> Vec<OwnedField> {
    fields
        .iter()
        .map(|(name, ty, documentation)| OwnedField {
            name: name.v.to_string(),
            ty: simple_to_owned_type(ty),
            documentation: documentation
                .map(|documentation| extract_documentation(documentation).to_string()),
        })
        .collect()
}

pub fn simple_to_owned_type(simple: &SimpleType<'_>) -> OwnedType {
    match simple {
        SimpleType::Optional(inner) => optional_type(simple_to_owned_type(inner)),
        SimpleType::Identifier(identifier, arguments) => OwnedType::Named {
            name: identifier.v.to_string(),
            arguments: arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(simple_to_owned_type)
                .collect(),
        },
        SimpleType::Array(identifier, arguments) => OwnedType::Array(Box::new(OwnedType::Named {
            name: identifier.v.to_string(),
            arguments: arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(simple_to_owned_type)
                .collect(),
        })),
        SimpleType::Literal(literal) => OwnedType::Literal(literal_to_owned(literal)),
    }
}

pub fn literal_to_owned(literal: &Literal<'_>) -> OwnedLiteral {
    match literal {
        Literal::Int(literal) => OwnedLiteral::Integer {
            value: literal.value.clone(),
            representation: literal.representation,
            cast: literal.cast.map(|cast| cast.v.to_string()),
        },
        Literal::Float(literal) => OwnedLiteral::Float {
            value: literal.value.clone(),
            representation: literal.representation,
            cast: literal.cast.map(|cast| cast.v.to_string()),
        },
        Literal::String(value, _) => OwnedLiteral::String(value.clone()),
        Literal::Boolean(value, _) => OwnedLiteral::Boolean(*value),
    }
}

fn narrowest_integer_type(representation: IntegerRepresentation) -> &'static str {
    let IntegerSize::Bits(bits) = representation.size else {
        return "bigint";
    };
    match (representation.signed, bits) {
        (false, 0..=4) => "u4",
        (false, 5..=8) => "u8",
        (false, 9..=16) => "u16",
        (false, 17..=32) => "u32",
        (false, 33..=64) => "u64",
        (false, 65..=128) => "u128",
        (true, 0..=4) => "i4",
        (true, 5..=8) => "i8",
        (true, 9..=16) => "i16",
        (true, 17..=32) => "i32",
        (true, 33..=64) => "i64",
        (true, 65..=128) => "i128",
        _ => "bigint",
    }
}

fn optional_type(ty: OwnedType) -> OwnedType {
    match ty {
        OwnedType::Optional(_) => ty,
        _ => OwnedType::Optional(Box::new(ty)),
    }
}

fn required_type(ty: OwnedType) -> OwnedType {
    match ty {
        OwnedType::Optional(inner) => *inner,
        _ => ty,
    }
}

fn validate_struct_keys(
    utility: &'static str,
    target: &OwnedType,
    fields: &[OwnedField],
    keys: &[String],
) -> Result<(), TypeEvaluationError> {
    for key in keys {
        if !fields.iter().any(|field| field.name == *key) {
            return Err(TypeEvaluationError::UnknownStructKey {
                utility,
                key: key.clone(),
                target: target.display_name(),
            });
        }
    }
    Ok(())
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
        OwnedType::Optional(inner) => {
            OwnedType::Optional(Box::new(substitute_owned_type(inner, substitutions)))
        }
        OwnedType::Tuple(items) => OwnedType::Tuple(
            items
                .iter()
                .map(|item| substitute_owned_type(item, substitutions))
                .collect(),
        ),
        OwnedType::Set(set) => OwnedType::Set(OwnedSet {
            element_type: set
                .element_type
                .as_deref()
                .map(|element| Box::new(substitute_owned_type(element, substitutions))),
            values: set.values.clone(),
        }),
        OwnedType::Struct(fields) => OwnedType::Struct(
            fields
                .iter()
                .map(|field| OwnedField {
                    name: field.name.clone(),
                    ty: substitute_owned_type(&field.ty, substitutions),
                    documentation: field.documentation.clone(),
                })
                .collect(),
        ),
        OwnedType::Enum(variants) => OwnedType::Enum(
            variants
                .iter()
                .map(|variant| OwnedField {
                    name: variant.name.clone(),
                    ty: substitute_owned_type(&variant.ty, substitutions),
                    documentation: variant.documentation.clone(),
                })
                .collect(),
        ),
        OwnedType::Sum(items) => OwnedType::Sum(
            items
                .iter()
                .map(|item| substitute_owned_type(item, substitutions))
                .collect(),
        ),
        OwnedType::Intersection(items) => OwnedType::Intersection(
            items
                .iter()
                .map(|item| substitute_owned_type(item, substitutions))
                .collect(),
        ),
        OwnedType::Literal(_) => ty.clone(),
        OwnedType::Generic { .. } => ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{ANY, BUILTIN_TYPES, LITERAL_SUM, STRING_LITERAL_SUM, STRUCT};

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
                body: OwnedType::named("T"),
                transparent_alias: true,
            },
        );
        let items_body = OwnedType::Array(Box::new(OwnedType::Named {
            name: "Identity".to_string(),
            arguments: vec![OwnedType::named("T")],
        }));
        hierarchy.insert_declaration(
            "test",
            "Items",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![items_body.clone()],
                body: items_body,
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
                body: OwnedType::Struct(Vec::new()),
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

    #[test]
    fn structural_constraints_support_singleton_and_multi_member_sums() {
        let mut hierarchy = utility_hierarchy();
        hierarchy.insert_declaration(
            "test",
            "TextKeys",
            declaration(OwnedType::Sum(vec![
                string_literal("id"),
                string_literal("name"),
            ])),
        );
        hierarchy.insert_declaration(
            "test",
            "MixedLiterals",
            declaration(OwnedType::Sum(vec![
                string_literal("id"),
                OwnedType::Literal(OwnedLiteral::Boolean(true)),
            ])),
        );

        assert!(hierarchy.type_implements_trait(&OwnedType::named("User"), &STRUCT));
        assert!(hierarchy.type_implements_trait(&string_literal("id"), &STRING_LITERAL_SUM));
        assert!(hierarchy.type_implements_trait(&string_literal("id"), &LITERAL_SUM));
        assert!(hierarchy.type_implements_trait(&OwnedType::named("TextKeys"), &STRING_LITERAL_SUM));
        assert!(!hierarchy
            .type_implements_trait(&OwnedType::named("MixedLiterals"), &STRING_LITERAL_SUM));
        assert!(hierarchy.type_implements_trait(&OwnedType::named("MixedLiterals"), &LITERAL_SUM));
        assert!(!hierarchy.type_implements_trait(&OwnedType::named("string"), &STRING_LITERAL_SUM));
    }

    #[test]
    fn evaluates_builtin_type_utilities_and_compositions() {
        let mut hierarchy = utility_hierarchy();
        hierarchy.insert_declaration(
            "test",
            "TextKeys",
            declaration(OwnedType::Sum(vec![
                string_literal("id"),
                string_literal("name"),
            ])),
        );
        hierarchy.insert_declaration(
            "test",
            "MixedLiterals",
            declaration(OwnedType::Sum(vec![
                string_literal("ready"),
                OwnedType::Literal(OwnedLiteral::Boolean(true)),
            ])),
        );

        assert_eq!(
            evaluate(&hierarchy, "Keyof", vec![OwnedType::named("User")]),
            OwnedType::Sum(vec![string_literal("id"), string_literal("name")])
        );
        assert_eq!(
            evaluate(
                &hierarchy,
                "Pick",
                vec![OwnedType::named("User"), string_literal("id")],
            ),
            OwnedType::Struct(vec![field("id", OwnedType::named("string"))])
        );
        assert_eq!(
            evaluate(
                &hierarchy,
                "Omit",
                vec![OwnedType::named("User"), string_literal("id")],
            ),
            OwnedType::Struct(vec![field(
                "name",
                OwnedType::Optional(Box::new(OwnedType::named("string"))),
            )])
        );

        let partial = evaluate(&hierarchy, "Partial", vec![OwnedType::named("User")]);
        assert!(matches!(
            &partial,
            OwnedType::Struct(fields)
                if fields.iter().all(|field| matches!(field.ty, OwnedType::Optional(_)))
        ));
        assert_eq!(
            hierarchy
                .evaluate_type(&OwnedType::Named {
                    name: "Complete".to_string(),
                    arguments: vec![partial],
                })
                .expect("Complete should evaluate"),
            OwnedType::Struct(vec![
                field("id", OwnedType::named("string")),
                field("name", OwnedType::named("string")),
            ])
        );

        let optional = OwnedType::Optional(Box::new(OwnedType::named("string")));
        assert_eq!(
            evaluate(&hierarchy, "Required", vec![optional.clone()]),
            OwnedType::named("string")
        );
        assert_eq!(
            evaluate(&hierarchy, "Required", vec![OwnedType::named("string")]),
            OwnedType::named("string"),
            "Required is idempotent"
        );

        let nested = OwnedType::Named {
            name: "Partial".to_string(),
            arguments: vec![OwnedType::Named {
                name: "Pick".to_string(),
                arguments: vec![OwnedType::named("User"), string_literal("id")],
            }],
        };
        assert_eq!(
            hierarchy.evaluate_type(&nested).expect("utilities compose"),
            OwnedType::Struct(vec![field(
                "id",
                OwnedType::Optional(Box::new(OwnedType::named("string"))),
            )])
        );
    }

    #[test]
    fn pick_and_omit_reject_unknown_struct_keys() {
        let hierarchy = utility_hierarchy();

        for utility in ["Pick", "Omit"] {
            let error = hierarchy
                .evaluate_type(&OwnedType::Named {
                    name: utility.to_string(),
                    arguments: vec![OwnedType::named("User"), string_literal("missing")],
                })
                .expect_err("unknown keys must be rejected");
            assert_eq!(
                error.to_string(),
                format!(
                    "Type utility '{utility}' cannot select unknown field 'missing' from 'test::User'."
                )
            );
        }
    }

    fn utility_hierarchy() -> TypeHierarchy {
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.set_current_module("test");
        hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            hierarchy.insert_semantic_type(semantic_type);
        }
        hierarchy.insert_declaration(
            "test",
            "User",
            declaration(OwnedType::Struct(vec![
                field("id", OwnedType::named("string")),
                field(
                    "name",
                    OwnedType::Optional(Box::new(OwnedType::named("string"))),
                ),
            ])),
        );
        hierarchy
    }

    fn declaration(body: OwnedType) -> TypeDeclarationInfo {
        TypeDeclarationInfo {
            generic_params: Vec::new(),
            parents: match &body {
                OwnedType::Struct(_) => vec![OwnedType::named("dict")],
                OwnedType::Sum(_) => vec![OwnedType::named("any")],
                _ => vec![body.clone()],
            },
            body,
            transparent_alias: false,
        }
    }

    fn field(name: &str, ty: OwnedType) -> OwnedField {
        OwnedField {
            name: name.to_string(),
            ty,
            documentation: None,
        }
    }

    fn string_literal(value: &str) -> OwnedType {
        OwnedType::Literal(OwnedLiteral::String(value.to_string()))
    }

    fn evaluate(hierarchy: &TypeHierarchy, name: &str, arguments: Vec<OwnedType>) -> OwnedType {
        hierarchy
            .evaluate_type(&OwnedType::Named {
                name: name.to_string(),
                arguments,
            })
            .expect("utility should evaluate")
    }

    fn generic(name: &str) -> GenericParameterInfo {
        GenericParameterInfo {
            name: name.to_string(),
            constraint: None,
            constraint_scope: None,
        }
    }
}
