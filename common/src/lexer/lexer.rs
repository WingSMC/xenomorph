use std::{fmt, iter::Peekable, str::Chars};

use crate::lexer::{Token, TokenVariant, XenoTokens};
use crate::{TokenData, XenoDiagnostic};

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
    pub it: Peekable<Chars<'src>>,
    pub location: LexerLocation,
    pub tokens: XenoTokens<'src>,
}

impl<'src> std::iter::Iterator for Lexer<'src> {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        let c = self.it.next();
        if let Some(c) = c {
            self.location.src_index += c.len_utf8();
            self.location.column += 1;
            if c == '\n' {
                self.location.line += 1;
                self.location.column = 0;
            }
        }
        c
    }
}

impl<'src> Lexer<'src> {
    fn new(src: &'src str) -> Self {
        Lexer {
            src,
            it: src.chars().peekable(),
            tokens: Vec::new(),
            location: LexerLocation {
                src_index: 0,
                line: 0,
                column: 0,
            },
        }
    }

    pub fn peek(&mut self) -> Option<&char> {
        self.it.peek()
    }

    pub fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.next();
            } else {
                break;
            }
        }
    }

    pub fn location_snapshot(&self) -> LexerLocation {
        self.location.clone()
    }

    pub fn single_char_token_next(&mut self) -> TokenData<'src> {
        let td = self.token_single_at_lexer();
        self.next();
        td
    }

    pub fn location_to_token(&self, location: &LexerLocation) -> TokenData<'src> {
        let end = self.src[location.src_index..]
            .chars()
            .next()
            .map(|c| location.src_index + c.len_utf8())
            .unwrap_or(location.src_index);
        TokenData {
            v: &self.src[location.src_index..end],
            l: location.line,
            c: location.column,
        }
    }

    /**
    Returns the source consumed since `start`.

    `src_index` is the UTF-8 byte offset of the next unconsumed character, so
    the token occupies the half-open byte range `start.src_index..src_index`.
    */
    pub fn token_from(&self, start: &LexerLocation) -> TokenData<'src> {
        TokenData {
            v: &self.src[start.src_index..self.location.src_index],
            l: start.line,
            c: start.column,
        }
    }

    pub fn token_single_at_lexer(&self) -> TokenData<'src> {
        let start = &self.location;
        let end = self.src[start.src_index..]
            .chars()
            .next()
            .map(|c| start.src_index + c.len_utf8())
            .unwrap_or(start.src_index);
        TokenData {
            v: &self.src[start.src_index..end],
            l: start.line,
            c: start.column,
        }
    }

    pub fn tokenize(src: &'src str) -> Result<XenoTokens<'src>, XenoDiagnostic<'src>> {
        Self::new(src)._tokenize()
    }
    fn _tokenize(mut self) -> Result<XenoTokens<'src>, XenoDiagnostic<'src>> {
        while let Some(c) = self.peek() {
            let token = match c {
                ':' => (TokenVariant::Colon, self.single_char_token_next()),
                '?' => (TokenVariant::Question, self.single_char_token_next()),
                ',' => (TokenVariant::Comma, self.single_char_token_next()),
                ';' => (TokenVariant::Semicolon, self.single_char_token_next()),
                '@' => (TokenVariant::At, self.single_char_token_next()),
                // '+' => (TokenVariant::Plus, self.single_char_token_next()),
                '0'..='9' => self.consume_number(None),
                '-' => self.consume_minus_or_number(),
                '"' => self.consume_string()?,
                '!' => self.consume_not_or_neq(),
                // '$' => (TokenVariant::Dollar, self.single_char_token_next()),
                '|' => (TokenVariant::Or, self.single_char_token_next()),
                '&' => (TokenVariant::And, self.single_char_token_next()),
                '{' => (TokenVariant::LCurly, self.single_char_token_next()),
                '}' => (TokenVariant::RCurly, self.single_char_token_next()),
                '[' => (TokenVariant::LBracket, self.single_char_token_next()),
                ']' => (TokenVariant::RBracket, self.single_char_token_next()),
                '>' => (TokenVariant::Gt, self.single_char_token_next()),
                '<' => (TokenVariant::Lt, self.single_char_token_next()),
                '(' => (TokenVariant::LParen, self.single_char_token_next()),
                ')' => (TokenVariant::RParen, self.single_char_token_next()),
                // '.' => (TokenVariant::Dot, self.single_char_token_next()),
                // '*' => (TokenVariant::Asterix, self.single_char_token_next()),
                // '^' => (TokenVariant::Caret, self.single_char_token_next()),
                '=' => (TokenVariant::Eq, self.single_char_token_next()),
                // '\\' => (TokenVariant::Backslash, self.single_char_token_next()),
                'a'..='z' | 'A'..='Z' | '_' => {
                    self.consume_word();
                    continue;
                }
                '/' => match self.consume_comment_or_regex()? {
                    Some(token) => token,
                    None => continue,
                },
                _ if c.is_whitespace() => {
                    self.skip_whitespace();
                    continue;
                }
                _ => {
                    return Err(XenoDiagnostic {
                        message: NOT_RECOGNIZED.to_string(),
                        location: self.token_single_at_lexer(),
                        severity: crate::XenoDiagSeverity::Err,
                    });
                }
            };

            self.tokens.push(token);
        }

        Ok(self.tokens)
    }

    fn consume_word(&mut self) {
        let initial_loc = self.location_snapshot();

        while let Some(&c) = self.peek() {
            match c {
                'a'..='z' | 'A'..='Z' | '_' | '0'..='9' => {
                    self.next();
                }
                _ => break,
            }
        }

        let token_data = self.token_from(&initial_loc);

        let w = match token_data.v {
            "type" => (TokenVariant::Type, token_data),
            // "validator" => (TokenVariant::Validator, token_data),
            "set" => (TokenVariant::Set, token_data),
            "enum" => (TokenVariant::Enum, token_data),
            "as" => (TokenVariant::As, token_data),
            "true" => (TokenVariant::True, token_data),
            "false" => (TokenVariant::False, token_data),
            "import" => {
                self.tokens.push((TokenVariant::Import, token_data));
                self.skip_whitespace();
                self.consume_path();
                return;
            }
            _ => (TokenVariant::Identifier, token_data),
        };

        self.tokens.push(w);
    }
    fn consume_path(&mut self) {
        let initial_loc = self.location_snapshot();
        while let Some('a'..='z' | 'A'..='Z' | '_' | '0'..='9' | '/') = self.peek() {
            self.next();
        }
        if self.location.src_index > initial_loc.src_index {
            self.tokens
                .push((TokenVariant::Path, self.token_from(&initial_loc)));
        }
    }

    fn consume_string(&mut self) -> Result<Token<'src>, XenoDiagnostic<'src>> {
        let initial_loc = self.location_snapshot();
        self.next();

        while let Some(c) = self.next() {
            match c {
                '"' => return Ok((TokenVariant::String, self.token_from(&initial_loc))),
                _ => continue,
            }
        }

        Err(XenoDiagnostic {
            message: STRING_TERMINATION_ERROR.to_string(),
            location: self.token_from(&initial_loc),
            severity: crate::XenoDiagSeverity::Err,
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
            _ => (TokenVariant::Not, self.location_to_token(&initial_loc)),
        }
    }

    fn consume_minus_or_number(&mut self) -> Token<'src> {
        let initial_loc = self.location_snapshot(); // -
        self.next();
        match self.peek() {
            Some('0'..='9') => self.consume_number(Some(initial_loc)),
            _ => (TokenVariant::Minus, self.location_to_token(&initial_loc)),
        }
    }

    fn consume_number(&mut self, minus_loc: Option<LexerLocation>) -> Token<'src> {
        let initial_loc = minus_loc.unwrap_or(self.location_snapshot());
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

        (TokenVariant::Number, self.token_from(&initial_loc))
    }

    // fn consume_range_lt_dot_symmdiff(&mut self) -> Token<'src> {
    //     let initial_loc = self.location_snapshot();
    //     let c = self.next().unwrap();
    //     let variant = match c {
    //         '.' => match self.peek() {
    //             Some('.' | '<') => {
    //                 self.next();
    //                 TokenVariant::Range
    //             }
    //             _ => TokenVariant::Dot,
    //         },
    //         '<' => match self.peek() {
    //             Some('.') => {
    //                 self.next();
    //                 match self.peek() {
    //                     Some('<') => {
    //                         self.next();
    //                         TokenVariant::Range
    //                     }
    //                     _ => TokenVariant::Range,
    //                 }
    //             }
    //             Some('>') => {
    //                 self.next();
    //                 TokenVariant::SymmDiff
    //             }
    //             _ => TokenVariant::Lt,
    //         },
    //         _ => unreachable!(),
    //     };

    //     (
    //         variant,
    //         self.token_from_but_not_including_lexer(&initial_loc),self.
    //     )
    // }

    fn consume_comment_or_regex(&mut self) -> Result<Option<Token<'src>>, XenoDiagnostic<'src>> {
        let initial_loc = self.location_snapshot();
        self.next(); // skip first '/'

        match self.peek() {
            Some(&'/') => self.skip_line_comment(),
            Some(&'*') => self.consume_doc_comment(initial_loc),
            _ => self.consume_regex(initial_loc),
        }
    }

    fn skip_line_comment(&mut self) -> Result<Option<Token<'src>>, XenoDiagnostic<'src>> {
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
    ) -> Result<Option<Token<'src>>, XenoDiagnostic<'src>> {
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
                        return Ok(Some((TokenVariant::Documentation, self.token_from(&start))));
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

        Err(XenoDiagnostic {
            message: COMMENT_NOT_TERMINATED.to_string(),
            location: self.token_from(&start),
            severity: crate::XenoDiagSeverity::Err,
        })
    }

    fn consume_regex(
        &mut self,
        start: LexerLocation,
    ) -> Result<Option<Token<'src>>, XenoDiagnostic<'src>> {
        let mut has_escape = false;
        while let Some(c) = self.next() {
            match c {
                '\n' => break,
                '\\' => has_escape = !has_escape,
                '/' if !has_escape => {
                    return Ok(Some((TokenVariant::Regex, self.token_from(&start))))
                }
                _ => has_escape = false,
            }
        }

        Err(XenoDiagnostic {
            message: MALFORMED_REGEX.to_string(),
            location: self.token_from(&start),
            severity: crate::XenoDiagSeverity::Err,
        })
    }
}
