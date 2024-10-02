#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenData<'src> {
    pub v: &'src str,
    pub src_index: usize,
    pub l: usize,
    pub c: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Token<'src> {
    Identifier(TokenData<'src>),
    Type(TokenData<'src>),
    Set(TokenData<'src>),

    Number(TokenData<'src>),
    String(TokenData<'src>),
    Regex(TokenData<'src>),

    Or(TokenData<'src>),
    And(TokenData<'src>),
    Dot(TokenData<'src>),
    Comma(TokenData<'src>),
    Colon(TokenData<'src>),
    Semicolon(TokenData<'src>),

    Plus(TokenData<'src>),
    Minus(TokenData<'src>),
    Asterix(TokenData<'src>),
    Slash(TokenData<'src>),
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
