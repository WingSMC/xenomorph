use crate::{
    lexer::{Token, TokenVariant, XenoTokens},
    parser::{Declaration, DeclarationMethods},
    TokenData, XenoDiagSeverity, XenoDiagnostic,
};

#[derive(Clone, Debug)]
pub struct Parser<'src> {
    pub tokens: &'src XenoTokens,
    pub source: &'src str,
    pub current: usize,
    pub diagnostics: Vec<XenoDiagnostic>,
}

pub type XenoAst = Vec<Declaration>;
pub type XenoParseResult = (XenoAst, Vec<XenoDiagnostic>);

impl<'src> std::iter::Iterator for Parser<'src> {
    type Item = &'src Token;

    fn next(&mut self) -> Option<&'src Token> {
        let ind = self.current;
        self.current += 1;
        self.tokens.get(ind)
    }
}

// --- Basic parser utilities / entry ---
impl<'src> Parser<'src> {
    pub fn new(tokens: &'src XenoTokens, source: &'src str) -> Self {
        Self {
            tokens,
            source,
            current: 0,
            diagnostics: Vec::new(),
        }
    }
    pub fn parse(tokens: &'src XenoTokens, source: &'src str) -> XenoParseResult {
        Self::new(tokens, source)._parse()
    }
    fn _parse(mut self) -> XenoParseResult {
        let mut ast = Vec::new();

        while self.is_not_eof() {
            Declaration::parse(&mut self)
                .map_or_else(|| self.recover_to(TokenVariant::Semicolon), |d| ast.push(d))
        }

        (ast, self.diagnostics)
    }

    pub fn recover_to(&mut self, variant: TokenVariant) {
        self.diagnostics.push(XenoDiagnostic {
            tags: vec![],
            message: format!("Recovering to {:?} at {}.", variant, self.current),
            severity: XenoDiagSeverity::Info,
            location: self
                .tokens
                .get(self.current)
                .map(|t| t.1)
                .unwrap_or_default(),
        });

        if let Some(t) = self.peek() {
            if t.0 == variant {
                self.step_forward();
                return;
            }
        }

        for t in self {
            if t.0 == variant {
                break;
            }
        }
    }

    fn is_not_eof(&self) -> bool {
        self.current < self.tokens.len()
    }

    pub fn need_next(&mut self) -> Option<&'src Token> {
        let ind = self.current;
        self.current += 1;
        let tok_opt = self.tokens.get(ind);
        if tok_opt.is_none() {
            self.diagnostics.push(XenoDiagnostic {
                tags: vec![],
                severity: XenoDiagSeverity::Err,
                location: self
                    .tokens
                    .get(ind.saturating_sub(1))
                    .map(|t| t.1)
                    .unwrap_or_default(),
                message: "Unexpected end of file.".to_string(),
            })
        };
        tok_opt
    }

    pub fn peek(&self) -> Option<&'src Token> {
        self.tokens.get(self.current)
    }

    pub fn peek_is(&self, expected: TokenVariant) -> bool {
        matches!(self.peek(), Some((variant, _)) if *variant == expected)
    }

    pub fn previous(&self) -> Option<TokenData> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map(|(_, data)| *data)
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
    pub fn expect(&mut self, expected: TokenVariant) -> Option<TokenData> {
        let (var, d) = self.need_next()?;
        if *var != expected {
            self.diagnostics.push(XenoDiagnostic {
                tags: vec![],
                location: *d,
                message: format!("Expected {:?} at {} instead got {:?}.", expected, d, var),
                severity: XenoDiagSeverity::Err,
            });
            return None;
        }

        Some(*d)
    }

    /// Expects the current token without consuming it on a mismatch. This is
    /// useful for local recovery because a delimiter may be the unexpected
    /// token and must remain available as a synchronization point.
    #[must_use]
    pub fn expect_at_current(&mut self, expected: TokenVariant) -> Option<TokenData> {
        let Some((actual, data)) = self.peek() else {
            self.need_next();
            return None;
        };

        if *actual != expected {
            self.diagnostics.push(XenoDiagnostic {
                tags: vec![],
                location: *data,
                message: format!(
                    "Expected {:?} at {} instead got {:?}.",
                    expected, data, actual
                ),
                severity: XenoDiagSeverity::Err,
            });
            return None;
        }

        self.step_forward();
        Some(*data)
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

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub ast: XenoAst,
    pub diagnostics: Vec<XenoDiagnostic>,
    pub spans: Vec<(usize, usize)>,
    pub reused_declarations: usize,
    pub reused_indices: Vec<usize>,
}
impl<'src> Parser<'src> {
    pub fn parse_incremental(
        tokens: &'src XenoTokens,
        source: &'src str,
        old: Option<(&XenoTokens, &str, &Parsed)>,
    ) -> Parsed {
        let old = old.filter(|(_, _, p)| p.diagnostics.is_empty());
        let signature = |tokens: &[Token], source: &str| {
            crate::utils::calculate_hash(
                &tokens
                    .iter()
                    .take(3)
                    .map(|(kind, r)| (*kind as u8, r.text(source)))
                    .collect::<Vec<_>>(),
            )
        };
        let mut candidates: std::collections::HashMap<u64, Vec<usize>> =
            std::collections::HashMap::new();
        if let Some((tokens, source, parsed)) = old {
            for (index, (a, b)) in parsed.spans.iter().enumerate() {
                candidates
                    .entry(signature(&tokens[*a..*b], source))
                    .or_default()
                    .push(index);
            }
        }
        let mut parser = Self::new(tokens, source);
        let mut parsed = Parsed::default();
        while parser.current < tokens.len() {
            let start = parser.current;
            let reusable = old.and_then(|(old_tokens, old_source, old_parsed)| {
                candidates
                    .get(&signature(&tokens[start..], source))?
                    .iter()
                    .find_map(|&index| {
                        let (a, b) = &old_parsed.spans[index];
                        let count = b - a;
                        let new = tokens.get(start..start + count)?;
                        let previous = &old_tokens[*a..*b];
                        let delta = new.first()?.1.start as i64 - previous.first()?.1.start as i64;
                        if !new.iter().zip(previous).all(|((k, r), (ok, or))| {
                            k == ok
                                && r.start as i64 == or.start as i64 + delta
                                && r.end as i64 == or.end as i64 + delta
                        }) {
                            return None;
                        }
                        let from = new.first()?.1.start as usize;
                        let to = new.last()?.1.end as usize;
                        let old_from = previous.first()?.1.start as usize;
                        let old_to = previous.last()?.1.end as usize;
                        if source[from..to] != old_source[old_from..old_to] {
                            return None;
                        }
                        Some((old_parsed.ast[index].clone(), delta, count))
                    })
            });
            if let Some((mut declaration, delta, count)) = reusable {
                declaration.relocate(delta);
                parsed.ast.push(declaration);
                parser.current += count;
                parsed.reused_indices.push(parsed.ast.len() - 1);
                parsed.reused_declarations += 1;
                parsed.spans.push((start, parser.current));
            } else if let Some(declaration) = Declaration::parse(&mut parser) {
                parsed.ast.push(declaration);
                parsed.spans.push((start, parser.current));
            } else {
                parser.recover_to(TokenVariant::Semicolon);
            }
        }
        parsed.diagnostics = parser.diagnostics;
        parsed
    }
}
