use crate::{
    parser::{AnonymType, Expr, TypeList},
    semantic::AnalyzerListener,
    TokenData, XenoError,
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
}

impl IfChainValidator {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
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
    fn on_before_type(&mut self, _exprs: &AnonymType<'src>, _errors: &mut Vec<XenoError<'src>>) {
        self.stack.push(IfChainState::None);
    }

    fn on_after_type(&mut self, _exprs: &AnonymType<'src>, _errors: &mut Vec<XenoError<'src>>) {
        self.stack.pop();
    }

    fn on_before_expr(&mut self, expr: &Expr<'src>, _errors: &mut Vec<XenoError<'src>>) {
        if !matches!(expr, Expr::Annotation(_, _)) {
            self.set(IfChainState::None);
        }
    }

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        _args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
        match name.v {
            "if" => self.set(IfChainState::AfterIf),
            "elseif" => match self.current() {
                IfChainState::AfterIf => {}
                _ => {
                    errors.push(XenoError {
                        location: (*name).clone(),
                        message: "'@elseif' must follow an '@if' or another '@elseif'.".to_string(),
                    });
                    self.set(IfChainState::None);
                }
            },
            "else" => match self.current() {
                IfChainState::AfterIf => self.set(IfChainState::AfterElse),
                _ => {
                    errors.push(XenoError {
                        location: (*name).clone(),
                        message: "'@else' must follow an '@if' or '@elseif'.".to_string(),
                    });
                    self.set(IfChainState::None);
                }
            },
            _ => self.set(IfChainState::None),
        }
    }
}
