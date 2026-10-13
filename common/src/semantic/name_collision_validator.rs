use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::{
    parser::{Annotation, Declaration, Expr, KeyValExpr, Type, XenoType},
    TokenData, XenoDiagSeverity, XenoDiagnostic,
};

/// Reports names that would make type or declaration-local lookup ambiguous.
///
/// Type owners include every module in the registry cache. A registry is
/// populated from one configured entry graph, so unrelated entry graphs in
/// other configurations never participate in the same uniqueness check.
pub struct NameCollisionValidator<'scope> {
    current_module: &'scope str,
    source: &'scope str,
    type_owners: &'scope BTreeMap<String, BTreeSet<String>>,
    static_types: &'scope HashSet<String>,
    local_types: HashSet<String>,
}

impl<'scope> NameCollisionValidator<'scope> {
    pub fn new(
        current_module: &'scope str,
        source: &'scope str,
        type_owners: &'scope BTreeMap<String, BTreeSet<String>>,
        static_types: &'scope HashSet<String>,
    ) -> Self {
        Self {
            current_module,
            source,
            type_owners,
            static_types,
            local_types: HashSet::new(),
        }
    }

    fn error(errors: &mut Vec<XenoDiagnostic>, location: &TokenData, message: String) {
        errors.push(XenoDiagnostic {
            location: *location,
            message,
            severity: XenoDiagSeverity::Err,
        });
    }

    fn validate_members(
        &self,
        members: &[KeyValExpr],
        member_kind: &str,
        errors: &mut Vec<XenoDiagnostic>,
    ) {
        let mut seen = HashSet::new();
        for (name, _, _) in members {
            if !seen.insert(name.text(self.source)) {
                Self::error(
                    errors,
                    name,
                    format!("Duplicate {member_kind} '{}'", name.text(self.source)),
                );
            }
        }
    }

    pub fn validate(&mut self, ast: &[Declaration]) -> Vec<XenoDiagnostic> {
        self.local_types.clear();
        let mut errors = Vec::new();
        for declaration in ast {
            let Declaration::Type {
                name, generics, ty, ..
            } = declaration
            else {
                continue;
            };
            self.validate_type_declaration(name, generics.as_deref(), &mut errors);
            self.validate_xeno_type(ty, &mut errors);
        }
        errors
    }

    fn validate_type_declaration(
        &mut self,
        name: &TokenData,
        generics: Option<&[(TokenData, Option<TokenData>)]>,
        errors: &mut Vec<XenoDiagnostic>,
    ) {
        if !self.local_types.insert(name.text(self.source).to_string()) {
            Self::error(
                errors,
                name,
                format!(
                    "Duplicate type name '{}' in this module",
                    name.text(self.source)
                ),
            );
        } else if self.static_types.contains(name.text(self.source)) {
            Self::error(
                errors,
                name,
                format!(
                    "Type name '{}' conflicts with a built-in or plugin type",
                    name.text(self.source)
                ),
            );
        } else if let Some(other_module) =
            self.type_owners
                .get(name.text(self.source))
                .and_then(|owners| {
                    owners
                        .iter()
                        .find(|owner| owner.as_str() != self.current_module)
                })
        {
            Self::error(
                errors,
                name,
                format!(
                    "Duplicate type name '{}' (also declared in module '{}')",
                    name.text(self.source),
                    other_module
                ),
            );
        }

        let mut generic_names = HashSet::new();
        for (generic, _) in generics.unwrap_or_default() {
            if !generic_names.insert(generic.text(self.source)) {
                Self::error(
                    errors,
                    generic,
                    format!(
                        "Duplicate generic parameter '{}'",
                        generic.text(self.source)
                    ),
                );
            } else if self.static_types.contains(generic.text(self.source))
                || self.type_owners.contains_key(generic.text(self.source))
            {
                Self::error(
                    errors,
                    generic,
                    format!(
                        "Generic parameter '{}' shadows an existing type",
                        generic.text(self.source)
                    ),
                );
            }
        }
    }

    fn validate_xeno_type(&self, (ty, annotations): &XenoType, errors: &mut Vec<XenoDiagnostic>) {
        self.validate_type(ty, errors);
        for annotation in annotations {
            self.validate_annotation(annotation, errors);
        }
    }

    fn validate_type(&self, ty: &Type, errors: &mut Vec<XenoDiagnostic>) {
        match ty {
            Type::Struct(fields) => self.validate_members(fields, "struct field", errors),
            Type::Enum(variants) => self.validate_members(variants, "enum variant", errors),
            Type::Simple(_)
            | Type::Tuple(_)
            | Type::Set(_)
            | Type::Sum(_)
            | Type::Intersection(_) => {}
        }
    }

    fn validate_annotation(&self, annotation: &Annotation, errors: &mut Vec<XenoDiagnostic>) {
        for parameter in &annotation.params {
            match parameter {
                Expr::Regex(_) => {}
                Expr::Annotation(annotation) => self.validate_annotation(annotation, errors),
                Expr::Type(ty) => self.validate_type(ty, errors),
            }
        }
    }
}
