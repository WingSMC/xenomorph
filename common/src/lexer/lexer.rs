use std::{fmt, iter::Peekable, str::Chars};

use crate::lexer::{Token, TokenVariant, Tokens};
use crate::{ParseError, TokenData};

static NOT_RECOGNIZED: &str = "Token not recognized";
static MALFORMED_REGEX: &str = "Malformed regex";
static STRING_TERMINATION_ERROR: &str = "String not terminated";
static COMMENT_NOT_TERMINATED: &str = "Comment not terminated";

#[derive(Debug, Clone)]
pub struct LexerLocation {
    pub src_index: usize,
    pub line: u32,
    pub column: u32,
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
    pub location: LexerLocation,
}

impl<'src> Lexer<'src> {
    fn new(src: &'src str) -> Self {
        Lexer {
            src,
            it: src.chars().peekable(),
            location: LexerLocation {
                src_index: 0,
                line: 0,
                column: 0,
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
                self.location.column = 0;
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
        let td = self.token_single_at_lexer();
        self.next();
        td
    }

    fn token_single_at_lexer(&self) -> TokenData<'src> {
        let start = &self.location;
        TokenData {
            v: &self.src[start.src_index..=start.src_index],
            l: start.line,
            c: start.column,
        }
    }

    fn token_from(&self, start: &LexerLocation) -> TokenData<'src> {
        TokenData {
            v: &self.src[start.src_index..=self.location.src_index],
            l: start.line,
            c: start.column,
        }
    }
    /**
    If you step over the last character of a token with next()
    so the current lexer location is one after the token, use this.
    */
    pub fn token_from_but_not_including_lexer(&self, start: &LexerLocation) -> TokenData<'src> {
        TokenData {
            v: &self.src[start.src_index..self.location.src_index],
            l: start.line,
            c: start.column,
        }
    }

    pub fn tokenize(src: &'src str) -> Result<Tokens<'src>, ParseError<'src>> {
        Self::new(src)._tokenize()
    }
    fn _tokenize(mut self) -> Result<Tokens<'src>, ParseError<'src>> {
        let mut tokens: Tokens<'src> = vec![];
        while let Some(c) = self.peek() {
            let token = match c {
                ' ' | '\n' | '\t' | '\r' => {
                    self.next();
                    continue;
                }
                '/' => match self.consume_comment_or_regex()? {
                    None => continue,
                    Some(t) => t,
                },
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
                '>' => (TokenVariant::Gt, self.single_char_token_next()),
                ';' => (TokenVariant::Semicolon, self.single_char_token_next()),
                '+' => (TokenVariant::Plus, self.single_char_token_next()),
                '-' => (TokenVariant::Minus, self.single_char_token_next()),
                '*' => (TokenVariant::Asterix, self.single_char_token_next()),
                '^' => (TokenVariant::Caret, self.single_char_token_next()),
                '=' => (TokenVariant::Eq, self.single_char_token_next()),
                '0'..='9' => self.consume_number(),
                '"' => self.consume_string()?,
                '.' | '<' => self.consume_range_lt_dot_symmdiff(),
                '!' => self.consume_not_or_neq(),
                '\\' => (TokenVariant::Backslash, self.single_char_token_next()),
                _ => {
                    return Err(ParseError {
                        message: NOT_RECOGNIZED.to_string(),
                        location: self.token_single_at_lexer(),
                    });
                }
            };

            tokens.push(token);
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

    fn consume_string(&mut self) -> Result<Token<'src>, ParseError<'src>> {
        let initial_loc = self.location_snapshot();
        self.next();

        while let Some(c) = self.next() {
            match c {
                '"' => {
                    return Ok((
                        TokenVariant::String,
                        self.token_from_but_not_including_lexer(&initial_loc),
                    ))
                }
                _ => continue,
            }
        }

        Err(ParseError {
            message: STRING_TERMINATION_ERROR.to_string(),
            location: self.token_from(&initial_loc),
        })
    }

    fn consume_not_or_neq(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        self.next();

        match self.peek() {
            Some('=') => {
                self.next();
                (TokenVariant::Neq, self.token_from(&initial_loc))
            }
            _ => (
                TokenVariant::Not,
                self.token_from_but_not_including_lexer(&initial_loc),
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
            self.token_from_but_not_including_lexer(&initial_loc),
        )
    }
    fn consume_range_lt_dot_symmdiff(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot();
        let c = self.next().unwrap();
        let variant = match c {
            '.' => match self.peek() {
                Some('.' | '<') => {
                    self.next();
                    TokenVariant::Range
                }
                _ => TokenVariant::Dot,
            },
            '<' => match self.peek() {
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
            _ => unreachable!(),
        };

        (
            variant,
            self.token_from_but_not_including_lexer(&initial_loc),
        )
    }

    fn consume_comment_or_regex(&mut self) -> Result<Option<Token<'src>>, ParseError<'src>> {
        let initial_loc = self.location_snapshot();
        self.next(); // skip first '/'

        match self.peek() {
            Some(&'/') => self.skip_line_comment(),
            Some(&'*') => self.consume_doc_comment(initial_loc),
            _ => self.consume_regex(initial_loc),
        }
    }

    fn skip_line_comment(&mut self) -> Result<Option<Token<'src>>, ParseError<'src>> {
        self.next(); // skip second '/'
        while let Some(&c) = self.peek() {
            self.next(); // skip til after comment
            if c == '\n' {
                break;
            }
        }

        Ok(None)
    }

    fn consume_doc_comment(
        &mut self,
        start: LexerLocation,
    ) -> Result<Option<Token<'src>>, ParseError<'src>> {
        self.next(); // skip '*'

        if let Some('*') = self.next() {
            // '/**'
            if let Some('/') = self.peek() {
                // '/**/
                self.next();
                return Ok(None); // just an empty multiline comment, skip
            }

            // doc comment, consume until '*/'
            while let Some(c) = self.next() {
                if c == '*' {
                    if let Some('/') = self.peek() {
                        self.next();
                        return Ok(Some((
                            TokenVariant::Documentation,
                            self.token_from_but_not_including_lexer(&start),
                        )));
                    }
                }
            }
        }

        // regular multiline comment, consume until '*/'
        while let Some(c) = self.next() {
            if c == '*' {
                if let Some('/') = self.peek() {
                    self.next();
                    return Ok(None);
                }
            }
        }

        return Err(ParseError {
            message: COMMENT_NOT_TERMINATED.to_string(),
            location: self.token_from_but_not_including_lexer(&start),
        });
    }

    fn consume_regex(
        &mut self,
        start: LexerLocation,
    ) -> Result<Option<Token<'src>>, ParseError<'src>> {
        let mut has_escape = false;
        while let Some(c) = self.next() {
            match c {
                '\n' => break,
                '\\' => has_escape = !has_escape,
                '/' if !has_escape => {
                    return Ok(Some((
                        TokenVariant::Regex,
                        self.token_from_but_not_including_lexer(&start),
                    )))
                }
                _ => has_escape = false,
            }
        }

        return Err(ParseError {
            message: MALFORMED_REGEX.to_string(),
            location: self.token_from_but_not_including_lexer(&start),
        });
    }
}
