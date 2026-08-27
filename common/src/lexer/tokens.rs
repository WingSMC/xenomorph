use crate::TokenData;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenVariant {
    Identifier,
    Type,
    Import,
    Validator,
    Set,
    Enum,
    True,
    False,

    Number,
    String,
    Regex,
    Not,
    Or,
    And,
    Dot,
    Comma,
    Colon,
    Semicolon,

    // Plus,
    Minus,
    // Backslash,
    // Slash,
    // Dollar,
    // Asterix,
    // Caret,
    Question,

    At,
    Eq,
    Neq,
    Gt,
    Lt,

    LParen,
    RParen,
    LCurly,
    RCurly,
    LBracket,
    RBracket,

    Documentation,
    Path,
}

pub type Token<'src> = (TokenVariant, TokenData<'src>);
pub type XenoTokens<'src> = Vec<Token<'src>>;

impl<'src> fmt::Display for TokenData<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\"{}\" on line:{} column:{}",
            self.v,
            self.l + 1,
            self.c + 1
        )
    }
}
