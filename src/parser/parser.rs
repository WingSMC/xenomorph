use super::parser_expr::Expr;
use crate::tokens::Token;

pub struct Parser<'src> {
    pub tokens: Vec<Token<'src>>,
    pub current: usize,
}

impl<'src> Parser<'src> {
    pub fn new(tokens: Vec<Token<'src>>) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current)
    }

    pub fn next(&mut self) -> Option<&Token<'src>> {
        let d = self.tokens.get(self.current);
        self.current += 1;
        d
    }

    pub fn prev(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.current - 1)
    }

    pub fn parse(&mut self) -> Result<Vec<Expr<'src>>, String> {
        let mut ast = Vec::new();

        while self.peek() != None {
            ast.push(self.parse_type_definition()?);
        }

        Ok(ast)
    }
}
