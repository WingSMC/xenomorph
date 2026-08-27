use crate::{
    parser::{Expr, SimpleType},
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
    fn on_simple_type(&mut self, ty: &SimpleType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        if let SimpleType::Identifier(id)
        | SimpleType::OptionalIdentifier(id)
        | SimpleType::Array(id)
        | SimpleType::OptionalArray(id) = ty
        {
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
        _args: &[Expr<'src>],
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
