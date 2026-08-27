use crate::{
    lexer::{Token, TokenVariant, XenoTokens},
    parser::Declaration,
    TokenData, XenoDiagSeverity, XenoDiagnostic,
};

#[derive(Clone, Debug)]
pub struct Parser<'src> {
    pub tokens: &'src XenoTokens<'src>,
    pub current: usize,
    pub diagnostics: Vec<XenoDiagnostic<'src>>,
}

pub type XenoAst<'src> = Vec<Declaration<'src>>;
pub type XenoParseResult<'src> = (XenoAst<'src>, Vec<XenoDiagnostic<'src>>);

// --- Basic parser utilities / entry ---
impl<'src> Parser<'src> {
    pub fn new(tokens: &'src XenoTokens<'src>) -> Self {
        Self {
            tokens,
            current: 0,
            diagnostics: Vec::new(),
        }
    }
    pub fn parse(tokens: &'src XenoTokens<'src>) -> XenoParseResult<'src> {
        Self::new(tokens)._parse()
    }
    fn _parse(mut self) -> XenoParseResult<'src> {
        let mut ast = Vec::new();

        while self.is_not_eof() {
            Declaration::parse(&mut self)
                .map_or_else(|| self.recover_to(TokenVariant::Semicolon), |d| ast.push(d))
        }

        (ast, self.diagnostics)
    }

    pub fn recover_to(&mut self, variant: TokenVariant) {
        self.diagnostics.push(XenoDiagnostic {
            message: format!("Recovering to {:?} at {}.", variant, self.current),
            severity: XenoDiagSeverity::Info,
            location: self
                .tokens
                .get(self.current)
                .map(|t| t.1.clone())
                .unwrap_or_default(),
        });

        if let Some(t) = self.peek() {
            if t.0 == variant {
                self.step_forward();
                return;
            }
        }

        while let Some(t) = self.next() {
            if t.0 == variant {
                break;
            }
        }
    }

    fn is_not_eof(&self) -> bool {
        self.current < self.tokens.len()
    }

    pub fn need_next(&mut self) -> Option<&'src Token<'src>> {
        let ind = self.current;
        self.current += 1;
        let tok_opt = self.tokens.get(ind);
        if let None = tok_opt {
            self.diagnostics.push(XenoDiagnostic {
                severity: XenoDiagSeverity::Err,
                location: self
                    .tokens
                    .get(ind.saturating_sub(1))
                    .map(|t| t.1.clone())
                    .unwrap_or_default(),
                message: "Unexpected end of file.".to_string(),
            })
        };
        tok_opt
    }

    pub fn next(&mut self) -> Option<&'src Token<'src>> {
        let ind = self.current;
        self.current += 1;
        self.tokens.get(ind)
    }

    pub fn peek(&self) -> Option<&'src Token<'src>> {
        self.tokens.get(self.current)
    }

    pub fn peek_is(&self, expected: TokenVariant) -> bool {
        matches!(self.peek(), Some((variant, _)) if *variant == expected)
    }

    pub fn step_forward(&mut self) {
        self.current += 1;
    }

    pub fn skip_if(&mut self, variant: TokenVariant) -> bool {
        let r = self.peek_is(variant);
        if r {
            self.step_forward();
        }
        r
    }

    #[must_use]
    pub fn expect(&mut self, expected: TokenVariant) -> Option<&'src TokenData<'src>> {
        let (var, d) = self.need_next()?;
        if *var != expected {
            self.diagnostics.push(XenoDiagnostic {
                location: d.clone(),
                message: format!("Expected {:?} at {} instead got {:?}.", expected, d, var),
                severity: XenoDiagSeverity::Err,
            });
            return None;
        }

        Some(d)
    }

    /// Expects the current token without consuming it on a mismatch. This is
    /// useful for local recovery because a delimiter may be the unexpected
    /// token and must remain available as a synchronization point.
    #[must_use]
    pub fn expect_at_current(&mut self, expected: TokenVariant) -> Option<&'src TokenData<'src>> {
        let Some((actual, data)) = self.peek() else {
            self.need_next();
            return None;
        };

        if *actual != expected {
            self.diagnostics.push(XenoDiagnostic {
                location: data.clone(),
                message: format!(
                    "Expected {:?} at {} instead got {:?}.",
                    expected, data, actual
                ),
                severity: XenoDiagSeverity::Err,
            });
            return None;
        }

        self.step_forward();
        Some(data)
    }

    /// Advances to the first synchronization token without consuming it.
    pub fn recover_to_any(&mut self, variants: &[TokenVariant]) -> Option<TokenVariant> {
        while let Some((variant, _)) = self.peek() {
            if variants.contains(variant) {
                return Some(*variant);
            }
            self.step_forward();
        }
        None
    }

    pub fn parse_list<T>(
        &mut self,
        opener: TokenVariant,
        sep: TokenVariant,
        closer: Option<TokenVariant>,
        member_parser: fn(&mut Parser<'src>) -> Option<T>,
    ) -> Option<Vec<T>> {
        self.expect(opener)?;

        let mut types = Vec::new();
        while !closer.is_some_and(|closer| self.peek_is(closer)) {
            let ty = member_parser(self)?;
            types.push(ty);

            if !self.peek_is(sep) {
                break;
            }
            self.step_forward();

            if closer.is_some_and(|closer| self.peek_is(closer)) {
                break;
            }
        }

        if let Some(closer_var) = closer {
            self.expect(closer_var)?;
        }

        Some(types)
    }
}
