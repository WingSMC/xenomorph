use super::parser_expr::{Declaration, Expr, Literal, NumberType};
use crate::tokens::{Token, TokenData, TokenVariant};

pub struct Parser<'src> {
    pub tokens: &'src Vec<Token<'src>>,
    pub current: usize,
}

impl<'src> Parser<'src> {
    pub fn new(tokens: &'src Vec<Token<'src>>) -> Self {
        Self { tokens, current: 0 }
    }
    pub fn parse(&mut self) -> Result<Vec<Declaration<'src>>, String> {
        let mut ast = Vec::new();

        while self.is_not_eof() {
            ast.push(self.parse_declaration()?);
        }

        Ok(ast)
    }

    fn is_not_eof(&self) -> bool {
        self.current < self.tokens.len() - 1
    }
    fn next(&mut self) -> Result<&'src Token<'src>, String> {
        let d = self.tokens.get(self.current);
        match d {
            None => Err("Unexpected end of input".to_string()),
            Some(t) => {
                self.current += 1;
                Ok(t)
            }
        }
    }
    fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
    }
    fn expect(&mut self, expected: TokenVariant) -> Result<&'src TokenData<'src>, String> {
        let (var, d) = self.next()?;
        if *var != expected {
            return Err(format!("Expected {} at {}", expected, d));
        }
        Ok(d)
    }

    fn parse_declaration(&mut self) -> Result<Declaration<'src>, String> {
        let (var, d) = self.next()?;
        let dec = match var {
            TokenVariant::Type => self.parse_type_declaration(),
            _ => Err(format!("Expected 'type' at {}, instead found {}.", d, var)),
        };

        if let Err(e) = dec {
            return Err(e);
        }

        self.expect(TokenVariant::Semicolon)?;

        dec
    }
    fn parse_type_declaration(&mut self) -> Result<Declaration<'src>, String> {
        let name = self.expect(TokenVariant::Identifier)?;
        self.expect(TokenVariant::Eq)?;
        let t = self.parse_expr_list()?;
        Ok(Declaration::TypeDecl { name, t })
    }
    fn parse_expr_list(&mut self) -> Result<Vec<Expr<'src>>, String> {
        let mut list = Vec::new();

        let current;
        loop {
            list.push(self.parse_expr()?);

            let curr = self.peek();
            match curr {
                Some(d) => {
                    if matches!(
                        d.0,
                        TokenVariant::Comma
                            | TokenVariant::RBracket
                            | TokenVariant::RCurly
                            | TokenVariant::RParen
                            | TokenVariant::Semicolon
                    ) {
                        current = d;
                        break;
                    }
                }
                None => return Ok(list),
            }
        }

        if matches!(current.0, TokenVariant::Comma) {
            self.next()?;
        }

        Ok(list)
    }
    fn parse_expr(&mut self) -> Result<Expr<'src>, String> {
        let (first_var, first) = self.next()?;

        match first_var {
            TokenVariant::Identifier => Ok(Expr::Identifier(first)),
            TokenVariant::String => Ok(Expr::Literal(Literal::String(
                first.v[1..first.v.len() - 1].to_string(),
                first,
            ))),
            TokenVariant::Number => self.parse_number(first),
            TokenVariant::LBracket => self.parse_list(),
            TokenVariant::LCurly => self.parse_struct(),
            TokenVariant::At => self.parse_annotation(),
            _ => Err(format!("Unexpected expression token {}", first)),
        }
    }

    fn parse_list(&mut self) -> Result<Expr<'src>, String> {
        let mut list = Vec::new();

        while self.peek().map(|t| t.0) != Some(TokenVariant::RBracket) {
            list.push(self.parse_expr_list()?);
        }

        self.expect(TokenVariant::RBracket)?;

        Ok(Expr::List(list))
    }
    fn parse_struct(&mut self) -> Result<Expr<'src>, String> {
        let mut fields = Vec::new();
        while self.peek().map(|t| t.0) != Some(TokenVariant::RCurly) {
            let d = self.expect(TokenVariant::Identifier)?;

            self.expect(TokenVariant::Colon)?;

            let expr = self.parse_expr_list()?;
            fields.push((d, expr));
        }

        self.expect(TokenVariant::RCurly)?;

        Ok(Expr::Struct(fields))
    }

    fn parse_annotation(&mut self) -> Result<Expr<'src>, String> {
        let id = self.expect(TokenVariant::Identifier)?;

        self.expect(TokenVariant::LParen)?;
        let t = self.parse_expr_list()?;
        self.expect(TokenVariant::RParen)?;

        Ok(Expr::Annotation(id, t))
    }
    fn parse_number(&mut self, d: &'src TokenData<'src>) -> Result<Expr<'src>, String> {
        let has_dot = d.v.contains('.');

        Ok(Expr::Literal(if has_dot {
            let num = d.v.parse::<f64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Float(n, d)),
                Err(e) => return Err(format!("Error parsing number: {}, at {}", e, d)),
            }
        } else {
            let num = d.v.parse::<i64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Int(n, d)),
                Err(e) => return Err(format!("Error parsing number: {}, at {}", e, d)),
            }
        }))
    }
}
