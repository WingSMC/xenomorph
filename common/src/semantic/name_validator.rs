use std::collections::HashSet;

use crate::{
    parser::{Expr, TypeList},
    semantic::AnalyzerListener,
    TokenData, XenoError,
};

/// Reports unknown type identifiers and unknown annotation names.
pub struct NameValidator<'k> {
    pub known_types: HashSet<&'k str>,
    pub known_annotations: HashSet<&'k str>,
}

impl<'k> NameValidator<'k> {
    pub fn new(known_types: HashSet<&'k str>, known_annotations: HashSet<&'k str>) -> Self {
        Self {
            known_types,
            known_annotations,
        }
    }
}

impl<'src> AnalyzerListener<'src> for NameValidator<'_> {
    fn on_before_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoError<'src>>) {
        if let Expr::Identifier(id) = expr {
            if !self.known_types.contains(id.v) {
                errors.push(XenoError {
                    location: (*id).clone(),
                    message: format!("Unknown type '{}'", id.v),
                });
            }
        }
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        _args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
        if !self.known_annotations.contains(name.v) {
            errors.push(XenoError {
                location: (*name).clone(),
                message: format!("Unknown annotation '@{}'", name.v),
            });
        }
    }
}
