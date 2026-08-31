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
    type_owners: &'scope BTreeMap<String, BTreeSet<String>>,
    static_types: &'scope HashSet<String>,
    local_types: HashSet<String>,
}

impl<'scope> NameCollisionValidator<'scope> {
    pub fn new(
        current_module: &'scope str,
        type_owners: &'scope BTreeMap<String, BTreeSet<String>>,
        static_types: &'scope HashSet<String>,
    ) -> Self {
        Self {
            current_module,
            type_owners,
            static_types,
            local_types: HashSet::new(),
        }
    }

    fn error<'src>(
        errors: &mut Vec<XenoDiagnostic<'src>>,
        location: &TokenData<'src>,
        message: String,
    ) {
        errors.push(XenoDiagnostic {
            location: location.clone(),
            message,
            severity: XenoDiagSeverity::Err,
        });
    }

    fn validate_members<'src>(
        members: &[KeyValExpr<'src>],
        member_kind: &str,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        let mut seen = HashSet::new();
        for (name, _, _) in members {
            if !seen.insert(name.v) {
                Self::error(
                    errors,
                    name,
                    format!("Duplicate {member_kind} '{}'", name.v),
                );
            }
        }
    }

    pub fn validate<'src>(&mut self, ast: &[Declaration<'src>]) -> Vec<XenoDiagnostic<'src>> {
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

    fn validate_type_declaration<'src>(
        &mut self,
        name: &TokenData<'src>,
        generics: Option<&[(&TokenData<'src>, Option<&TokenData<'src>>)]>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if !self.local_types.insert(name.v.to_string()) {
            Self::error(
                errors,
                name,
                format!("Duplicate type name '{}' in this module", name.v),
            );
        } else if self.static_types.contains(name.v) {
            Self::error(
                errors,
                name,
                format!(
                    "Type name '{}' conflicts with a built-in or plugin type",
                    name.v
                ),
            );
        } else if let Some(other_module) = self.type_owners.get(name.v).and_then(|owners| {
            owners
                .iter()
                .find(|owner| owner.as_str() != self.current_module)
        }) {
            Self::error(
                errors,
                name,
                format!(
                    "Duplicate type name '{}' (also declared in module '{}')",
                    name.v, other_module
                ),
            );
        }

        let mut generic_names = HashSet::new();
        for (generic, _) in generics.unwrap_or_default() {
            if !generic_names.insert(generic.v) {
                Self::error(
                    errors,
                    generic,
                    format!("Duplicate generic parameter '{}'", generic.v),
                );
            } else if self.static_types.contains(generic.v)
                || self.type_owners.contains_key(generic.v)
            {
                Self::error(
                    errors,
                    generic,
                    format!("Generic parameter '{}' shadows an existing type", generic.v),
                );
            }
        }
    }

    fn validate_xeno_type<'src>(
        &self,
        (ty, annotations): &XenoType<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.validate_type(ty, errors);
        for annotation in annotations {
            self.validate_annotation(annotation, errors);
        }
    }

    fn validate_type<'src>(&self, ty: &Type<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        match ty {
            Type::Struct(fields) => Self::validate_members(fields, "struct field", errors),
            Type::Enum(variants) => Self::validate_members(variants, "enum variant", errors),
            Type::Simple(_)
            | Type::Tuple(_)
            | Type::Set(_)
            | Type::Sum(_)
            | Type::Intersection(_) => {}
        }
    }

    fn validate_annotation<'src>(
        &self,
        annotation: &Annotation<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        for parameter in &annotation.params {
            match parameter {
                Expr::Regex(_) => {}
                Expr::Annotation(annotation) => self.validate_annotation(annotation, errors),
                Expr::Type(ty) => self.validate_type(ty, errors),
            }
        }
    }
}
