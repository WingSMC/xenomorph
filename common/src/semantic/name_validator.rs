use crate::{
    parser::{Expr, TypeList},
    semantic::{AnalyzerListener, ScopeInfo},
    TokenData, XenoDiagnostic,
};

/// Reports unknown type identifiers and unknown annotation names.
pub struct NameValidator {
    scope: ScopeInfo,
}

impl NameValidator {
    pub fn new(scope: &ScopeInfo) -> Self {
        Self {
            scope: scope.clone(),
        }
    }
}

impl<'src> AnalyzerListener<'src> for NameValidator {
    fn on_before_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        if let Expr::Identifier(id) = expr {
            if !self.scope.has_type(id.v) {
                errors.push(XenoDiagnostic {
                    location: (*id).clone(),
                    message: format!("Unknown type '{}'", id.v),
                    severity: crate::XenoDiagSeverity::Err,
                });
            }
        }
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        _args: &TypeList<'src>,
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
