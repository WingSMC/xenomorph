use crate::lexer::tokens::{Token, TokenData, TokenVariant};
use std::{fmt, iter::Peekable, str::Chars};

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

impl fmt::Display for LexerLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "index: {}, line: {}, column: {}",
            self.src_index, self.line, self.column
        )
    }
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
                '@' => (TokenVariant::At, self.single_char_token_next()),
                ':' => (TokenVariant::Colon, self.single_char_token_next()),
                '$' => (TokenVariant::Dollar, self.single_char_token_next()),
                '|' => (TokenVariant::Or, self.single_char_token_next()),
                '&' => (TokenVariant::And, self.single_char_token_next()),
                '(' => (TokenVariant::LParen, self.single_char_token_next()),
                ')' => (TokenVariant::RParen, self.single_char_token_next()),
                ',' => (TokenVariant::Comma, self.single_char_token_next()),
                '{' => (TokenVariant::LCurly, self.single_char_token_next()),
                '}' => (TokenVariant::RCurly, self.single_char_token_next()),
                '[' => (TokenVariant::LBracket, self.single_char_token_next()),
                ']' => (TokenVariant::RBracket, self.single_char_token_next()),
                '0'..='9' => self.consume_number(),
                '"' => self.consume_string()?,
                '.' | '<' => self.consume_range_lt_dot_symmdiff(),
                '>' => (TokenVariant::Gt, self.single_char_token_next()),
                ';' => (TokenVariant::Semicolon, self.single_char_token_next()),
                '+' => (TokenVariant::Plus, self.single_char_token_next()),
                '-' => (TokenVariant::Minus, self.single_char_token_next()),
                '*' => (TokenVariant::Asterix, self.single_char_token_next()),
                '^' => (TokenVariant::Caret, self.single_char_token_next()),
                '=' => (TokenVariant::Eq, self.single_char_token_next()),
                '!' => self.consume_not_or_neq(),
                '\\' => (TokenVariant::Backslash, self.single_char_token_next()),
                '/' => match self.consume_comment_or_regex()? {
                    Some(t) => t,
                    None => continue,
                },
                _ => return Err((NOT_RECOGNIZED, self.location)),
            });
        }

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
            "type" => (TokenVariant::Type, token_data),
            "set" => (TokenVariant::Set, token_data),
            "enum" => (TokenVariant::Enum, token_data),
            "true" => (TokenVariant::True, token_data),
            "false" => (TokenVariant::False, token_data),
            _ => (TokenVariant::Identifier, token_data),
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
                return Ok(Some((
                    TokenVariant::Regex,
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
                    return Ok((
                        TokenVariant::String,
                        TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self),
                    ))
                }
                _ => continue,
            }
        }

        Err((STRING_TERMINATION_ERROR, initial_loc))
    }

    fn consume_not_or_neq(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        self.next();

        match self.peek() {
            Some('=') => {
                self.next();
                (
                    TokenVariant::Neq,
                    TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self),
                )
            }
            _ => (
                TokenVariant::Not,
                TokenData::at_loc_in_lexer(&initial_loc, &self),
            ),
        }
    }

    fn consume_number(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        let mut has_decimal_point = false;

        while let Some(&c) = self.peek() {
            match c {
                '0'..='9' => {
                    self.next().unwrap();
                }
                '.' if !has_decimal_point => {
                    // Peek forward one more for range
                    if matches!(self.it.clone().nth(1), Some('0'..='9')) {
                        has_decimal_point = true;
                        self.next().unwrap();
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        (
            TokenVariant::Number,
            TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self),
        )
    }
    fn consume_range_lt_dot_symmdiff(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        let variant = match self.next() {
            Some('.') => match self.peek() {
                Some('.' | '<') => {
                    self.next();
                    TokenVariant::Range
                }
                _ => TokenVariant::Dot,
            },
            _ => match self.peek() {
                Some('.') => {
                    self.next();
                    match self.peek() {
                        Some('<') => {
                            self.next();
                            TokenVariant::Range
                        }
                        _ => TokenVariant::Range,
                    }
                }
                Some('>') => {
                    self.next();
                    TokenVariant::SymmDiff
                }
                _ => TokenVariant::Lt,
            },
        };

        (
            variant,
            TokenData::from_loc_to_but_not_including_lexer(&initial_loc, &self),
        )
    }
}
