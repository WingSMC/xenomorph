use std::collections::HashMap;

use crate::{
    lexer::{Lexer, TokenVariant},
    semantic::*,
};

pub(super) fn validate_namespace(namespace: &str) -> Result<(), String> {
    let tokens = Lexer::tokenize(namespace)
        .map_err(|_| format!("Invalid plugin namespace '{namespace}'"))?;
    if !matches!(tokens.as_slice(), [(TokenVariant::Identifier, token)] if token.text(namespace) == namespace)
    {
        return Err(format!(
            "Invalid plugin namespace '{namespace}': expected an identifier"
        ));
    }
    Ok(())
}

/// Qualify declarations before copying dependencies, so a plugin can use a core
/// name without confusing its own descriptor with the actual core descriptor.
pub(super) fn from_descriptors(
    namespace: &str,
    types: &[&XenoType],
    annotations: &[&XenoAnnotation],
) -> Result<MetadataGraph, String> {
    validate_namespace(namespace)?;
    let mut builder = Builder {
        namespace,
        graph: MetadataGraph::default(),
        identities: HashMap::new(),
    };
    for ty in types {
        builder.ty(ty, true)?;
    }
    for annotation in annotations {
        let name = builder.identity(
            "annotation",
            annotation.name,
            *annotation as *const _ as usize,
        )?;
        if builder.graph.annotations.contains_key(&name) {
            return Err(format!("Duplicate plugin annotation '{name}'"));
        }
        let params = annotation
            .params
            .iter()
            .map(|param| {
                Ok(OwnedParam {
                    name: param.name.into(),
                    constraint: builder.constraint(param.constraint)?,
                    optional: param.optional,
                })
            })
            .collect::<Result<_, String>>()?;
        builder.graph.annotations.insert(
            name.clone(),
            OwnedAnnotation {
                name,
                documentation: annotation.documentation.map(str::to_string),
                kind: annotation.kind,
                params,
                variadic: annotation.variadic,
            },
        );
    }
    let mut merged = MetadataGraph::builtins();
    merged.merge(&builder.graph)?;
    Ok(builder.graph)
}

struct Builder<'a> {
    namespace: &'a str,
    graph: MetadataGraph,
    identities: HashMap<(&'static str, String), usize>,
}

impl Builder<'_> {
    fn identity(
        &mut self,
        kind: &'static str,
        local: &str,
        pointer: usize,
    ) -> Result<String, String> {
        validate_namespace(local).map_err(|_| {
            format!("Invalid plugin {kind} name '{local}': expected an unqualified identifier")
        })?;
        let name = format!("{}::{local}", self.namespace);
        if let Some(previous) = self.identities.insert((kind, name.clone()), pointer) {
            if previous != pointer {
                return Err(format!("Duplicate plugin {kind} '{name}'"));
            }
        }
        Ok(name)
    }

    fn constraint(&mut self, constraint: XenoConstraint) -> Result<OwnedConstraint, String> {
        Ok(match constraint {
            XenoConstraint::Type(ty) => OwnedConstraint::Type(self.ty(ty, false)?),
            XenoConstraint::Trait(tr) => OwnedConstraint::Trait(self.tr(tr)?),
        })
    }

    fn ty(&mut self, ty: &XenoType, exported: bool) -> Result<String, String> {
        if !exported
            && BUILTIN_TYPES
                .iter()
                .any(|builtin| std::ptr::eq(*builtin, ty))
        {
            return Ok(ty.name.into());
        }
        let name = self.identity("type", ty.name, ty as *const _ as usize)?;
        if self.graph.types.contains_key(&name) {
            return Ok(name);
        }
        self.graph.types.insert(
            name.clone(),
            OwnedSemanticType {
                name: name.clone(),
                documentation: ty.documentation.map(str::to_string),
                parents: Vec::new(),
                generic_params: Vec::new(),
            },
        );
        let parents = ty
            .parents
            .unwrap_or(&[])
            .iter()
            .map(|parent| {
                self.constraint(match parent {
                    XenoParent::Type(ty) => XenoConstraint::Type(ty),
                    XenoParent::Trait(tr) => XenoConstraint::Trait(tr),
                })
            })
            .collect::<Result<_, _>>()?;
        let generic_params = ty
            .generic_params
            .unwrap_or(&[])
            .iter()
            .map(|param| {
                Ok(OwnedGenericParam {
                    name: param.name.into(),
                    constraint: param
                        .constraint
                        .map(|constraint| self.constraint(constraint))
                        .transpose()?,
                })
            })
            .collect::<Result<_, String>>()?;
        let definition = self.graph.types.get_mut(&name).unwrap();
        definition.parents = parents;
        definition.generic_params = generic_params;
        Ok(name)
    }

    fn tr(&mut self, tr: &XenoTrait) -> Result<String, String> {
        if BUILTIN_TRAITS
            .iter()
            .any(|builtin| std::ptr::eq(*builtin, tr))
        {
            return Ok(tr.name.into());
        }
        let name = self.identity("trait", tr.name, tr as *const _ as usize)?;
        if self.graph.traits.contains_key(&name) {
            return Ok(name);
        }
        self.graph.traits.insert(
            name.clone(),
            OwnedTrait {
                name: name.clone(),
                documentation: tr.documentation.map(str::to_string),
                kind: OwnedTraitKind::Semantic,
                parents: Vec::new(),
            },
        );
        let parents = tr
            .parents
            .unwrap_or(&[])
            .iter()
            .map(|parent| self.tr(parent))
            .collect::<Result<_, _>>()?;
        let kind = match tr.kind {
            XenoTraitKind::Semantic => OwnedTraitKind::Semantic,
            XenoTraitKind::Struct => OwnedTraitKind::Struct,
            XenoTraitKind::Sum(member) => OwnedTraitKind::Sum(self.tr(member)?),
            XenoTraitKind::Expression => OwnedTraitKind::Expression,
            XenoTraitKind::Literal => OwnedTraitKind::Literal,
            XenoTraitKind::LiteralType => OwnedTraitKind::LiteralType,
            XenoTraitKind::RegexLiteral => OwnedTraitKind::RegexLiteral,
            XenoTraitKind::Identifier => OwnedTraitKind::Identifier,
            XenoTraitKind::Type => OwnedTraitKind::Type,
            XenoTraitKind::Annotation => OwnedTraitKind::Annotation,
        };
        let definition = self.graph.traits.get_mut(&name).unwrap();
        definition.parents = parents;
        definition.kind = kind;
        Ok(name)
    }
}
