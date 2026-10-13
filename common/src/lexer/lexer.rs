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
}

impl fmt::Display for LexerLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "index: {}", self.src_index)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Lexed {
    pub tokens: XenoTokens,
    pub checkpoints: Vec<(usize, usize)>,
    pub error: Option<XenoDiagnostic>,
    pub scanned_bytes: usize,
}

pub struct Lexer<'src> {
    pub src: &'src str,
    pub it: Peekable<Chars<'src>>,
    pub location: LexerLocation,
    pub tokens: XenoTokens,
    pub checkpoints: Vec<(usize, usize)>,
}

impl<'src> std::iter::Iterator for Lexer<'src> {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        let c = self.it.next();
        if let Some(c) = c {
            self.location.src_index += c.len_utf8();
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
            checkpoints: Vec::new(),
            location: LexerLocation { src_index: 0 },
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

    pub fn single_char_token_next(&mut self) -> TokenData {
        let td = self.token_single_at_lexer();
        self.next();
        td
    }

    pub fn location_to_token(&self, location: &LexerLocation) -> TokenData {
        let end = self.src[location.src_index..]
            .chars()
            .next()
            .map(|c| location.src_index + c.len_utf8())
            .unwrap_or(location.src_index);
        TokenData::new(location.src_index, end)
    }

    /**
    Returns the source consumed since `start`.

    `src_index` is the UTF-8 byte offset of the next unconsumed character, so
    the token occupies the half-open byte range `start.src_index..src_index`.
    */
    pub fn token_from(&self, start: &LexerLocation) -> TokenData {
        TokenData::new(start.src_index, self.location.src_index)
    }

    pub fn token_single_at_lexer(&self) -> TokenData {
        let start = &self.location;
        let end = self.src[start.src_index..]
            .chars()
            .next()
            .map(|c| start.src_index + c.len_utf8())
            .unwrap_or(start.src_index);
        TokenData::new(start.src_index, end)
    }

    pub fn tokenize(src: &'src str) -> Result<XenoTokens, XenoDiagnostic> {
        let mut lexer = Self::new(src);
        lexer.scan_tokens(|_, _| false)?;
        Ok(lexer.tokens)
    }
    fn scan_tokens(
        &mut self,
        mut stop: impl FnMut(usize, usize) -> bool,
    ) -> Result<(), XenoDiagnostic> {
        loop {
            let offset = self.location.src_index;
            self.checkpoints.push((offset, self.tokens.len()));
            if stop(offset, self.tokens.len()) {
                return Ok(());
            }
            let Some(c) = self.peek() else {
                break;
            };
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

        Ok(())
    }

    pub fn lex(source: &'src str) -> Lexed {
        let mut lexer = Self::new(source);
        let error = lexer.scan_tokens(|_, _| false).err();
        Lexed {
            tokens: lexer.tokens,
            checkpoints: lexer.checkpoints,
            error,
            scanned_bytes: source.len(),
        }
    }

    /// Restart only at a neutral scanner checkpoint. A whole string, regex,
    /// comment, or import/path pair is consumed between checkpoints.
    pub fn relex(old_source: &str, old: &Lexed, source: &'src str) -> Lexed {
        if old.error.is_some() {
            return Self::lex(source);
        }

        let mut prefix = old_source
            .bytes()
            .zip(source.bytes())
            .take_while(|(a, b)| a == b)
            .count();

        while !source.is_char_boundary(prefix) {
            prefix -= 1;
        }

        if prefix == old_source.len() && prefix == source.len() {
            let mut result = old.clone();
            result.scanned_bytes = 0;
            return result;
        }

        let mut suffix = old_source[prefix..]
            .bytes()
            .rev()
            .zip(source[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();

        while !source.is_char_boundary(source.len() - suffix)
            || !old_source.is_char_boundary(old_source.len() - suffix)
        {
            suffix -= 1;
        }

        let old_suffix = old_source.len() - suffix;
        let new_suffix = source.len() - suffix;
        let delta = source.len() as i64 - old_source.len() as i64;
        let restart = old
            .checkpoints
            .iter()
            .rfind(|(offset, _)| *offset < prefix)
            .copied()
            .unwrap_or((0, 0));
        let mut lexer = Self::new(source);
        lexer.location.src_index = restart.0;
        lexer.it = source[restart.0..].chars().peekable();
        lexer.tokens.extend_from_slice(&old.tokens[..restart.1]);
        lexer.checkpoints.extend(
            old.checkpoints
                .iter()
                .copied()
                .take_while(|(offset, _)| *offset < restart.0),
        );
        let mut convergence = None;
        let error = lexer
            .scan_tokens(|offset, count| {
                if offset < new_suffix {
                    return false;
                }
                let old_offset = (offset as i64 - delta) as usize;
                if old_offset < old_suffix {
                    return false;
                }
                if let Ok(index) = old
                    .checkpoints
                    .binary_search_by_key(&old_offset, |(offset, _)| *offset)
                {
                    convergence = Some((index, count));
                    return true;
                }
                false
            })
            .err();
        let scanned_bytes = lexer.location.src_index - restart.0;
        if let Some((index, count)) = convergence {
            let old_count = old.checkpoints[index].1;
            lexer
                .tokens
                .extend(old.tokens[old_count..].iter().map(|(kind, range)| {
                    (
                        *kind,
                        crate::TextRange {
                            start: (range.start as i64 + delta) as u32,
                            end: (range.end as i64 + delta) as u32,
                        },
                    )
                }));
            lexer
                .checkpoints
                .extend(old.checkpoints[index + 1..].iter().map(|(offset, tokens)| {
                    (
                        (*offset as i64 + delta) as usize,
                        count + tokens - old_count,
                    )
                }));
        }
        Lexed {
            tokens: lexer.tokens,
            checkpoints: lexer.checkpoints,
            error,
            scanned_bytes,
        }
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

        let w = match token_data.text(self.src) {
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

    fn consume_string(&mut self) -> Result<Token, XenoDiagnostic> {
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

    fn consume_not_or_neq(&mut self) -> Token {
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

    fn consume_minus_or_number(&mut self) -> Token {
        let initial_loc = self.location_snapshot(); // -
        self.next();
        match self.peek() {
            Some('0'..='9') => self.consume_number(Some(initial_loc)),
            _ => (TokenVariant::Minus, self.location_to_token(&initial_loc)),
        }
    }

    fn consume_number(&mut self, minus_loc: Option<LexerLocation>) -> Token {
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

    // fn consume_range_lt_dot_symmdiff(&mut self) -> Token {
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

    fn consume_comment_or_regex(&mut self) -> Result<Option<Token>, XenoDiagnostic> {
        let initial_loc = self.location_snapshot();
        self.next(); // skip first '/'

        match self.peek() {
            Some(&'/') => self.skip_line_comment(),
            Some(&'*') => self.consume_doc_comment(initial_loc),
            _ => self.consume_regex(initial_loc),
        }
    }

    fn skip_line_comment(&mut self) -> Result<Option<Token>, XenoDiagnostic> {
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
    ) -> Result<Option<Token>, XenoDiagnostic> {
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

    fn consume_regex(&mut self, start: LexerLocation) -> Result<Option<Token>, XenoDiagnostic> {
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
