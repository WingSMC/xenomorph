use crate::source::TokenData;
use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use std::fmt;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Declaration {
    Import {
        path: Vec<String>,
        location: TokenData,
    },
    Type {
        docs: Option<String>,
        name: TokenData,
        generics: Option<Vec<(TokenData, Option<TokenData>)>>,
        ty: XenoType,
        from: TokenData,
        to: TokenData,
    },
    Custom {
        plugin_id: String,
        decl_id: String,
        docs: Option<String>,
        name: Option<TokenData>,
        value: Vec<u8>,
    },
}

// pub type BinaryExpr<'src> = Box<(Expr, Expr)>;
// #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
// pub enum BinaryExprType {
//     Union,
//     Intersection,
//     Difference,
//     SymmetricDifference,
//     // Xor,
//     // Range,
//     // Add,
//     // Remove,
// }

/**
 * Something on the right side of a type declaration
 * Type is either the parent type or a new type,
 * and the annotations are the constraints/meta-info on the type.
 */
pub type XenoType = (Type, Vec<Annotation>);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    pub ident: TokenData,
    pub params: Vec<Expr>,
}

/**
 * A type expression (or literal expression)
 */
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Type {
    /** Read `SimpleType` docs */
    Simple(SimpleType),

    /** e.g. [Type1, Type2, Type3] */
    Tuple(Vec<SimpleType>),
    /**
    A unique collection with an optional element type and optional constant prefill.

    e.g. `set<string>`, `set ["Blue", "Red"]`, or
    `set<string> ["Blue", "Red"]`.
    */
    Set(SetType),

    /** e.g. { field1: Type1, field2: Type2 } */
    Struct(Vec<KeyValExpr>),
    /**
    List of mutually exclusive variants, each with a name and type.

    enum { variant1: Type1, variant2: Type2 }
    */
    Enum(Vec<KeyValExpr>),
    /** e.g. | Type1 | Type2 | Type3 */
    Sum(Vec<SimpleType>),
    /** e.g. & Type1 & Type2 & Type3 */
    Intersection(Vec<SimpleType>),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SetType {
    /// The `set` keyword, retained for diagnostics and source navigation.
    pub keyword: TokenData,
    /// An explicit element type from `set<T>`.
    pub element_type: Option<SimpleType>,
    /// A literal-only prefill. `None` means no prefill; `Some([])` is an
    /// explicitly empty prefill.
    pub values: Option<Vec<Literal>>,
    /// The closing `>` or `]`, or the keyword when parsing cannot advance.
    pub last_token: TokenData,
}

/** Used for struct fields, sets, tuples */
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SimpleType {
    /** e.g. 42, 3.141592653, true, "hello" */
    Literal(Literal),

    /** name of a type */
    Identifier(TokenData, Option<Vec<SimpleType>>),

    /** Postfix array syntax, e.g. u32[] */
    Array(TokenData, Option<Vec<SimpleType>>),

    /** Prefix optional syntax, e.g. ?u32 */
    Optional(Box<SimpleType>),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Literal {
    Int(IntLiteral),
    Float(FloatLiteral),
    String(String, TokenData),
    Boolean(bool, TokenData),
}

/// Storage required by an integer literal. `Bits` is either the exact minimum
/// bit count inferred from the value or the fixed width requested by a cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IntegerSize {
    Bits(u64),
    Arbitrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IntegerRepresentation {
    pub signed: bool,
    pub size: IntegerSize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntLiteral {
    #[serde(with = "crate::numbers::integer")]
    pub value: BigInt,
    pub representation: IntegerRepresentation,
    pub token: TokenData,
    pub cast: Option<TokenData>,
}

/// The smallest supported floating-point storage that can round-trip the
/// source decimal, or an explicitly requested storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum FloatSize {
    F32 = 1,
    F64 = 2,
    Decimal = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FloatRepresentation {
    /// IEEE floats and arbitrary-precision decimals support negative values.
    pub signed: bool,
    /// Significant decimal digits in the source literal.
    pub precision: u64,
    /// Decimal digits following the decimal point in the source literal.
    pub scale: u64,
    pub size: FloatSize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FloatLiteral {
    #[serde(with = "crate::numbers::decimal")]
    pub value: BigDecimal,
    pub representation: FloatRepresentation,
    pub token: TokenData,
    pub cast: Option<TokenData>,
}

pub type KeyValExpr = (
    // Field identifier. Quoted source keys are normalized to their unquoted
    // contents by the parser, so this token is owned rather than borrowed.
    TokenData,
    // field type
    SimpleType,
    // documentation comment for the field
    Option<TokenData>,
);

/** Used in annotation params */
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    /// e.g. /regex/
    Regex(TokenData),
    /// Identifier with params, e.g. @annotation(param1, param2)
    Annotation(Annotation),
    /// e.g. u32, "asd", [Type1, Type2], { field1: Type1, field2: Type2 }
    Type(Type),
}

impl fmt::Debug for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Declaration::Import { path, location } => f
                .debug_struct("Import")
                .field("path", path)
                .field("location", location)
                .finish(),
            Declaration::Type {
                docs,
                name,
                generics,
                ty,
                from,
                to,
            } => f
                .debug_struct("Type")
                .field("docs", docs)
                .field("name", name)
                .field("generics", generics)
                .field("ty", ty)
                .field("from", from)
                .field("to", to)
                .finish(),
            Declaration::Custom {
                plugin_id,
                decl_id,
                docs,
                name,
                ..
            } => f
                .debug_struct("Custom")
                .field("plugin_id", plugin_id)
                .field("decl_id", decl_id)
                .field("docs", docs)
                .field("name", name)
                .field("value", &"<opaque>")
                .finish(),
        }
    }
}

impl Annotation {
    pub fn get_last_token(&self) -> TokenData {
        self.params
            .last()
            .and_then(|e| e.get_last_token())
            .unwrap_or(self.ident)
    }
}

impl Type {
    pub fn get_last_token(&self) -> Option<TokenData> {
        match self {
            Type::Simple(ty) => Some(ty.get_last_token()),
            Type::Struct(fs) | Type::Enum(fs) => fs.last().map(|f| f.1.get_last_token()),
            Type::Tuple(ts) | Type::Intersection(ts) | Type::Sum(ts) => {
                ts.last().map(|t| t.get_last_token())
            }
            Type::Set(set) => Some(set.last_token),
        }
    }
}

impl SimpleType {
    pub fn is_optional(&self) -> bool {
        matches!(self, SimpleType::Optional(_))
    }
    pub fn inner(&self) -> &SimpleType {
        match self {
            SimpleType::Optional(inner) => inner.inner(),
            ty => ty,
        }
    }
    pub fn get_last_token(&self) -> TokenData {
        match self {
            SimpleType::Optional(inner) => inner.get_last_token(),
            SimpleType::Literal(l) => l.get_last_token(),
            SimpleType::Identifier(d, arguments) | SimpleType::Array(d, arguments) => arguments
                .as_deref()
                .and_then(|arguments| arguments.last())
                .map(SimpleType::get_last_token)
                .unwrap_or(*d),
        }
    }
}

impl Literal {
    pub fn get_last_token(&self) -> TokenData {
        match self {
            Literal::Int(literal) => literal.cast.unwrap_or(literal.token),
            Literal::Float(literal) => literal.cast.unwrap_or(literal.token),
            Literal::String(_, token) | Literal::Boolean(_, token) => *token,
        }
    }
    pub fn token(&self) -> TokenData {
        match self {
            Literal::Int(literal) => literal.token,
            Literal::Float(literal) => literal.token,
            Literal::String(_, token) | Literal::Boolean(_, token) => *token,
        }
    }
    pub fn cast_target(&self) -> Option<TokenData> {
        match self {
            Literal::Int(literal) => literal.cast,
            Literal::Float(literal) => literal.cast,
            Literal::String(_, _) | Literal::Boolean(_, _) => None,
        }
    }
    pub fn semantic_type_name<'a>(&self, source: &'a str) -> &'a str {
        match self {
            Literal::Int(literal) => literal.cast.map_or("integer", |target| target.text(source)),
            Literal::Float(literal) => literal.cast.map_or("number", |target| target.text(source)),
            Literal::String(_, _) => "string",
            Literal::Boolean(_, _) => "bool",
        }
    }
    pub fn source_text(&self, source: &str) -> String {
        match self.cast_target() {
            Some(target) => format!("{} as {}", self.token().text(source), target.text(source)),
            None => self.token().text(source).to_string(),
        }
    }
}

impl Expr {
    pub fn get_last_token(&self) -> Option<TokenData> {
        match self {
            Expr::Regex(d) => Some(*d),
            Expr::Annotation(a) => Some(a.get_last_token()),
            Expr::Type(t) => t.get_last_token(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TokenVariant {
    Identifier = 1,
    Type = 2,
    Import = 3,
    Validator = 4,
    Set = 5,
    Enum = 6,
    As = 7,
    True = 8,
    False = 9,

    Number = 10,
    String = 11,
    Regex = 12,
    Not = 13,
    Or = 14,
    And = 15,
    Dot = 16,
    Comma = 17,
    Colon = 18,
    Semicolon = 19,

    // Plus,
    Minus = 20,
    // Backslash,
    // Slash,
    // Dollar,
    // Asterix,
    // Caret,
    // Percent,
    // Hash,
    Question = 21,

    At = 22,
    Eq = 23,
    Neq = 24,
    Gt = 25,
    Lt = 26,

    LParen = 27,
    RParen = 28,
    LCurly = 29,
    RCurly = 30,
    LBracket = 31,
    RBracket = 32,

    Documentation = 33,
    Path = 34,
}

pub type Token = (TokenVariant, TokenData);
pub type XenoTokens = Vec<Token>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Syntax {
    pub source: String,
    pub tokens: XenoTokens,
    pub declarations: Vec<Declaration>,
}

impl Declaration {
    /// Applies a relocation after an edit to a preceding declaration.
    pub fn relocate(&mut self, delta: i64) {
        fn range(r: &mut TokenData, delta: i64) {
            r.start = (r.start as i64 + delta)
                .try_into()
                .expect("invalid relocation");
            r.end = (r.end as i64 + delta)
                .try_into()
                .expect("invalid relocation");
        }
        fn literal(l: &mut Literal, d: i64) {
            match l {
                Literal::Int(l) => {
                    range(&mut l.token, d);
                    if let Some(c) = &mut l.cast {
                        range(c, d)
                    }
                }
                Literal::Float(l) => {
                    range(&mut l.token, d);
                    if let Some(c) = &mut l.cast {
                        range(c, d)
                    }
                }
                Literal::String(_, r) | Literal::Boolean(_, r) => range(r, d),
            }
        }
        fn simple(s: &mut SimpleType, d: i64) {
            match s {
                SimpleType::Optional(s) => simple(s, d),
                SimpleType::Literal(l) => literal(l, d),
                SimpleType::Identifier(r, args) | SimpleType::Array(r, args) => {
                    range(r, d);
                    for s in args.as_deref_mut().unwrap_or_default() {
                        simple(s, d)
                    }
                }
            }
        }
        fn ty(t: &mut Type, d: i64) {
            match t {
                Type::Simple(s) => simple(s, d),
                Type::Tuple(v) | Type::Sum(v) | Type::Intersection(v) => {
                    for s in v {
                        simple(s, d)
                    }
                }
                Type::Struct(v) | Type::Enum(v) => {
                    for (r, s, doc) in v {
                        range(r, d);
                        simple(s, d);
                        if let Some(r) = doc {
                            range(r, d)
                        }
                    }
                }
                Type::Set(s) => {
                    range(&mut s.keyword, d);
                    range(&mut s.last_token, d);
                    if let Some(e) = &mut s.element_type {
                        simple(e, d)
                    }
                    for l in s.values.as_deref_mut().unwrap_or_default() {
                        literal(l, d)
                    }
                }
            }
        }
        fn ann(a: &mut Annotation, d: i64) {
            range(&mut a.ident, d);
            for e in &mut a.params {
                match e {
                    Expr::Regex(r) => range(r, d),
                    Expr::Type(t) => ty(t, d),
                    Expr::Annotation(a) => ann(a, d),
                }
            }
        }
        match self {
            Self::Import { location, .. } => range(location, delta),
            Self::Type {
                name,
                generics,
                ty: t,
                from,
                to,
                ..
            } => {
                range(name, delta);
                range(from, delta);
                range(to, delta);
                for (n, c) in generics.as_deref_mut().unwrap_or_default() {
                    range(n, delta);
                    if let Some(c) = c {
                        range(c, delta)
                    }
                }
                ty(&mut t.0, delta);
                for a in &mut t.1 {
                    ann(a, delta)
                }
            }
            Self::Custom { name, .. } => {
                if let Some(r) = name {
                    range(r, delta)
                }
            }
        }
    }
}
