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

    Plus,
    Minus,
    Backslash,
    Slash,
    Dollar,
    Asterix,
    Caret,
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
}

pub type Token<'src> = (TokenVariant, TokenData<'src>);
pub type XenoTokens<'src> = Vec<Token<'src>>;

impl fmt::Display for TokenVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenVariant::And => write!(f, "And"),
            TokenVariant::Asterix => write!(f, "Asterix"),
            TokenVariant::At => write!(f, "At"),
            TokenVariant::Backslash => write!(f, "Backslash"),
            TokenVariant::Caret => write!(f, "Caret"),
            TokenVariant::Colon => write!(f, "Colon"),
            TokenVariant::Comma => write!(f, "Comma"),
            TokenVariant::Documentation => write!(f, "Documentation"),
            TokenVariant::Dollar => write!(f, "Dollar"),
            TokenVariant::Dot => write!(f, "Dot"),
            TokenVariant::Enum => write!(f, "Enum"),
            TokenVariant::Eq => write!(f, "Eq"),
            TokenVariant::False => write!(f, "False"),
            TokenVariant::Gt => write!(f, "Gt"),
            TokenVariant::Identifier => write!(f, "Identifier"),
            TokenVariant::Import => write!(f, "Import"),
            TokenVariant::LBracket => write!(f, "LBracket"),
            TokenVariant::LCurly => write!(f, "LCurly"),
            TokenVariant::LParen => write!(f, "LParen"),
            TokenVariant::Lt => write!(f, "Lt"),
            TokenVariant::Minus => write!(f, "Minus"),
            TokenVariant::Neq => write!(f, "Neq"),
            TokenVariant::Not => write!(f, "Not"),
            TokenVariant::Number => write!(f, "Number"),
            TokenVariant::Or => write!(f, "Or"),
            TokenVariant::Plus => write!(f, "Plus"),
            TokenVariant::Question => write!(f, "Question"),
            TokenVariant::RBracket => write!(f, "RBracket"),
            TokenVariant::RCurly => write!(f, "RCurly"),
            TokenVariant::RParen => write!(f, "RParen"),
            TokenVariant::Regex => write!(f, "Regex"),
            TokenVariant::Semicolon => write!(f, "Semicolon"),
            TokenVariant::Set => write!(f, "Set"),
            TokenVariant::Slash => write!(f, "Slash"),
            TokenVariant::String => write!(f, "String"),
            TokenVariant::True => write!(f, "True"),
            TokenVariant::Type => write!(f, "Type"),
            TokenVariant::Validator => write!(f, "Validator"),
        }
    }
}

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
