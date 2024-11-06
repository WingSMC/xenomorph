use super::parser_expr::{Declaration, Expr, Literal, NumberType};
use crate::tokens::{format_token_opt, Token, TokenData, TokenVariant};

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
            ast.push(self.parse_type_declaration()?);
        }

        Ok(ast)
    }

    fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
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

    fn prev(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current - 1)
    }

    fn parse_type_declaration(&mut self) -> Result<Declaration<'src>, String> {
        let type_tok = self.next()?;
        if type_tok.0 != TokenVariant::Type {
            return Err(format!("Expected 'type' at {}", type_tok.1));
        }

        let (nv, name) = self.next()?;
        if *nv != TokenVariant::Identifier {
            return Err(format!("Expected identifier at {}", name));
        }

        let (nv, d) = self.next()?;
        if *nv != TokenVariant::Eq {
            return Err(format!("Expected '=' at {:?}", d));
        }

        let t = self.parse_expr_list()?;

        let (nv, d) = self.next()?;
        if *nv != TokenVariant::RCurly {
            return Err(format!("Expected '}}' at {}", d));
        }

        Ok(Declaration::TypeDecl { name, t })
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
            TokenVariant::At => self.parse_annotation(),
            _ => Err(format!(
                "Unexpected token {}",
                format_token_opt(self.prev())
            )),
        }
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
                        TokenVariant::Comma | TokenVariant::RBracket | TokenVariant::RCurly | TokenVariant::RParen
                    ) {
                        current = d;
                        break;
                    }
                }

                None => {
                    dbg!("UE1");
                    return Err("Unexpected end of input".to_string());
                }
            }
        }

        if matches!(current.0, TokenVariant::Comma) {
            self.next()?;
        }

        Ok(list)
    }

    fn parse_list(&mut self) -> Result<Expr<'src>, String> {
        let mut list = Vec::new();

        while self.peek().map(|t| t.0) != Some(TokenVariant::RBracket) {
            list.push(self.parse_expr_list()?);
        }

        let (var, d) = self.next()?;
        if *var != TokenVariant::RBracket {
            return Err(format!("Expected ']' at {}", d));
        }

        Ok(Expr::List(list))
    }

    fn parse_annotation(&mut self) -> Result<Expr<'src>, String> {
        let (var, id) = self.next()?;
        if *var != TokenVariant::Identifier {
            return Err(format!("Expected identifier after '@' at {}", id));
        }

        let (var, d) = self.next()?;
        if *var != TokenVariant::LParen {
            return Err(format!("Expected '(' at {}", d));
        }

        let t = self.parse_expr_list()?;
        let (var, d) = self.next()?;
        if *var != TokenVariant::RParen {
            return Err(format!("Expected ')' at {}", d));
        }

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
