use crate::{
    lexer::{Token, TokenVariant, Tokens},
    parser::{
        AnonymType, BinaryExprType, Declaration, Expr, KeyValExpr, Literal, NumberType, TypeList,
    },
    utils::extract_documentation,
    TokenData, XenoError,
};

#[derive(Clone, Debug)]
pub struct Parser<'src> {
    pub tokens: &'src Tokens<'src>,
    pub current: usize,
}

pub type XenoAst<'src> = Vec<Declaration<'src>>;
pub type XenoParseResult<'src> = (XenoAst<'src>, Vec<XenoError<'src>>);

impl<'src> Parser<'src> {
    fn new(tokens: &'src Tokens<'src>) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn parse(tokens: &'src Tokens<'src>) -> XenoParseResult<'src> {
        Self::new(tokens)._parse()
    }
    fn _parse(mut self) -> XenoParseResult<'src> {
        let mut ast = Vec::new();
        let mut errs = Vec::new();

        while self.is_not_eof() {
            match self.parse_declaration() {
                Err(e) => {
                    errs.extend(e);
                    self.recover_to(TokenVariant::Semicolon);
                }
                Ok(d) => ast.push(d),
            }
        }

        (ast, errs)
    }

    pub fn parse_range(
        _tokens: &'src Tokens<'src>,
        _old_ast: Vec<Declaration<'src>>,
        _range: (usize, usize),
    ) -> XenoParseResult<'src> {
        panic!("Not implemented yet")
    }

    fn recover_to(&mut self, variant: TokenVariant) {
        if let Some(t) = self.peek() {
            if t.0 == variant {
                return;
            }
        }

        while let Ok(t) = self.next() {
            if t.0 == variant {
                break;
            }
        }
    }

    fn is_not_eof(&self) -> bool {
        self.current < self.tokens.len()
    }

    fn next(&mut self) -> Result<&'src Token<'src>, XenoError<'src>> {
        let d = self.tokens.get(self.current);
        match d {
            None => {
                let prev = self.tokens.get(self.current - 1).unwrap();
                Err(XenoError {
                    location: prev.1.clone(),
                    message: "Unexpected end of file.".to_string(),
                })
            }
            Some(t) => {
                self.current += 1;
                Ok(t)
            }
        }
    }
    fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
    }
    fn expect(
        &mut self,
        expected: TokenVariant,
    ) -> Result<&'src TokenData<'src>, Vec<XenoError<'src>>> {
        let (var, d) = self.next().map_err(Parser::map_err_vec)?;
        if *var != expected {
            return Err(vec![XenoError {
                location: d.clone(),
                message: format!("Expected {} at {} instead got {}.", expected, d, var),
            }]);
        }
        Ok(d)
    }

    fn map_err_vec(e: XenoError<'src>) -> Vec<XenoError<'src>> {
        vec![e]
    }

    fn parse_declaration(&mut self) -> Result<Declaration<'src>, Vec<XenoError<'src>>> {
        let docs: Option<&'src str> = self.peek().and_then(|(v, d)| {
            (*v == TokenVariant::Documentation).then_some(extract_documentation(d))
        });

        let (var, d) = self.next().map_err(Parser::map_err_vec)?;
        let dec = match var {
            TokenVariant::Type => self.parse_type_declaration(docs)?,
            TokenVariant::Import => {
                if docs.is_some() {
                    return Err(vec![XenoError {
                        location: d.clone(),
                        message: "Import declarations cannot have documentation comments."
                            .to_string(),
                    }]);
                }
                self.parse_import_declaration(d)?
            }
            _ => {
                return Err(vec![XenoError {
                    location: d.clone(),
                    message: format!("Expected declaration at {}, instead found {}.", d, var),
                }])
            }
        };

        self.expect(TokenVariant::Semicolon)?;

        Ok(dec)
    }
    fn parse_type_declaration(
        &mut self,
        docs: Option<&'src str>,
    ) -> Result<Declaration<'src>, Vec<XenoError<'src>>> {
        let name = self.expect(TokenVariant::Identifier)?;
        self.expect(TokenVariant::Eq)?;
        let t = self.parse_anonym_type()?;
        Ok(Declaration::TypeDecl { name, t, docs })
    }
    fn parse_import_declaration(
        &mut self,
        location: &'src TokenData<'src>,
    ) -> Result<Declaration<'src>, Vec<XenoError<'src>>> {
        let first = self.expect(TokenVariant::Identifier)?;
        let mut path = vec![first.v];

        while self.peek().map(|t| t.0) == Some(TokenVariant::Slash) {
            self.next().map_err(Parser::map_err_vec)?; // consume '/'
            let segment = self.expect(TokenVariant::Identifier)?;
            path.push(segment.v);
        }

        Ok(Declaration::Import { path, location })
    }
    fn parse_anonym_type(&mut self) -> Result<AnonymType<'src>, Vec<XenoError<'src>>> {
        let mut list: Vec<Expr<'src>> = Vec::new();
        let mut errs = Vec::new();

        loop {
            match self.parse_expr(&mut list) {
                Err(e) => {
                    errs.extend(e);
                    // self.recover_to(TokenVariant::Comma);
                    return Err(errs);
                }
                _ => {}
            };

            let terminator_variant = match self.peek() {
                Some(d) => d.0,
                None => return Err(errs),
            };

            if matches!(
                terminator_variant,
                TokenVariant::Comma
                    | TokenVariant::RBracket
                    | TokenVariant::RCurly
                    | TokenVariant::RParen
                    | TokenVariant::Semicolon
            ) {
                if terminator_variant == TokenVariant::Comma {
                    self.next().map_err(Parser::map_err_vec)?;
                }
                break Ok(list);
            }
        }
    }

    /**
    Parses an actual atomic type (the thing you would separate with '|' or '&' in TypeScript)
     */
    fn parse_expr(&mut self, list: &mut AnonymType<'src>) -> Result<(), Vec<XenoError<'src>>> {
        let (variant, loc) = self.next().map_err(Parser::map_err_vec)?;

        let res = match variant {
            TokenVariant::Identifier => Expr::Identifier(loc), // just type reuse
            TokenVariant::Dollar => Expr::FieldAccess(self.expect(TokenVariant::Identifier)?),
            TokenVariant::Number => self.parse_number(loc).map_err(Parser::map_err_vec)?,
            TokenVariant::True | TokenVariant::False => {
                Expr::Literal(Literal::Boolean(*variant == TokenVariant::True, loc))
            }
            TokenVariant::String => {
                Expr::Literal(Literal::String(loc.v[1..loc.v.len() - 1].to_string(), loc))
            }
            TokenVariant::Regex => Expr::Regex(loc),

            TokenVariant::LBracket => {
                let res = self.parse_list()?;
                self.expect(TokenVariant::RBracket)?;
                Expr::List(res)
            }
            TokenVariant::Set => {
                self.expect(TokenVariant::LBracket)?;
                let res = self.parse_list()?;
                self.expect(TokenVariant::RBracket)?;
                Expr::Set(res)
            }
            TokenVariant::LCurly => Expr::Struct(self.parse_struct()?),
            TokenVariant::Enum => {
                self.expect(TokenVariant::LCurly)?;
                let res = self.parse_struct()?;
                self.expect(TokenVariant::RCurly)?;
                Expr::Enum(res)
            }

            TokenVariant::At => self.parse_annotation()?,
            TokenVariant::Not => {
                self.parse_expr(list)?;
                Expr::Not(Box::new(list.pop().unwrap()))
            }

            TokenVariant::Or => {
                if list.len() == 0 {
                    return Ok(());
                }
                self.parse_binary(BinaryExprType::Or, loc, list)?
            }
            TokenVariant::And => self.parse_binary(BinaryExprType::Union, loc, list)?,
            TokenVariant::Asterix => self.parse_binary(BinaryExprType::Intersection, loc, list)?,
            TokenVariant::Caret => self.parse_binary(BinaryExprType::Xor, loc, list)?,
            TokenVariant::Backslash => self.parse_binary(BinaryExprType::Difference, loc, list)?,
            TokenVariant::Range => self.parse_binary(BinaryExprType::Range, loc, list)?,
            TokenVariant::Plus => self.parse_binary(BinaryExprType::Add, loc, list)?,
            TokenVariant::Minus => self.parse_binary(BinaryExprType::Remove, loc, list)?,
            TokenVariant::SymmDiff => {
                self.parse_binary(BinaryExprType::SymmetricDifference, loc, list)?
            }

            TokenVariant::Type
            | TokenVariant::Import
            | TokenVariant::Validator
            | TokenVariant::Slash
            | TokenVariant::Dot
            | TokenVariant::Comma
            | TokenVariant::Colon
            | TokenVariant::Semicolon
            | TokenVariant::Eq
            | TokenVariant::Neq
            | TokenVariant::Gt
            | TokenVariant::Lt
            | TokenVariant::LParen
            | TokenVariant::RParen
            | TokenVariant::RCurly
            | TokenVariant::RBracket
            | TokenVariant::Documentation => {
                return Err(vec![XenoError {
                    location: loc.clone(),
                    message: format!("Unexpected token {}", variant),
                }])
            }
        };

        list.push(res);
        Ok(())
    }

    fn parse_binary(
        &mut self,
        t: BinaryExprType,
        loc: &'src TokenData<'src>,
        list: &mut AnonymType<'src>,
    ) -> Result<Expr<'src>, Vec<XenoError<'src>>> {
        let prev = list.pop();
        if let None = prev {
            return Err(vec![XenoError {
                location: loc.clone(),
                message: "Expected expression before binary operator.".to_string(),
            }]);
        }

        self.parse_expr(list)?;
        return Ok(Expr::BinaryExpr(
            t,
            Box::new((prev.unwrap(), list.pop().unwrap())),
        ));
    }
    fn parse_list(&mut self) -> Result<TypeList<'src>, Vec<XenoError<'src>>> {
        let mut list = Vec::new();

        while !matches!(
            self.peek().map(|t| t.0),
            |Some(TokenVariant::RBracket)| Some(TokenVariant::RParen)
                | Some(TokenVariant::Semicolon)
        ) {
            list.push(self.parse_anonym_type()?);
        }

        Ok(list)
    }
    fn parse_struct(&mut self) -> Result<Vec<KeyValExpr<'src>>, Vec<XenoError<'src>>> {
        let mut fields = Vec::new();
        while self.peek().map(|t| t.0) != Some(TokenVariant::RCurly) {
            let d = self.expect(TokenVariant::Identifier)?;

            self.expect(TokenVariant::Colon)?;

            let expr = self.parse_anonym_type()?;
            fields.push((d, expr));
        }

        self.expect(TokenVariant::RCurly)?;

        Ok(fields)
    }
    fn parse_annotation(&mut self) -> Result<Expr<'src>, Vec<XenoError<'src>>> {
        let id = self.expect(TokenVariant::Identifier)?;

        let has_args = self.peek().map(|t| t.0) == Some(TokenVariant::LParen);

        if !has_args {
            return Ok(Expr::Annotation(id, Vec::new()));
        }

        self.expect(TokenVariant::LParen).unwrap();
        let t = self.parse_list()?;
        self.expect(TokenVariant::RParen)?;

        Ok(Expr::Annotation(id, t))
    }
    fn parse_number(&mut self, d: &'src TokenData<'src>) -> Result<Expr<'src>, XenoError<'src>> {
        let has_dot = d.v.contains('.');

        Ok(Expr::Literal(if has_dot {
            let num = d.v.parse::<f64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Float(n, d)),
                Err(e) => {
                    return Err(XenoError {
                        location: d.clone(),
                        message: format!("Error parsing number: {}", e),
                    })
                }
            }
        } else {
            let num = d.v.parse::<i64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Int(n, d)),
                Err(e) => {
                    return Err(XenoError {
                        location: d.clone(),
                        message: format!("Error parsing number: {}", e),
                    })
                }
            }
        }))
    }
}
