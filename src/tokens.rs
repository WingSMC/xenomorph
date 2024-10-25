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
pub enum Token<'src> {
    Identifier(TokenData<'src>),
    Type(TokenData<'src>),
    Set(TokenData<'src>),

    Number(TokenData<'src>, NumberType),
    String(TokenData<'src>),
    Regex(TokenData<'src>),

    Not(TokenData<'src>),
    Or(TokenData<'src>),
    And(TokenData<'src>),
    Dot(TokenData<'src>),
    Comma(TokenData<'src>),
    Colon(TokenData<'src>),
    Semicolon(TokenData<'src>),

    Plus(TokenData<'src>),
    Minus(TokenData<'src>),
    Asterix(TokenData<'src>),
    Backslash(TokenData<'src>),
    Dollar(TokenData<'src>),

    At(TokenData<'src>),
    Eq(TokenData<'src>),
    Neq(TokenData<'src>),
    Caret(TokenData<'src>),
    SymmDiff(TokenData<'src>),

    Gt(TokenData<'src>),
    Lt(TokenData<'src>),

    LParen(TokenData<'src>),
    RParen(TokenData<'src>),
    LCurly(TokenData<'src>),
    RCurly(TokenData<'src>),
    LBracket(TokenData<'src>),
    RBracket(TokenData<'src>),

    EOF,
}
