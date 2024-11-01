#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenData<'src> {
    pub v: &'src str,
    pub l: usize,
    pub c: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberType {
    Int(bool, u8), // signed, size
    Float(u8),
    // BigInt,
    // Real,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenVariant {
    Identifier,
    Type,
    Set,

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
    Asterix,
    Backslash,
    Dollar,

    At,
    Eq,
    Neq,
    Caret,
    SymmDiff,

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
