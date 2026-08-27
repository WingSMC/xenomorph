use crate::{
    parser::{Expr, XenoType},
    semantic::AnalyzerListener,
    TokenData, XenoDiagnostic,
};

#[derive(Clone, Copy, PartialEq)]
enum IfChainState {
    None,
    AfterIf,
    AfterElse,
}

/// Validates that @elseif / @else only appear after @if or @elseif.
/// Uses a stack so chain state is scoped per type (expression list).
pub struct IfChainValidator {
    stack: Vec<IfChainState>,
    annotation_depth: usize,
}

impl IfChainValidator {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            annotation_depth: 0,
        }
    }
    fn current(&self) -> IfChainState {
        self.stack.last().copied().unwrap_or(IfChainState::None)
    }
    fn set(&mut self, s: IfChainState) {
        if let Some(top) = self.stack.last_mut() {
            *top = s;
        }
    }
}

// ── Built-in listeners ──────────────────────────────────────────────

impl<'src> AnalyzerListener<'src> for IfChainValidator {
    fn on_before_type(&mut self, _exprs: &XenoType<'src>, _errors: &mut Vec<XenoDiagnostic<'src>>) {
        self.stack.push(IfChainState::None);
    }

    fn on_after_type(&mut self, _exprs: &XenoType<'src>, _errors: &mut Vec<XenoDiagnostic<'src>>) {
        self.stack.pop();
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        _args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        if self.annotation_depth == 0 {
            match name.v {
                "if" => self.set(IfChainState::AfterIf),
                "elseif" => match self.current() {
                    IfChainState::AfterIf => {}
                    _ => {
                        errors.push(XenoDiagnostic {
                            location: (*name).clone(),
                            message: "'@elseif' must follow an '@if' or another '@elseif'."
                                .to_string(),
                            severity: crate::XenoDiagSeverity::Err,
                        });
                        self.set(IfChainState::None);
                    }
                },
                "else" => match self.current() {
                    IfChainState::AfterIf => self.set(IfChainState::AfterElse),
                    _ => {
                        errors.push(XenoDiagnostic {
                            location: (*name).clone(),
                            message: "'@else' must follow an '@if' or '@elseif'.".to_string(),
                            severity: crate::XenoDiagSeverity::Err,
                        });
                        self.set(IfChainState::None);
                    }
                },
                _ => self.set(IfChainState::None),
            }
        }
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData<'src>,
        _args: &[Expr<'src>],
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        lexer::Lexer,
        parser::{Declaration, Parser},
        semantic::AnalyzerListener,
        test_strings::parser as source,
        XenoDiagSeverity,
    };

    use super::IfChainValidator;

    fn validate(src: &str) -> Vec<String> {
        let tokens = Lexer::tokenize(src).expect("source must lex");
        let (ast, parser_diagnostics) = Parser::parse(&tokens);
        assert!(
            !parser_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err),
            "source must parse: {parser_diagnostics:#?}"
        );

        let mut validator = IfChainValidator::new();
        let mut diagnostics = Vec::new();
        for declaration in &ast {
            if let Declaration::Type { ty, .. } = declaration {
                validator.on_before_type(ty, &mut diagnostics);
                for annotation in &ty.1 {
                    validator.on_before_annotation(
                        annotation.ident,
                        &annotation.params,
                        &mut diagnostics,
                    );
                    for parameter in &annotation.params {
                        if let crate::parser::Expr::Annotation(nested) = parameter {
                            validator.on_before_annotation(
                                nested.ident,
                                &nested.params,
                                &mut diagnostics,
                            );
                            validator.on_after_annotation(
                                nested.ident,
                                &nested.params,
                                &mut diagnostics,
                            );
                        }
                    }
                    validator.on_after_annotation(
                        annotation.ident,
                        &annotation.params,
                        &mut diagnostics,
                    );
                }
                validator.on_after_type(ty, &mut diagnostics);
            }
        }
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    #[test]
    fn nested_annotation_arguments_do_not_break_if_else_chains() {
        assert!(validate(&source::if_else_with_nested_annotation()).is_empty());
    }

    #[test]
    fn else_without_if_is_rejected() {
        let source = source::else_without_if();
        let diagnostics = validate(&source);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("must follow"));
    }
}
