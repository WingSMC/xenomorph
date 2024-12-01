use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenData<'src> {
    pub v: &'src str,
    pub l: usize,
    pub c: usize,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenVariant {
    Identifier,
    Type,
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
    Dollar,
    Asterix,
    Caret,

    At,
    Eq,
    Neq,
    SymmDiff,
    Range,

    Gt,
    Lt,

    LParen,
    RParen,
    LCurly,
    RCurly,
    LBracket,
    RBracket,
}

pub type Token<'src> = (TokenVariant, TokenData<'src>);


impl fmt::Display for TokenVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenVariant::Identifier => write!(f, "Identifier"),
            TokenVariant::Type => write!(f, "Type"),
            TokenVariant::Set => write!(f, "Set"),
            TokenVariant::Enum => write!(f, "Enum"),
            TokenVariant::True => write!(f, "True"),
            TokenVariant::False => write!(f, "False"),
            TokenVariant::Number => write!(f, "Number"),
            TokenVariant::String => write!(f, "String"),
            TokenVariant::Regex => write!(f, "Regex"),
            TokenVariant::Not => write!(f, "Not"),
            TokenVariant::Or => write!(f, "Or"),
            TokenVariant::And => write!(f, "And"),
            TokenVariant::Dot => write!(f, "Dot"),
            TokenVariant::Comma => write!(f, "Comma"),
            TokenVariant::Colon => write!(f, "Colon"),
            TokenVariant::Semicolon => write!(f, "Semicolon"),
            TokenVariant::Plus => write!(f, "Plus"),
            TokenVariant::Minus => write!(f, "Minus"),
            TokenVariant::Asterix => write!(f, "Asterix"),
            TokenVariant::Backslash => write!(f, "Backslash"),
            TokenVariant::Dollar => write!(f, "Dollar"),
            TokenVariant::At => write!(f, "At"),
            TokenVariant::Eq => write!(f, "Eq"),
            TokenVariant::Neq => write!(f, "Neq"),
            TokenVariant::Caret => write!(f, "Caret"),
            TokenVariant::SymmDiff => write!(f, "SymmDiff"),
            TokenVariant::Gt => write!(f, "Gt"),
            TokenVariant::Lt => write!(f, "Lt"),
            TokenVariant::LParen => write!(f, "LParen"),
            TokenVariant::RParen => write!(f, "RParen"),
            TokenVariant::LCurly => write!(f, "LCurly"),
            TokenVariant::RCurly => write!(f, "RCurly"),
            TokenVariant::LBracket => write!(f, "LBracket"),
            TokenVariant::RBracket => write!(f, "RBracket"),
            TokenVariant::Range => write!(f, "Range"),
        }
    }
}

impl<'src> fmt::Display for TokenData<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\" on line:{} column:{}", self.v, self.l, self.c)
    }
}
