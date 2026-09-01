use crate::parser::SimpleType;
use crate::{TokenData, XenoDiagSeverity, XenoDiagnostic};

use super::{simple_to_owned_type, OwnedType, TypeHierarchy};

/// Creates a fatal diagnostic when a source type contains a static semantic
/// type that has no native mapping for the target language.
///
/// Transparent aliases and nested type arguments are resolved recursively.
/// Source declarations and generic parameters remain representable by name.
pub fn unsupported_target_type_diagnostic<'src>(
    target: &str,
    ty: &SimpleType<'src>,
    hierarchy: &TypeHierarchy,
    has_native_mapping: impl FnMut(&str) -> bool,
) -> Option<XenoDiagnostic<'src>> {
    let identifier = simple_type_identifier(ty)?;
    let resolved = hierarchy.resolve_transparent_aliases(&simple_to_owned_type(ty));
    let type_name = first_unmapped_target_type(&resolved, hierarchy, has_native_mapping)?;

    Some(XenoDiagnostic {
        location: identifier.clone(),
        message: format!(
            "{target} cannot represent type '{type_name}' because it has no native mapping."
        ),
        severity: XenoDiagSeverity::Err,
    })
}

fn simple_type_identifier<'src>(ty: &SimpleType<'src>) -> Option<&'src TokenData<'src>> {
    match ty {
        SimpleType::Identifier(identifier, _)
        | SimpleType::OptionalIdentifier(identifier, _)
        | SimpleType::Array(identifier, _)
        | SimpleType::OptionalArray(identifier, _) => Some(identifier),
        SimpleType::Literal(_) | SimpleType::OptionalLiteral(_) => None,
    }
}

fn first_unmapped_target_type(
    ty: &OwnedType,
    hierarchy: &TypeHierarchy,
    mut has_native_mapping: impl FnMut(&str) -> bool,
) -> Option<String> {
    first_unmapped_target_type_inner(ty, hierarchy, &mut has_native_mapping)
}

fn first_unmapped_target_type_inner(
    ty: &OwnedType,
    hierarchy: &TypeHierarchy,
    has_native_mapping: &mut impl FnMut(&str) -> bool,
) -> Option<String> {
    match ty {
        OwnedType::Array(inner) => {
            first_unmapped_target_type_inner(inner, hierarchy, has_native_mapping)
        }
        OwnedType::Generic { .. } => None,
        OwnedType::Named { name, arguments } => {
            let is_static = hierarchy
                .get_type(name)
                .is_some_and(|definition| definition.module_path.is_none());
            if is_static && !has_native_mapping(name) {
                return Some(name.clone());
            }
            arguments.iter().find_map(|argument| {
                first_unmapped_target_type_inner(argument, hierarchy, has_native_mapping)
            })
        }
        OwnedType::Qualified {
            module_path,
            name,
            arguments,
        } => {
            if module_path.is_none() && !has_native_mapping(name) {
                return Some(name.clone());
            }
            arguments.iter().find_map(|argument| {
                first_unmapped_target_type_inner(argument, hierarchy, has_native_mapping)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{TypeDeclarationInfo, XenoType};

    static UNMAPPED: XenoType = XenoType {
        name: "unmapped",
        documentation: None,
        generic_params: None,
        parents: None,
    };

    #[test]
    fn reports_resolved_unmapped_alias_at_the_source_identifier() {
        let alias = TokenData {
            v: "Alias",
            l: 3,
            c: 7,
        };
        let ty = SimpleType::Identifier(&alias, None);
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.set_current_module("test");
        hierarchy.register_module("test", Vec::new());
        hierarchy.insert_semantic_type(&UNMAPPED);
        hierarchy.insert_declaration(
            "test",
            "Alias",
            TypeDeclarationInfo {
                generic_params: Vec::new(),
                parents: vec![OwnedType::named("unmapped")],
                transparent_alias: true,
            },
        );

        let diagnostic = unsupported_target_type_diagnostic("Target", &ty, &hierarchy, |_| false)
            .expect("the resolved static type has no native mapping");

        assert_eq!(diagnostic.location.v, "Alias");
        assert_eq!(diagnostic.location.l, 3);
        assert_eq!(diagnostic.location.c, 7);
        assert_eq!(
            diagnostic.message,
            "Target cannot represent type 'unmapped' because it has no native mapping."
        );
        assert_eq!(diagnostic.severity, XenoDiagSeverity::Err);
    }

    #[test]
    fn accepts_mapped_static_types_and_source_declarations() {
        let mapped = TokenData {
            v: "unmapped",
            l: 0,
            c: 0,
        };
        let source = TokenData {
            v: "SourceType",
            l: 0,
            c: 0,
        };
        let mut hierarchy = TypeHierarchy::default();
        hierarchy.set_current_module("test");
        hierarchy.register_module("test", Vec::new());
        hierarchy.insert_semantic_type(&UNMAPPED);
        hierarchy.insert_declaration(
            "test",
            "SourceType",
            TypeDeclarationInfo {
                generic_params: Vec::new(),
                parents: vec![OwnedType::named("unmapped")],
                transparent_alias: false,
            },
        );

        assert!(unsupported_target_type_diagnostic(
            "Target",
            &SimpleType::Identifier(&mapped, None),
            &hierarchy,
            |name| name == "unmapped",
        )
        .is_none());
        assert!(unsupported_target_type_diagnostic(
            "Target",
            &SimpleType::Identifier(&source, None),
            &hierarchy,
            |_| false,
        )
        .is_none());
    }
}
