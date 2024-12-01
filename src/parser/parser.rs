use super::parser_expr::{
    AnonymType, BinaryExprType, Declaration, Expr, KeyValExpr, Literal, NumberType, TypeList,
};
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
        }?;

        self.expect(TokenVariant::Semicolon)?;

        Ok(dec)
    }
    fn parse_type_declaration(&mut self) -> Result<Declaration<'src>, String> {
        let name = self.expect(TokenVariant::Identifier)?;
        self.expect(TokenVariant::Eq)?;
        let t = self.parse_anonym_type()?;
        Ok(Declaration::TypeDecl { name, t })
    }
    fn parse_anonym_type(&mut self) -> Result<AnonymType<'src>, String> {
        let mut list: Vec<Expr<'src>> = Vec::new();

        loop {
            self.parse_expr(&mut list)?;

            let variant = match self.peek() {
                Some(d) => d.0,
                None => return Ok(list),
            };

            if matches!(
                variant,
                TokenVariant::Comma
                    | TokenVariant::RBracket
                    | TokenVariant::RCurly
                    | TokenVariant::RParen
                    | TokenVariant::Semicolon
            ) {
                if variant == TokenVariant::Comma {
                    self.next()?;
                }
                break Ok(list);
            }
        }
    }

    fn parse_expr(&mut self, list: &mut AnonymType<'src>) -> Result<(), String> {
        let (variant, loc) = self.next()?;

        let res = match variant {
            TokenVariant::Identifier => Expr::Identifier(loc),
            TokenVariant::Dollar => Expr::FieldAccess(self.expect(TokenVariant::Identifier)?),
            TokenVariant::Number => self.parse_number(loc)?,
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

            _ => return Err(format!("Unexpected expression token {}", loc)),
        };

        list.push(res);
        Ok(())
    }

    fn parse_binary(
        &mut self,
        t: BinaryExprType,
        loc: &TokenData<'src>,
        list: &mut AnonymType<'src>,
    ) -> Result<Expr<'src>, String> {
        let prev = list.pop();
        if let None = prev {
            return Err(format!(
                "Expected expression before binary operator at {}",
                loc
            ));
        }

        self.parse_expr(list)?;
        return Ok(Expr::BinaryExpr(
            t,
            Box::new((prev.unwrap(), list.pop().unwrap())),
        ));
    }
    fn parse_list(&mut self) -> Result<TypeList<'src>, String> {
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
    fn parse_struct(&mut self) -> Result<Vec<KeyValExpr<'src>>, String> {
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
    fn parse_annotation(&mut self) -> Result<Expr<'src>, String> {
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
