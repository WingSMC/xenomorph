use crate::tokens::{NumberType, Token, TokenData};
use std::{iter::Peekable, str::Chars};

static NOT_RECOGNIZED: &str = "Token not recognized";
static MALFORMED_REGEX: &str = "Malformed regex";
static STRING_TERMINATION_ERROR: &str = "String not terminated";

type LexerError = (&'static str, LexerLocation);

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
    pub fn one_at_lexer(lexer: &Lexer<'src>) -> Self {
        let start = lexer.location;
        TokenData {
            v: &lexer.src[start.src_index..=start.src_index],
            l: start.line,
            c: start.column,
        }
    }
    pub fn at_loc_in_lexer(start: &LexerLocation, lexer: &Lexer<'src>) -> Self {
        TokenData {
            v: &lexer.src[start.src_index..=start.src_index],
            l: start.line,
            c: start.column,
        }
    }
    pub fn from_loc_to_but_not_including_lexer(start: &LexerLocation, lexer: &Lexer<'src>) -> Self {
        TokenData {
            v: &lexer.src[start.src_index..lexer.location.src_index],
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

    fn single_char_token_next(&mut self) -> TokenData<'src> {
        let td = TokenData::one_at_lexer(&self);
        self.next();
        td
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token<'src>>, LexerError> {
        let mut tokens: Vec<Token<'src>> = vec![];
        while let Some(c) = self.peek() {
            tokens.push(match c {
                ' ' | '\n' | '\t' | '\r' => {
                    self.next();
                    continue;
                }
                'a'..='z' | 'A'..='Z' | '_' => self.consume_word(),
                '@' => Token::At(self.single_char_token_next()),
                ':' => Token::Colon(self.single_char_token_next()),
                '$' => Token::Dollar(self.single_char_token_next()),
                '|' => Token::Or(self.single_char_token_next()),
                '&' => Token::And(self.single_char_token_next()),
                '0'..='9' => self.consume_number(),
                '"' => self.consume_string()?,
                '.' => Token::Dot(self.single_char_token_next()),
                '(' => Token::LParen(self.single_char_token_next()),
                ')' => Token::RParen(self.single_char_token_next()),
                ',' => Token::Comma(self.single_char_token_next()),
                '{' => Token::LCurly(self.single_char_token_next()),
                '}' => Token::RCurly(self.single_char_token_next()),
                '[' => Token::LBracket(self.single_char_token_next()),
                ']' => Token::RBracket(self.single_char_token_next()),
                '<' => self.consume_lt_or_symdiff(),
                '>' => Token::Gt(self.single_char_token_next()),
                ';' => Token::Semicolon(self.single_char_token_next()),
                '+' => Token::Plus(self.single_char_token_next()),
                '-' => Token::Minus(self.single_char_token_next()),
                '*' => Token::Asterix(self.single_char_token_next()),
                '^' => Token::Caret(self.single_char_token_next()),
                '=' => Token::Eq(self.single_char_token_next()),
                '!' => self.consume_not_or_neq(),
                '\\' => Token::Backslash(self.single_char_token_next()),
                '/' => match self.consume_comment_or_regex()? {
                    Some(t) => t,
                    None => continue,
                },
                _ => return Err((NOT_RECOGNIZED, self.location)),
            });
        }

        tokens.push(Token::EOF);
        Ok(tokens)
    }

    fn consume_word(&mut self) -> Token<'src> {
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
            l: initial_loc.line,
            c: initial_loc.column,
        };

        match word.as_str() {
            "type" => Token::Type(token_data),
            "set" => Token::Set(token_data),
            _ => Token::Identifier(token_data),
        }
    }

    fn consume_comment_or_regex(&mut self) -> Result<Option<Token<'src>>, LexerError> {
        let initial_loc = self.location_snapshot();
        self.next();

        if self.peek() == Some(&'/') {
            self.next();
            while let Some(&c) = self.peek() {
                self.next();
                if c == '\n' {
                    break;
                }
            }

            return Ok(None);
        }

        while let Some(c) = self.next() {
            if c == '/' {
                return Ok(Some(Token::Regex(
                    TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self),
                )));
            }
        }

        Err((MALFORMED_REGEX, initial_loc))
    }

    fn consume_string(&mut self) -> Result<Token<'src>, LexerError> {
        let initial_loc = self.location_snapshot();
        self.next();

        while let Some(c) = self.next() {
            match c {
                '"' => {
                    return Ok(Token::String(TokenData::from_loc_to_but_not_including_lexer(
                        &initial_loc,
                        &self,
                    )))
                }
                _ => continue,
            }
        }

        Err((STRING_TERMINATION_ERROR, initial_loc))
    }

    fn consume_lt_or_symdiff(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        self.next();

        match self.peek() {
            Some('>') => {
                self.next();
                Token::SymmDiff(TokenData::from_loc_to_but_not_including_lexer(
                    &initial_loc,
                    &self,
                ))
            }
            _ => Token::Lt(TokenData::at_loc_in_lexer(&initial_loc, &self)),
        }
    }

    fn consume_not_or_neq(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        self.next();

        match self.peek() {
            Some('=') => {
                self.next();
                Token::Neq(TokenData::from_loc_to_but_not_including_lexer(
                    &initial_loc,
                    &self,
                ))
            }
            _ => Token::Not(TokenData::at_loc_in_lexer(&initial_loc, &self)),
        }
    }

    fn consume_number(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        let mut number_str = String::new();
        let mut is_float = false;

        while let Some(&c) = self.peek() {
            match c {
                '0'..='9' => {
                    number_str.push(self.next().unwrap());
                }
                '.' => {
                    is_float = true;
                    number_str.push(self.next().unwrap());
                }
                _ => break,
            }
        }

        let token_data = TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self);

        if is_float {
            Token::Number(token_data, NumberType::Float(64))
        } else {
            Token::Number(token_data, NumberType::Int(false, 64))
        }
    }
}
