use super::parser_expr::{
    AnonymType, BinaryExprType, Declaration, Expr, KeyValExpr, Literal, NumberType, TypeList,
};
use crate::lexer::{
    lexer::Lexer,
    tokens::{Token, TokenData, TokenVariant},
};
use std::fmt;

#[derive(Debug, Clone)]
pub struct ParseError<'src> {
    pub message: String,
    pub location: Option<TokenData<'src>>,
}

impl<'src> ParseError<'src> {
    pub fn new(message: String) -> ParseError<'src> {
        Self {
            message,
            location: None,
        }
    }
    pub fn new_with_location(message: String, loc: &'src TokenData<'src>) -> ParseError<'src> {
        Self {
            message,
            location: Some(loc.clone()),
        }
    }
    pub fn new_with_move_loc(message: String, loc: TokenData<'src>) -> ParseError<'src> {
        Self {
            message,
            location: Some(loc),
        }
    }
    pub fn new_with_token(message: String, token: &'src Token<'src>) -> ParseError<'src> {
        Self {
            message,
            location: Some(token.1.clone()),
        }
    }
}

impl<'src> fmt::Display for ParseError<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub struct Parser<'src> {
    pub tokens: Box<Vec<Token<'src>>>,
    pub current: usize,
}

pub fn parse<'src>(
    src: &'src str,
) -> (
    Vec<Declaration<'src>>,
    Vec<ParseError<'src>>,
    Box<Vec<Token<'src>>>,
) {
    let tokens = Box::new(match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(e) => {
            return (vec![], vec![e], Box::new(vec![]));
        }
    });

    let mut parser = Parser { tokens, current: 0 };
    let mut ast = Vec::new();
    let mut errors = Vec::new();

    while parser.is_not_eof() {
        match parser.parse_declaration() {
            Ok(decl) => ast.push(decl),
            Err(err) => {
                errors.push(err);
                parser.recover_to_declaration_boundary();
            }
        }
    }

    let Parser { tokens, current: _ } = parser;

    (ast, errors, tokens)
}

impl<'src> Parser<'src> {
    fn parse_declaration(&mut self) -> Result<Declaration<'src>, ParseError<'src>> {
        let dec = match self.next()? {
            (TokenVariant::Type, _) => self.parse_type_declaration(),
            token => Err(ParseError::new_with_token(
                format!("Expected 'type' at {}, instead found {}.", token.1, token.0),
                token,
            )),
        };

        if let Ok(_) = dec {
            self.expect(TokenVariant::Semicolon)?;
        }

        dec
    }

    fn is_not_eof(&self) -> bool {
        self.current < self.tokens.len() - 1
    }
    fn next(&mut self) -> Result<&'src Token<'src>, ParseError<'src>> {
        let d = self.tokens.get(self.current);
        match d {
            None => Err(ParseError::new(format!("Unexpected end of input"))),
            Some(t) => {
                self.current += 1;
                // TODO: This is unsafe, but we know that the tokens are valid and will not be dropped as long as the AST is alive.
                Ok(unsafe { std::mem::transmute::<&Token<'src>, &'src Token<'src>>(t) })
            }
        }
    }
    fn peek(&'src self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
    }
    fn expect(
        &mut self,
        expected: TokenVariant,
    ) -> Result<&'src TokenData<'src>, ParseError<'src>> {
        let (var, d) = self.next()?;
        if *var != expected {
            Err(ParseError::new(format!("Expected {} at {}", expected, d)))
        } else {
            Ok(d)
        }
    }
    fn parse_type_declaration(&mut self) -> Result<Declaration<'src>, ParseError<'src>> {
        let name = self.expect(TokenVariant::Identifier)?;
        self.expect(TokenVariant::Eq)?;
        let t = self.parse_anonym_type()?;
        Ok(Declaration::TypeDecl { name, t })
    }
    fn parse_anonym_type(&mut self) -> Result<AnonymType<'src>, ParseError<'src>> {
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

    fn parse_expr(&mut self, list: &mut AnonymType<'src>) -> Result<(), ParseError<'src>> {
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

            _ => {
                return Err(ParseError::new_with_location(
                    format!("Unexpected expression token {}", loc),
                    loc,
                ))
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
    ) -> Result<Expr<'src>, ParseError<'src>> {
        let prev = list.pop();
        if let None = prev {
            return Err(ParseError::new_with_location(
                format!("Expected expression before binary operator at {}", loc),
                loc,
            ));
        }

        self.parse_expr(list)?;
        return Ok(Expr::BinaryExpr(
            t,
            Box::new((prev.unwrap(), list.pop().unwrap())),
        ));
    }
    fn parse_list(&mut self) -> Result<TypeList<'src>, ParseError<'src>> {
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
    fn parse_struct(&mut self) -> Result<Vec<KeyValExpr<'src>>, ParseError<'src>> {
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
    fn parse_annotation(&mut self) -> Result<Expr<'src>, ParseError<'src>> {
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

    fn err_parse_number<T: std::fmt::Display>(
        &self,
        d: &'src TokenData<'src>,
        e: T,
    ) -> ParseError<'src> {
        ParseError::new_with_location(format!("Error parsing number: {}, at {}", e, d.v), d)
    }
    fn parse_number(&mut self, d: &'src TokenData<'src>) -> Result<Expr<'src>, ParseError<'src>> {
        let has_dot = d.v.contains('.');

        Ok(Expr::Literal(if has_dot {
            let num = d.v.parse::<f64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Float(n, d)),
                Err(e) => return Err(self.err_parse_number(d, e)),
            }
        } else {
            let num = d.v.parse::<i64>();
            match num {
                Ok(n) => Literal::Number(NumberType::Int(n, d)),
                Err(e) => return Err(self.err_parse_number(d, e)),
            }
        }))
    }

    fn recover_to_declaration_boundary(&mut self) {
        while self.is_not_eof() {
            let token = match self.peek() {
                Some(t) => t,
                None => break,
            };

            match token.0 {
                TokenVariant::Type => break,
                TokenVariant::Semicolon => {
                    let _ = self.next();
                    break;
                }
                _ => {
                    let _ = self.next();
                }
            };
        }
    }
}
