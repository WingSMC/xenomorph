//! Lifetime-free metadata exchanged with plugins and retained by semantic consumers.
//! Names are stable graph IDs: core names are bare, plugin names use `namespace::name`. Source
//! declarations retain their separate module-qualified hierarchy identity.
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{
    XenoAnnotation, XenoAnnotationKind, XenoConstraint, XenoParent, XenoTrait, XenoTraitKind,
    XenoType,
};
use crate::parser::{FloatRepresentation, FloatSize, IntegerRepresentation, IntegerSize};

#[derive(Serialize, Deserialize)]
#[serde(remote = "IntegerSize")]
enum IntegerSizeDef {
    Bits(u64),
    Arbitrary,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "IntegerRepresentation")]
pub(crate) struct IntegerRepresentationDef {
    signed: bool,
    #[serde(with = "IntegerSizeDef")]
    size: IntegerSize,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "FloatSize")]
enum FloatSizeDef {
    F32,
    F64,
    Decimal,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "FloatRepresentation")]
pub(crate) struct FloatRepresentationDef {
    signed: bool,
    precision: u64,
    scale: u64,
    #[serde(with = "FloatSizeDef")]
    size: FloatSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnedConstraint {
    Type(String),
    Trait(String),
}

impl OwnedConstraint {
    pub fn name(&self) -> &str {
        match self {
            Self::Type(name) | Self::Trait(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnedTraitKind {
    Semantic,
    Struct,
    Sum(String),
    Expression,
    Literal,
    LiteralType,
    RegexLiteral,
    Identifier,
    Type,
    Annotation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedTrait {
    pub name: String,
    pub documentation: Option<String>,
    pub kind: OwnedTraitKind,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedGenericParam {
    pub name: String,
    pub constraint: Option<OwnedConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedSemanticType {
    pub name: String,
    pub documentation: Option<String>,
    pub generic_params: Vec<OwnedGenericParam>,
    pub parents: Vec<OwnedConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedParam {
    pub name: String,
    pub constraint: OwnedConstraint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedAnnotation {
    pub name: String,
    pub documentation: Option<String>,
    pub kind: XenoAnnotationKind,
    pub params: Vec<OwnedParam>,
    pub variadic: bool,
}

impl OwnedAnnotation {
    pub fn target_parameter(&self) -> Option<&OwnedParam> {
        self.params.first()
    }

    pub fn explicit_parameters(&self) -> &[OwnedParam] {
        self.params.get(1..).unwrap_or(&[])
    }

    pub fn parameter_at(&self, index: usize) -> Option<&OwnedParam> {
        let params = self.explicit_parameters();
        params
            .get(index)
            .or_else(|| self.variadic.then(|| params.last()).flatten())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataGraph {
    #[serde(with = "unique_named_map")]
    pub types: HashMap<String, OwnedSemanticType>,
    #[serde(with = "unique_named_map")]
    pub traits: HashMap<String, OwnedTrait>,
    #[serde(with = "unique_named_map")]
    pub annotations: HashMap<String, OwnedAnnotation>,
}

impl MetadataGraph {
    pub fn builtins() -> Self {
        Self::from_all_descriptors(
            super::BUILTIN_TYPES,
            super::BUILTIN_TRAITS,
            super::BUILTIN_ANNOTATIONS,
        )
    }

    /// Rejects duplicate declarations and invalid edges without changing this graph.
    pub fn merge(&mut self, metadata: &Self) -> Result<(), String> {
        for (kind, collision) in [
            (
                "type",
                metadata
                    .types
                    .keys()
                    .find(|key| self.types.contains_key(*key)),
            ),
            (
                "trait",
                metadata
                    .traits
                    .keys()
                    .find(|key| self.traits.contains_key(*key)),
            ),
            (
                "annotation",
                metadata
                    .annotations
                    .keys()
                    .find(|key| self.annotations.contains_key(*key)),
            ),
        ] {
            if let Some(name) = collision {
                return Err(format!("Duplicate metadata {kind} '{name}'"));
            }
        }
        let mut merged = self.clone();
        merged.types.extend(metadata.types.clone());
        merged.traits.extend(metadata.traits.clone());
        merged.annotations.extend(metadata.annotations.clone());
        merged.validate()?;
        *self = merged;
        Ok(())
    }

    /// Checks all identities and typed edges before publishing decoded metadata.
    /// Cyclic edges are permitted: runtime traversal uses visited identities.
    pub fn validate(&self) -> Result<(), String> {
        for (key, ty) in &self.types {
            validate_identity(key, &ty.name)?;
            validate_parameter_names(
                ty.generic_params
                    .iter()
                    .map(|parameter| parameter.name.as_str()),
                &ty.name,
            )?;
            let mut parents = HashSet::new();
            for parent in &ty.parents {
                if !parents.insert((matches!(parent, OwnedConstraint::Trait(_)), parent.name())) {
                    return Err(format!(
                        "Type '{}' repeats parent '{}'",
                        ty.name,
                        parent.name()
                    ));
                }
                self.validate_constraint(parent)?;
            }
            for parameter in &ty.generic_params {
                if let Some(constraint) = &parameter.constraint {
                    self.validate_constraint(constraint)?;
                }
            }
        }
        for (key, descriptor) in &self.traits {
            validate_identity(key, &descriptor.name)?;
            let mut parents = HashSet::new();
            for parent in &descriptor.parents {
                if !parents.insert(parent) {
                    return Err(format!(
                        "Trait '{}' repeats parent '{parent}'",
                        descriptor.name
                    ));
                }
                self.validate_constraint(&OwnedConstraint::Trait(parent.clone()))?;
            }
            if let OwnedTraitKind::Sum(member) = &descriptor.kind {
                self.validate_constraint(&OwnedConstraint::Trait(member.clone()))?;
            }
        }
        for (key, annotation) in &self.annotations {
            validate_identity(key, &annotation.name)?;
            validate_parameter_names(
                annotation
                    .params
                    .iter()
                    .map(|parameter| parameter.name.as_str()),
                &annotation.name,
            )?;
            for parameter in &annotation.params {
                self.validate_constraint(&parameter.constraint)?;
            }
            if annotation.variadic && annotation.explicit_parameters().is_empty() {
                return Err(format!(
                    "Variadic annotation '{}' has no explicit parameter to repeat",
                    annotation.name
                ));
            }
        }
        Ok(())
    }

    fn validate_constraint(&self, constraint: &OwnedConstraint) -> Result<(), String> {
        validate_constraint(
            constraint,
            |name| self.types.contains_key(name),
            |name| self.traits.contains_key(name),
        )
    }

    /// Deep-copies root declarations and every dependency referenced by their
    /// parents, generic constraints, annotation parameters, and sum traits.
    pub fn from_descriptors(types: &[&XenoType], annotations: &[&XenoAnnotation]) -> Self {
        Self::from_all_descriptors(types, &[], annotations)
    }
}

pub(crate) fn validate_identity(key: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Metadata definition has an empty name".into());
    }
    if key != name {
        return Err(format!(
            "Metadata key '{key}' does not match definition name '{name}'"
        ));
    }
    Ok(())
}

pub(crate) fn validate_parameter_names<'a>(
    names: impl Iterator<Item = &'a str>,
    owner: &str,
) -> Result<(), String> {
    let mut unique = HashSet::new();
    for name in names {
        if name.trim().is_empty() || !unique.insert(name) {
            return Err(format!(
                "Metadata '{owner}' has an empty or repeated parameter name '{name}'"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_constraint(
    constraint: &OwnedConstraint,
    has_type: impl Fn(&str) -> bool,
    has_trait: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let (kind, present) = match constraint {
        OwnedConstraint::Type(name) => ("type", has_type(name)),
        OwnedConstraint::Trait(name) => ("trait", has_trait(name)),
    };
    if constraint.name().trim().is_empty() || !present {
        return Err(format!(
            "Metadata references missing {kind} '{}'",
            constraint.name()
        ));
    }
    Ok(())
}

pub(crate) mod unique_named_map {
    use super::*;
    use serde::de::{MapAccess, Visitor};

    pub fn serialize<T: Serialize, S: serde::Serializer>(
        map: &HashMap<String, T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.serialize(serializer)
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HashMap<String, T>, D::Error> {
        struct UniqueMap<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for UniqueMap<T> {
            type Value = HashMap<String, T>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a metadata map with unique names")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = HashMap::new();
                while let Some((key, value)) = access.next_entry::<String, T>()? {
                    if map.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "Repeated metadata definition '{key}'"
                        )));
                    }
                }
                Ok(map)
            }
        }
        deserializer.deserialize_map(UniqueMap(std::marker::PhantomData))
    }
}

impl MetadataGraph {
    /// Also registers standalone traits that no type or annotation references.
    pub fn from_all_descriptors(
        types: &[&XenoType],
        traits: &[&XenoTrait],
        annotations: &[&XenoAnnotation],
    ) -> Self {
        let mut graph = Self::default();
        for ty in types {
            graph.insert_type_descriptor(ty);
        }
        for descriptor in traits {
            graph.insert_trait_descriptor(descriptor);
        }
        for annotation in annotations {
            graph.insert_annotation_descriptor(annotation);
        }
        graph
    }
}

impl MetadataGraph {
    pub fn insert_trait_descriptor(&mut self, descriptor: &XenoTrait) {
        if self.traits.contains_key(descriptor.name) {
            return;
        }
        let kind = match descriptor.kind {
            XenoTraitKind::Semantic => OwnedTraitKind::Semantic,
            XenoTraitKind::Struct => OwnedTraitKind::Struct,
            XenoTraitKind::Sum(member) => OwnedTraitKind::Sum(member.name.to_string()),
            XenoTraitKind::Expression => OwnedTraitKind::Expression,
            XenoTraitKind::Literal => OwnedTraitKind::Literal,
            XenoTraitKind::LiteralType => OwnedTraitKind::LiteralType,
            XenoTraitKind::RegexLiteral => OwnedTraitKind::RegexLiteral,
            XenoTraitKind::Identifier => OwnedTraitKind::Identifier,
            XenoTraitKind::Type => OwnedTraitKind::Type,
            XenoTraitKind::Annotation => OwnedTraitKind::Annotation,
        };
        self.traits.insert(
            descriptor.name.to_string(),
            OwnedTrait {
                name: descriptor.name.to_string(),
                documentation: descriptor.documentation.map(str::to_string),
                kind,
                parents: descriptor
                    .parents
                    .unwrap_or(&[])
                    .iter()
                    .map(|parent| parent.name.to_string())
                    .collect(),
            },
        );
        for parent in descriptor.parents.unwrap_or(&[]) {
            self.insert_trait_descriptor(parent);
        }
        if let XenoTraitKind::Sum(member) = descriptor.kind {
            self.insert_trait_descriptor(member);
        }
    }

    fn convert_constraint(&mut self, constraint: XenoConstraint) -> OwnedConstraint {
        match constraint {
            XenoConstraint::Type(ty) => {
                self.insert_type_descriptor(ty);
                OwnedConstraint::Type(ty.name.to_string())
            }
            XenoConstraint::Trait(descriptor) => {
                self.insert_trait_descriptor(descriptor);
                OwnedConstraint::Trait(descriptor.name.to_string())
            }
        }
    }

    pub fn insert_type_descriptor(&mut self, descriptor: &XenoType) {
        if self.types.contains_key(descriptor.name) {
            return;
        }
        // Reserve the ID before following edges, including recursive generic constraints.
        self.types.insert(
            descriptor.name.to_string(),
            OwnedSemanticType {
                name: descriptor.name.to_string(),
                documentation: descriptor.documentation.map(str::to_string),
                generic_params: Vec::new(),
                parents: Vec::new(),
            },
        );
        let parents = descriptor
            .parents
            .unwrap_or(&[])
            .iter()
            .map(|parent| {
                self.convert_constraint(match parent {
                    XenoParent::Type(ty) => XenoConstraint::Type(ty),
                    XenoParent::Trait(descriptor) => XenoConstraint::Trait(descriptor),
                })
            })
            .collect();
        let generic_params = descriptor
            .generic_params
            .unwrap_or(&[])
            .iter()
            .map(|param| OwnedGenericParam {
                name: param.name.to_string(),
                constraint: param
                    .constraint
                    .map(|constraint| self.convert_constraint(constraint)),
            })
            .collect();
        let ty = self.types.get_mut(descriptor.name).unwrap();
        ty.parents = parents;
        ty.generic_params = generic_params;
    }

    pub fn insert_annotation_descriptor(&mut self, descriptor: &XenoAnnotation) {
        let params = descriptor
            .params
            .iter()
            .map(|param| OwnedParam {
                name: param.name.to_string(),
                constraint: self.convert_constraint(param.constraint),
            })
            .collect();
        self.annotations.insert(
            descriptor.name.to_string(),
            OwnedAnnotation {
                name: descriptor.name.to_string(),
                documentation: descriptor.documentation.map(str::to_string),
                kind: descriptor.kind,
                params,
                variadic: descriptor.variadic,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{
        GenericParam, OwnedLiteral, OwnedType, ScopeInfo, TypeDeclarationInfo, TypeHierarchy,
        BUILTIN_ANNOTATIONS, BUILTIN_TRAITS, BUILTIN_TYPES,
    };
    use std::collections::{HashMap, HashSet};

    static FIRST: XenoTrait = XenoTrait {
        name: "PluginFirst",
        documentation: Some("First trait"),
        kind: XenoTraitKind::Semantic,
        parents: Some(&[&SECOND]),
    };
    static SECOND: XenoTrait = XenoTrait {
        name: "PluginSecond",
        documentation: None,
        kind: XenoTraitKind::Semantic,
        parents: Some(&[&FIRST]),
    };
    static SUM: XenoTrait = XenoTrait {
        name: "PluginSum",
        documentation: None,
        kind: XenoTraitKind::Sum(&FIRST),
        parents: None,
    };
    static RECURSIVE: XenoType = XenoType {
        name: "PluginRecursive",
        documentation: Some("Recursive type"),
        generic_params: Some(&[&GenericParam {
            name: "T",
            constraint: Some(XenoConstraint::Type(&RECURSIVE)),
        }]),
        parents: Some(&[XenoParent::Type(&CHILD), XenoParent::Trait(&SUM)]),
    };
    static CHILD: XenoType = XenoType {
        name: "PluginChild",
        documentation: None,
        generic_params: None,
        parents: Some(&[XenoParent::Type(&RECURSIVE)]),
    };
    static ANNOTATION: XenoAnnotation = XenoAnnotation {
        name: "pluginAnnotation",
        documentation: Some("Plugin annotation"),
        kind: XenoAnnotationKind::Meta,
        params: &[
            &super::super::XenoParam {
                name: "self",
                constraint: XenoConstraint::Type(&RECURSIVE),
            },
            &super::super::XenoParam {
                name: "value",
                constraint: XenoConstraint::Trait(&SUM),
            },
        ],
        variadic: true,
    };

    #[test]
    fn annotation_conversion_copies_all_recursive_dependencies() {
        let graph = MetadataGraph::from_descriptors(&[], &[&ANNOTATION]);
        graph.validate().unwrap();
        assert_eq!(graph.types.len(), 2);
        assert_eq!(graph.traits.len(), 3);
        assert_eq!(
            graph.types["PluginRecursive"].generic_params[0].constraint,
            Some(OwnedConstraint::Type("PluginRecursive".into()))
        );
        assert_eq!(
            graph.traits["PluginSum"].kind,
            OwnedTraitKind::Sum("PluginFirst".into())
        );
        assert_eq!(
            graph.types["PluginRecursive"].documentation.as_deref(),
            Some("Recursive type")
        );
        let encoded = serde_json::to_string(&graph).unwrap();
        let decoded: MetadataGraph = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, graph);
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.insert_metadata(&decoded).unwrap();
        drop(decoded);
        drop(graph);
        assert!(hierarchy.trait_is_or_inherits("PluginFirst", "PluginSecond"));
        assert!(!hierarchy.trait_is_or_inherits("PluginFirst", "Missing"));
        assert!(hierarchy.implements_trait(&OwnedType::named("PluginChild"), "PluginSum"));
        assert!(hierarchy
            .descends_from_semantic_type(&OwnedType::named("PluginChild"), "PluginRecursive"));
    }

    #[test]
    fn recursive_owned_sum_traits_terminate() {
        let mut graph = MetadataGraph::default();
        graph.traits.insert(
            "RecursiveSum".into(),
            OwnedTrait {
                name: "RecursiveSum".into(),
                documentation: None,
                kind: OwnedTraitKind::Sum("RecursiveSum".into()),
                parents: Vec::new(),
            },
        );
        let mut hierarchy = TypeHierarchy::default();
        graph.validate().unwrap();
        hierarchy.insert_metadata(&graph).unwrap();
        assert!(!hierarchy.implements_trait(
            &OwnedType::Literal(OwnedLiteral::String("value".into())),
            "RecursiveSum",
        ));
    }

    #[test]
    fn owned_scope_roundtrip_preserves_qualified_types_and_large_numbers() {
        let mut scope = ScopeInfo {
            module_path: "local".into(),
            abs_path: std::env::current_dir().unwrap().join("schema"),
            own_types: vec!["Value".into()],
            imported_types: HashMap::from([("other".into(), vec!["Value".into()])]),
            builtin_types: HashSet::new(),
            known_annotations: HashSet::new(),
            type_hierarchy: TypeHierarchy::default(),
            annotations: HashMap::new(),
        };
        scope
            .insert_metadata(&MetadataGraph::from_all_descriptors(
                BUILTIN_TYPES,
                BUILTIN_TRAITS,
                BUILTIN_ANNOTATIONS,
            ))
            .unwrap();
        scope.type_hierarchy.set_current_module("local");
        scope
            .type_hierarchy
            .register_module("local", vec!["other".into()]);
        let values = [
            (
                "local",
                OwnedLiteral::Integer {
                    value: "1234567890123456789012345678901234567890".parse().unwrap(),
                    representation: IntegerRepresentation {
                        signed: false,
                        size: IntegerSize::Arbitrary,
                    },
                    cast: Some("bigint".into()),
                },
            ),
            (
                "other",
                OwnedLiteral::Float {
                    value: "12345678901234567890.12345678901234567890".parse().unwrap(),
                    representation: FloatRepresentation {
                        signed: true,
                        precision: 40,
                        scale: 20,
                        size: FloatSize::Decimal,
                    },
                    cast: None,
                },
            ),
        ];
        for (module, value) in &values {
            scope.type_hierarchy.insert_declaration(
                *module,
                "Value",
                TypeDeclarationInfo {
                    generic_params: Vec::new(),
                    parents: Vec::new(),
                    body: OwnedType::Literal(value.clone()),
                    transparent_alias: false,
                },
            );
        }
        let json = serde_json::to_string(&scope).unwrap();
        assert!(json.contains("\"1234567890123456789012345678901234567890\""));
        let restored: ScopeInfo = serde_json::from_str(&json).unwrap();
        restored.validate_metadata().unwrap();
        assert_eq!(restored.abs_path, scope.abs_path);
        for (module, value) in values {
            assert_eq!(
                restored
                    .type_hierarchy
                    .get_type_in(Some(module), "Value")
                    .unwrap()
                    .body,
                Some(OwnedType::Literal(value))
            );
        }
        assert!(restored.implements_trait(&OwnedType::named("u8"), "Numeric"));
        assert!(restored.descends_from_semantic_type(&OwnedType::named("NEVER"), "string"));
        assert!(restored.find_annotation("min").is_some());
        assert!(restored.has_type("Value"));
        let mut invalid = restored.clone();
        invalid.type_hierarchy.set_current_module("wrong");
        assert!(invalid.validate().unwrap_err().contains("lookup context"));
        let mut invalid = restored.clone();
        invalid.annotations.get_mut("min").unwrap().params[0].constraint =
            OwnedConstraint::Trait("Missing".into());
        assert!(invalid
            .validate_metadata()
            .unwrap_err()
            .contains("missing trait"));
        let mut invalid = restored.clone();
        invalid.known_annotations.insert("Missing".into());
        assert!(invalid.validate_metadata().is_err());
        let mut invalid = restored.clone();
        invalid
            .imported_types
            .get_mut("other")
            .unwrap()
            .push("Missing".into());
        assert!(invalid.validate_metadata().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn scope_roundtrip_preserves_non_unicode_native_paths() {
        use std::os::windows::ffi::OsStringExt;
        let scope = ScopeInfo {
            module_path: String::new(),
            abs_path: std::ffi::OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800]).into(),
            own_types: Vec::new(),
            imported_types: HashMap::new(),
            builtin_types: HashSet::new(),
            known_annotations: HashSet::new(),
            type_hierarchy: TypeHierarchy::default(),
            annotations: HashMap::new(),
        };
        let restored: ScopeInfo =
            serde_json::from_str(&serde_json::to_string(&scope).unwrap()).unwrap();
        assert_eq!(restored.abs_path, scope.abs_path);
    }

    #[test]
    fn validation_rejects_missing_and_wrong_kind_dependencies() {
        let original = MetadataGraph::from_descriptors(&[], &[&ANNOTATION]);
        let mut graph = original.clone();
        graph.types.get_mut("PluginRecursive").unwrap().parents[0] =
            OwnedConstraint::Type("PluginFirst".into());
        assert!(graph.validate().unwrap_err().contains("missing type"));
        let mut graph = original.clone();
        graph.traits.get_mut("PluginFirst").unwrap().parents = vec!["Missing".into()];
        assert!(graph.validate().unwrap_err().contains("missing trait"));
        let mut graph = original.clone();
        graph.traits.get_mut("PluginSum").unwrap().kind =
            OwnedTraitKind::Sum("PluginRecursive".into());
        assert!(graph.validate().unwrap_err().contains("missing trait"));
        let mut graph = original.clone();
        graph
            .types
            .get_mut("PluginRecursive")
            .unwrap()
            .generic_params[0]
            .constraint = Some(OwnedConstraint::Trait("Missing".into()));
        assert!(graph.validate().unwrap_err().contains("missing trait"));
        let mut graph = original;
        graph
            .annotations
            .get_mut("pluginAnnotation")
            .unwrap()
            .params[0]
            .constraint = OwnedConstraint::Type("Missing".into());
        assert!(graph.validate().unwrap_err().contains("missing type"));
    }

    #[test]
    fn validation_rejects_mismatched_empty_and_repeated_definitions() {
        let original = MetadataGraph::from_descriptors(&[], &[&ANNOTATION]);
        let mut graph = original.clone();
        graph.types.get_mut("PluginRecursive").unwrap().name = "Other".into();
        assert!(graph.validate().unwrap_err().contains("does not match"));
        let mut graph = original.clone();
        graph.traits.get_mut("PluginFirst").unwrap().name = String::new();
        assert!(graph.validate().unwrap_err().contains("empty name"));
        let mut graph = original.clone();
        graph.annotations.get_mut("pluginAnnotation").unwrap().name = "Other".into();
        assert!(graph.validate().unwrap_err().contains("does not match"));
        let mut graph = original.clone();
        let ty = graph.types.get_mut("PluginRecursive").unwrap();
        ty.generic_params.push(ty.generic_params[0].clone());
        assert!(graph.validate().unwrap_err().contains("repeated parameter"));
        let mut graph = original.clone();
        let annotation = graph.annotations.get_mut("pluginAnnotation").unwrap();
        annotation.params[1].name.clear();
        assert!(graph.validate().unwrap_err().contains("empty or repeated"));
        let mut graph = original.clone();
        let ty = graph.types.get_mut("PluginRecursive").unwrap();
        ty.parents.push(ty.parents[0].clone());
        assert!(graph.validate().unwrap_err().contains("repeats parent"));
        let mut graph = original;
        let annotation = graph.annotations.get_mut("pluginAnnotation").unwrap();
        annotation.params.truncate(1);
        assert!(graph
            .validate()
            .unwrap_err()
            .contains("no explicit parameter"));
    }

    #[test]
    fn decoding_rejects_repeated_metadata_map_keys() {
        let graph = MetadataGraph::from_descriptors(&[], &[&ANNOTATION]);
        for (field, name, definition) in [
            (
                "types",
                "PluginRecursive",
                serde_json::to_string(&graph.types["PluginRecursive"]).unwrap(),
            ),
            (
                "traits",
                "PluginFirst",
                serde_json::to_string(&graph.traits["PluginFirst"]).unwrap(),
            ),
            (
                "annotations",
                "pluginAnnotation",
                serde_json::to_string(&graph.annotations["pluginAnnotation"]).unwrap(),
            ),
        ] {
            let repeated = format!("{{\"{name}\":{definition},\"{name}\":{definition}}}");
            let json = format!(
                "{{\"types\":{},\"traits\":{},\"annotations\":{}}}",
                if field == "types" { &repeated } else { "{}" },
                if field == "traits" { &repeated } else { "{}" },
                if field == "annotations" {
                    &repeated
                } else {
                    "{}"
                }
            );
            let error = serde_json::from_str::<MetadataGraph>(&json)
                .unwrap_err()
                .to_string();
            assert!(error.contains("Repeated metadata definition"), "{error}");
        }
    }
}
