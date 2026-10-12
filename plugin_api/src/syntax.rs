use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Syntax {
    pub tokens: Vec<Token>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Token {
    pub value: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Declaration {
    Import {
        path: Vec<String>,
        location: usize,
    },
    Type {
        docs: Option<String>,
        name: usize,
        generics: Option<Vec<(usize, Option<usize>)>>,
        ty: Type,
        annotations: Vec<Annotation>,
        from: usize,
        to: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub ident: usize,
    pub params: Vec<Expr>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Expr {
    Regex(usize),
    Annotation(Annotation),
    Type(Type),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Type {
    Simple(Simple),
    Tuple(Vec<Simple>),
    Set {
        keyword: usize,
        element_type: Option<Simple>,
        values: Option<Vec<Literal>>,
        last_token: usize,
    },
    Struct(Vec<Field>),
    Enum(Vec<Field>),
    Sum(Vec<Simple>),
    Intersection(Vec<Simple>),
}

pub type Field = (usize, Simple, Option<usize>);

#[derive(Debug, Serialize, Deserialize)]
pub enum Simple {
    Literal(Literal),
    Identifier(usize, Option<Vec<Simple>>),
    Array(usize, Option<Vec<Simple>>),
    Optional(Box<Simple>),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Literal {
    Integer {
        value: String,
        signed: bool,
        bits: Option<u64>,
        token: usize,
        cast: Option<usize>,
    },
    Float {
        value: String,
        signed: bool,
        precision: u64,
        scale: u64,
        size: FloatSize,
        token: usize,
        cast: Option<usize>,
    },
    String(String, usize),
    Boolean(bool, usize),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FloatSize {
    F32,
    F64,
    Decimal,
}
