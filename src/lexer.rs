use crate::tokens::{Token, TokenData};
use std::{iter::Peekable, str::Chars};

static NOT_RECOGNIZED: &str = "Token not recognized";
static STRING_TERMINATION_ERROR: &str = "String not terminated";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexerLocation {
    pub src_index: usize,
    pub line: usize,
    pub column: usize,
}

pub struct Lexer<'src> {
    pub src: &'src str,
    it: Peekable<Chars<'src>>,
    location: LexerLocation,
}

impl<'src> TokenData<'src> {
    pub fn new_short(lexer: &Lexer<'src>) -> Self {
        let start = lexer.location;
        TokenData {
            v: &lexer.src[start.src_index..=start.src_index],
            src_index: start.src_index,
            l: start.line,
            c: start.column,
        }
    }
    pub fn new(start: &LexerLocation, lexer: &Lexer<'src>) -> Self {
        TokenData {
            v: &lexer.src[start.src_index..lexer.location.src_index],
            src_index: start.src_index,
            l: start.line,
            c: start.column,
        }
    }
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Lexer {
            src,
            it: src.chars().peekable(),
            location: LexerLocation {
                src_index: 0,
                line: 1,
                column: 1,
            },
        }
    }

    fn next(&mut self) -> Option<char> {
        let c = self.it.next();
        if let Some(c) = c {
            self.location.src_index += 1;
            self.location.column += 1;
            if c == '\n' {
                self.location.line += 1;
                self.location.column = 1;
            }
        }
        c
    }

    fn peek(&mut self) -> Option<&char> {
        self.it.peek()
    }

    fn location_snapshot(&self) -> LexerLocation {
        self.location.clone()
    }

    fn slice_from(&self, start: usize) -> &'src str {
        &self.src[start..self.location.src_index]
    }

    fn token_data_next(&mut self) -> TokenData<'src> {
        let td = TokenData::new_short(&self);
        self.next();
        td
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token<'src>>, (&'static str, LexerLocation)> {
        let mut tokens: Vec<Token<'src>> = vec![];
        while let Some(c) = self.peek() {
            tokens.push(match c {
                ' ' | '\n' | '\t' | '\r' => {
                    self.next();
                    continue;
                }
                'a'..='z' | 'A'..='Z' | '_' => self.word(),
                '"' => self.consume_string().map(Token::String)?,
                ':' => Token::Colon(self.token_data_next()),
                '(' => Token::LParen(self.token_data_next()),
                ')' => Token::RParen(self.token_data_next()),
                '{' => Token::LCurly(self.token_data_next()),
                '}' => Token::RCurly(self.token_data_next()),
                '[' => Token::LBracket(self.token_data_next()),
                ']' => Token::RBracket(self.token_data_next()),
                '<' => Token::Lt(self.token_data_next()),
                '>' => Token::Gt(self.token_data_next()),
                '.' => Token::Dot(self.token_data_next()),
                ',' => Token::Comma(self.token_data_next()),
                ';' => Token::Semicolon(self.token_data_next()),
                '+' => Token::Plus(self.token_data_next()),
                '-' => Token::Minus(self.token_data_next()),
                '|' => Token::Or(self.token_data_next()),
                '&' => Token::And(self.token_data_next()),
                '*' => Token::Asterix(self.token_data_next()),
                '@' => Token::At(self.token_data_next()),
                '$' => Token::Dollar(self.token_data_next()),
                '^' => Token::Caret(self.token_data_next()),
                '=' => Token::Eq(self.token_data_next()),
                '\\' => Token::Backslash(self.token_data_next()),
                '/' => match self.consume_comment_or_slash() {
                    Some(token) => token,
                    None => continue,
                },
                // TODO regex & number & symmdiff & neq
                _ => return Err((NOT_RECOGNIZED, self.location)),
                
            });
        }

        tokens.push(Token::EOF);
        Ok(tokens)
    }

    fn word(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        let mut word = String::new();
        word.push(self.next().unwrap());

        while let Some(&c) = self.peek() {
            match c {
                'a'..='z' | 'A'..='Z' | '_' | '0'..='9' => {
                    self.next();
                    word.push(c);
                }
                _ => break,
            }
        }

        let token_data = TokenData {
            v: &self.slice_from(initial_loc.src_index),
            src_index: initial_loc.src_index,
            l: initial_loc.line,
            c: initial_loc.column,
        };

        match word.as_str() {
            "type" => Token::Type(token_data),
            "set" => Token::Set(token_data),
            _ => Token::Identifier(token_data),
        }
    }

    fn consume_comment_or_slash(&mut self) -> Option<Token<'src>> {
        let initial_loc = self.location_snapshot();
        self.next();

        if self.peek() != Some(&'/') {
            return Some(Token::Slash(TokenData::new(&initial_loc, &self)));
        }
        self.next();

        while let Some(&c) = self.peek() {
            if c == '\n' {
                break;
            }

            self.next();
        }

        return None;
    }

    fn consume_string(&mut self) -> Result<TokenData<'src>, (&'static str, LexerLocation)> {
        let initial_loc = self.location_snapshot();
        self.next();

        while let Some(c) = self.next() {
            match c {
                '"' => return Ok(TokenData::new(&initial_loc, &self)),
                _ => continue,
            }
        }

        Err((STRING_TERMINATION_ERROR, initial_loc))
    }
}
