use crate::{parser_expr::Expr, tokens::Token};

pub struct Parser<'src> {
    tokens: Vec<Token<'src>>,
    current: usize,
}

impl<'src> Parser<'src> {
    pub fn new(tokens: Vec<Token<'src>>) -> Self {
        Self { tokens, current: 0 }
    }

    fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
    }

    fn next(&mut self) -> Option<&Token<'src>> {
        self.current += 1;
        self.tokens.get(self.current - 1)
    }

    pub fn parse(&mut self) -> Result<Vec<Expr<'src>>, &'static str> {
        let mut ast = Vec::new();

        while self.peek() != Some(&Token::EOF) {
            ast.push(self.parse_type_definition()?);
        }

        Ok(ast)
    }

    pub fn print_location(&self) {
        if self.current == 0 {
            println!("No tokens have been parsed yet");
        }

        let token = &self.tokens[self.current - 1];
        use Token::*;
        match token {
            EOF => println!("EOF"),
            Number(l, _)
            | Identifier(l)
            | Type(l)
            | Set(l)
            | String(l)
            | Regex(l)
            | Not(l)
            | Or(l)
            | And(l)
            | Dot(l)
            | Comma(l)
            | Colon(l)
            | Semicolon(l)
            | Plus(l)
            | Minus(l)
            | Asterix(l)
            | Backslash(l)
            | Dollar(l)
            | At(l)
            | Eq(l)
            | Neq(l)
            | Caret(l)
            | SymmDiff(l)
            | Gt(l)
            | Lt(l)
            | LParen(l)
            | RParen(l)
            | LCurly(l)
            | RCurly(l)
            | LBracket(l)
            | RBracket(l) => println!("{}:{}:{}", l.l, l.c, l.v),
        };
    }
}

impl<'src> Parser<'src> {
    fn parse_identifier(&mut self) -> Result<Expr<'src>, &'static str> {
        if let Some(Token::Identifier(token_data)) = self.next() {
            Ok(Expr::Identifier(token_data.v))
        } else {
            Err("Expected an identifier")
        }
    }

    fn parse_number(&mut self) -> Result<Expr<'src>, &'static str> {
        if let Some(Token::Number(token_data, _)) = self.next() {
            let number = token_data.v.parse::<i64>().map_err(|_| "Invalid number")?;
            Ok(Expr::Number(number))
        } else {
            Err("Expected a number")
        }
    }

    fn parse_list(&mut self) -> Result<Expr<'src>, &'static str> {
        self.next().unwrap();
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            match token {
                Token::RBracket(_) => {
                    self.next();
                    break;
                }
                _ => elements.push(self.parse_expr()?),
            }
        }

        Ok(Expr::List(elements))
    }

    fn parse_struct(&mut self) -> Result<Expr<'src>, &'static str> {
        self.next().unwrap();
        let mut fields = Vec::new();

        while let Some(token) = self.peek() {
            match token {
                Token::RCurly(_) => {
                    self.next(); // consume '}'
                    break;
                }
                Token::Identifier(_) => {
                    let field_name = self.parse_identifier()?;

                    match self.peek() {
                        Some(Token::Colon(_)) => {
                            self.next();
                        }
                        _ => return Err("Expected ':'"),
                    }

                    let field_value = self.parse_expr()?;
                    fields.push((field_name.to_string(), field_value));
                }
                _ => return Err("Expected identifier or '}'"),
            }
        }

        Ok(Expr::Struct(fields))
    }

    fn parse_expr(&mut self) -> Result<Expr<'src>, &'static str> {
        match self.peek() {
            Some(Token::Identifier(_)) => self.parse_identifier(),
            Some(Token::Number(_, _)) => self.parse_number(),
            Some(Token::LBracket(_)) => self.parse_list(),
            Some(Token::LCurly(_)) => self.parse_struct(),
            _ => Err("Unexpected token while parsing expression"),
        }
    }

    fn parse_type_definition(&mut self) -> Result<Expr<'src>, &'static str> {
        match self.peek() {
            Some(Token::Type(_)) => self.next(),
            _ => return Err("Expected 'type'"),
        };

        let type_name = self.parse_identifier()?;

        match self.peek() {
            Some(Token::Eq(_)) => self.next(),
            _ => return Err("Expected '='"),
        };

        let type_value = self.parse_expr()?;

        Ok(Expr::TypeDef {
            name: match type_name {
                Expr::Identifier(name) => name,
                _ => return Err("Invalid type name"),
            },
            value: Box::new(type_value),
        })
    }
}
