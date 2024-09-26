
pub enum Token {
    Identifier,

    TypeKeyword,
    Set,
    Enum,

    Literal,
    Regex,
    
    Disjunction,
    Dot,
    Comma,
    
    LParen,
    RParen,
    LCurly,
    RCurly,
    LBracket,
    RBracket,
    LAngle,
    RAngle,
    
    EOF,
}

pub enum NumberVariant {
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Decimal,
}