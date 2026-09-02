use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use std::{fmt, str::FromStr};

use crate::{
    lexer::{Token, TokenVariant},
    parser::Parser,
    utils::extract_documentation,
    TokenData, XenoDiagSeverity, XenoDiagnostic,
};

pub enum Declaration<'src> {
    Import {
        path: Vec<&'src str>,
        location: &'src TokenData<'src>,
    },
    Type {
        docs: Option<&'src str>,
        name: &'src TokenData<'src>,
        generics: Option<Vec<(&'src TokenData<'src>, Option<&'src TokenData<'src>>)>>,
        ty: XenoType<'src>,
        from: &'src TokenData<'src>,
        to: &'src TokenData<'src>,
    },
    Custom {
        plugin_id: &'static str,
        decl_id: &'static str,
        docs: Option<&'src str>,
        name: Option<&'src TokenData<'src>>,
        value: Box<dyn std::any::Any + Send + Sync>,
    },
}

impl fmt::Debug for Declaration<'_> {
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

// pub type BinaryExpr<'src> = Box<(Expr<'src>, Expr<'src>)>;
// #[derive(Debug, Clone, Copy, PartialEq)]
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
pub type XenoType<'src> = (Type<'src>, Vec<Annotation<'src>>);

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation<'src> {
    pub ident: &'src TokenData<'src>,
    pub params: Vec<Expr<'src>>,
}

impl<'src> Annotation<'src> {
    pub fn get_last_token(&self) -> &'src TokenData<'src> {
        self.params
            .last()
            .and_then(|e| e.get_last_token())
            .unwrap_or(self.ident)
    }

    pub fn parse_annotations(parser: &mut Parser<'src>) -> Vec<Annotation<'src>> {
        let mut annotations = Vec::new();
        while let Some((TokenVariant::At, _)) = parser.peek() {
            if let Some(a) = Self::parse_annotation(parser) {
                annotations.push(a)
            }
        }
        annotations
    }

    pub fn parse_annotation(parser: &mut Parser<'src>) -> Option<Annotation<'src>> {
        parser.expect(TokenVariant::At)?;
        let ident = parser.expect(TokenVariant::Identifier)?;

        let params = if let Some((TokenVariant::LParen, _)) = parser.peek() {
            parser.parse_list(
                TokenVariant::LParen,
                TokenVariant::Comma,
                Some(TokenVariant::RParen),
                Expr::parse,
            )?
        } else {
            Vec::new()
        };

        Some(Annotation { ident, params })
    }
}

/**
 * A type expression (or literal expression)
 */
#[derive(Debug, Clone, PartialEq)]
pub enum Type<'src> {
    /** Read `SimpleType` docs */
    Simple(SimpleType<'src>),

    /** e.g. [Type1, Type2, Type3] */
    Tuple(Vec<SimpleType<'src>>),
    /**
    A unique collection with an optional element type and optional constant prefill.

    e.g. `set<string>`, `set ["Blue", "Red"]`, or
    `set<string> ["Blue", "Red"]`.
    */
    Set(SetType<'src>),

    /** e.g. { field1: Type1, field2: Type2 } */
    Struct(Vec<KeyValExpr<'src>>),
    /**
    List of mutually exclusive variants, each with a name and type.

    enum { variant1: Type1, variant2: Type2 }
    */
    Enum(Vec<KeyValExpr<'src>>),
    /** e.g. | Type1 | Type2 | Type3 */
    Sum(Vec<SimpleType<'src>>),
    /** e.g. & Type1 & Type2 & Type3 */
    Intersection(Vec<SimpleType<'src>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetType<'src> {
    /// The `set` keyword, retained for diagnostics and source navigation.
    pub keyword: &'src TokenData<'src>,
    /// An explicit element type from `set<T>`.
    pub element_type: Option<SimpleType<'src>>,
    /// A literal-only prefill. `None` means no prefill; `Some([])` is an
    /// explicitly empty prefill.
    pub values: Option<Vec<Literal<'src>>>,
    /// The closing `>` or `]`, or the keyword when parsing cannot advance.
    pub last_token: &'src TokenData<'src>,
}

/** Used for struct fields, sets, tuples */
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleType<'src> {
    /** e.g. 42, 3.141592653, true, "hello" */
    Literal(Literal<'src>),

    /** name of a type */
    Identifier(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),

    /** Postfix array syntax, e.g. u32[] */
    Array(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),

    /** Prefix optional syntax, e.g. ?u32 */
    Optional(Box<SimpleType<'src>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal<'src> {
    Int(IntLiteral<'src>),
    Float(FloatLiteral<'src>),
    String(String, &'src TokenData<'src>),
    Boolean(bool, &'src TokenData<'src>),
}

/// Storage required by an integer literal. `Bits` is either the exact minimum
/// bit count inferred from the value or the fixed width requested by a cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerSize {
    Bits(u64),
    Arbitrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegerRepresentation {
    pub signed: bool,
    pub size: IntegerSize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntLiteral<'src> {
    pub value: BigInt,
    pub representation: IntegerRepresentation,
    pub token: &'src TokenData<'src>,
    pub cast: Option<&'src TokenData<'src>>,
}

/// The smallest supported floating-point storage that can round-trip the
/// source decimal, or an explicitly requested storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatSize {
    F32,
    F64,
    Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloatRepresentation {
    /// IEEE floats and arbitrary-precision decimals support negative values.
    pub signed: bool,
    /// Significant decimal digits in the source literal.
    pub precision: u64,
    /// Decimal digits following the decimal point in the source literal.
    pub scale: u64,
    pub size: FloatSize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FloatLiteral<'src> {
    pub value: BigDecimal,
    pub representation: FloatRepresentation,
    pub token: &'src TokenData<'src>,
    pub cast: Option<&'src TokenData<'src>>,
}

pub type KeyValExpr<'src> = (
    // field identifier
    &'src TokenData<'src>,
    // field type
    SimpleType<'src>,
    // documentation comment for the field
    Option<&'src TokenData<'src>>,
);

/** Used in annotation params */
#[derive(Debug, Clone, PartialEq)]
pub enum Expr<'src> {
    /// e.g. /regex/
    Regex(&'src TokenData<'src>),
    /// Identifier with params, e.g. @annotation(param1, param2)
    Annotation(Annotation<'src>),
    /// e.g. u32, "asd", [Type1, Type2], { field1: Type1, field2: Type2 }
    Type(Type<'src>),
}

impl<'src> Declaration<'src> {
    pub fn parse(parser: &mut Parser<'src>) -> Option<Declaration<'src>> {
        let token = parser.need_next()?;
        let docs = match token {
            (TokenVariant::Documentation, data) => Some(extract_documentation(data)),
            _ => None,
        };

        let (var, d) = if docs.is_some() {
            parser.need_next()?
        } else {
            token
        };

        let dec = match var {
            TokenVariant::Type => Declaration::parse_type_declaration(parser, d, docs),
            TokenVariant::Import => Declaration::parse_import_declaration(parser, d, docs),
            _ => {
                parser.diagnostics.push(XenoDiagnostic {
                    location: d.clone(),
                    message: format!("Unknown declaration at {}, ({:?}).", d, var),
                    severity: XenoDiagSeverity::Err,
                });
                return None;
            }
        }?;

        parser.expect(TokenVariant::Semicolon)?;

        Some(dec)
    }

    pub fn parse_type_declaration(
        parser: &mut Parser<'src>,
        from: &'src TokenData<'src>,
        docs: Option<&'src str>,
    ) -> Option<Declaration<'src>> {
        let name = parser.expect(TokenVariant::Identifier)?;

        let generics = if let Some((TokenVariant::Lt, _)) = parser.peek() {
            parser.step_forward();

            let mut generics = Vec::new();
            while let Some((TokenVariant::Identifier, d)) = parser.peek() {
                parser.step_forward();

                let constraint = if let Some((TokenVariant::Colon, _)) = parser.peek() {
                    parser.step_forward();
                    let ident = parser.expect(TokenVariant::Identifier)?;
                    Some(ident)
                } else {
                    None
                };
                parser.skip_if(TokenVariant::Comma);

                generics.push((d, constraint));
            }
            parser.expect(TokenVariant::Gt)?;
            Some(generics)
        } else {
            None
        };

        parser.expect(TokenVariant::Eq)?;

        let core_type = Type::parse(parser)?;
        let anns = Annotation::parse_annotations(parser);

        let t = (core_type, anns);
        let to =
            t.1.last()
                .map(|a| a.get_last_token())
                .or_else(|| t.0.get_last_token())
                .unwrap_or(from);
        Some(Declaration::Type {
            docs,
            name,
            generics,
            ty: t,
            from,
            to,
        })
    }
    pub fn parse_import_declaration(
        parser: &mut Parser<'src>,
        location: &'src TokenData<'src>,
        docs: Option<&'src str>,
    ) -> Option<Declaration<'src>> {
        if docs.is_some() {
            parser.diagnostics.push(XenoDiagnostic {
                location: location.clone(),
                message: "Import declarations cannot have documentation comments.".to_string(),
                severity: XenoDiagSeverity::Info,
            });
        }

        // TODO can this be used with LSP for path recommendations?
        let path_tok = parser.expect(TokenVariant::Path)?;
        let path = path_tok.v.split('/').collect::<Vec<&str>>();

        Some(Declaration::Import { path, location })
    }
}

impl<'src> Type<'src> {
    pub fn get_last_token(&self) -> Option<&'src TokenData<'src>> {
        match self {
            Type::Simple(ty) => Some(ty.get_last_token()),
            Type::Struct(fs) | Type::Enum(fs) => fs.last().map(|f| f.1.get_last_token()),
            Type::Tuple(ts) | Type::Intersection(ts) | Type::Sum(ts) => {
                ts.last().map(|t| t.get_last_token())
            }
            Type::Set(set) => Some(set.last_token),
        }
    }

    pub fn parse(parser: &mut Parser<'src>) -> Option<Type<'src>> {
        let (variant, _) = parser.peek()?;
        match variant {
            TokenVariant::Set => Self::parse_set(parser),
            TokenVariant::LBracket => Self::parse_tuple(parser).map(Type::Tuple),

            TokenVariant::Enum => Self::parse_enum(parser),
            TokenVariant::LCurly => Self::parse_struct(parser).map(Type::Struct),

            TokenVariant::And => parser
                .parse_list(
                    TokenVariant::And,
                    TokenVariant::And,
                    None,
                    SimpleType::parse,
                )
                .map(Type::Intersection),
            TokenVariant::Or => parser
                .parse_list(TokenVariant::Or, TokenVariant::Or, None, SimpleType::parse)
                .map(Type::Sum),

            _ => SimpleType::parse(parser).map(Type::Simple),
        }
    }

    fn parse_set(parser: &mut Parser<'src>) -> Option<Type<'src>> {
        let keyword = parser.expect(TokenVariant::Set)?;
        let mut last_token = keyword;

        let element_type = if parser.skip_if(TokenVariant::Lt) {
            let element_type = SimpleType::parse(parser)?;
            last_token = parser.expect(TokenVariant::Gt)?;
            Some(element_type)
        } else {
            None
        };

        let values = if parser.peek_is(TokenVariant::LBracket) {
            let values = parser.parse_list(
                TokenVariant::LBracket,
                TokenVariant::Comma,
                Some(TokenVariant::RBracket),
                Self::parse_set_value,
            )?;
            last_token = parser.previous().unwrap_or(last_token);
            Some(values)
        } else {
            None
        };

        if element_type.is_none() && values.is_none() {
            parser.diagnostics.push(XenoDiagnostic {
                location: keyword.clone(),
                message: "A set requires an element type, a literal prefill, or both.".to_string(),
                severity: XenoDiagSeverity::Err,
            });
            return None;
        }

        Some(Type::Set(SetType {
            keyword,
            element_type,
            values,
            last_token,
        }))
    }

    fn parse_set_value(parser: &mut Parser<'src>) -> Option<Literal<'src>> {
        let token = parser.peek()?;
        let literal = Literal::parse(parser, token);
        if literal.is_none()
            && !matches!(
                token.0,
                TokenVariant::Number
                    | TokenVariant::String
                    | TokenVariant::True
                    | TokenVariant::False
            )
        {
            parser.diagnostics.push(XenoDiagnostic {
                location: token.1.clone(),
                message: "Set prefills can contain only literals.".to_string(),
                severity: XenoDiagSeverity::Err,
            });
        }
        literal
    }

    fn parse_tuple(parser: &mut Parser<'src>) -> Option<Vec<SimpleType<'src>>> {
        parser.parse_list(
            TokenVariant::LBracket,
            TokenVariant::Comma,
            Some(TokenVariant::RBracket),
            SimpleType::parse,
        )
    }

    fn parse_enum(parser: &mut Parser<'src>) -> Option<Type<'src>> {
        parser.expect(TokenVariant::Enum)?;

        let fs = Self::parse_struct(parser)?;
        Some(Type::Enum(fs))
    }

    fn parse_struct(parser: &mut Parser<'src>) -> Option<Vec<KeyValExpr<'src>>> {
        parser.expect(TokenVariant::LCurly)?;

        let mut fields = Vec::new();
        while !parser.peek_is(TokenVariant::RCurly) && parser.peek().is_some() {
            let docs = if let Some((TokenVariant::Documentation, d)) = parser.peek() {
                parser.step_forward();
                Some(d)
            } else {
                None
            };

            if parser.peek_is(TokenVariant::RCurly) {
                if let Some(docs) = docs {
                    parser.diagnostics.push(XenoDiagnostic {
                        location: docs.clone(),
                        message: "Documentation comment without a field.".to_string(),
                        severity: XenoDiagSeverity::Warn,
                    });
                }
                break;
            }

            let field = 'field: {
                let Some(key) = parser.expect_at_current(TokenVariant::Identifier) else {
                    break 'field None;
                };
                if parser.expect_at_current(TokenVariant::Colon).is_none() {
                    break 'field None;
                }
                let Some(ty) = SimpleType::parse(parser) else {
                    break 'field None;
                };
                Some((key, ty, docs))
            };

            if let Some(field) = field {
                fields.push(field);
                if parser.peek_is(TokenVariant::RCurly) {
                    continue;
                }
                if parser.skip_if(TokenVariant::Comma) {
                    continue;
                }

                let _ = parser.expect_at_current(TokenVariant::Comma);
            }

            match parser.recover_to_any(&[TokenVariant::Comma, TokenVariant::RCurly]) {
                Some(TokenVariant::Comma) => parser.step_forward(),
                Some(TokenVariant::RCurly) | None => break,
                _ => unreachable!(),
            }
        }

        parser.expect(TokenVariant::RCurly)?;

        Some(fields)
    }
}

impl<'src> SimpleType<'src> {
    /// Whether the type carries the `?` prefix.
    pub fn is_optional(&self) -> bool {
        matches!(self, SimpleType::Optional(_))
    }

    /// The type without its optional wrapper.
    pub fn inner(&self) -> &SimpleType<'src> {
        match self {
            SimpleType::Optional(inner) => inner.inner(),
            ty => ty,
        }
    }

    pub fn get_last_token(&self) -> &'src TokenData<'src> {
        match self {
            SimpleType::Optional(inner) => inner.get_last_token(),
            SimpleType::Literal(l) => l.get_last_token(),
            SimpleType::Identifier(d, arguments) | SimpleType::Array(d, arguments) => arguments
                .as_deref()
                .and_then(|arguments| arguments.last())
                .map(SimpleType::get_last_token)
                .unwrap_or(d),
        }
    }

    pub fn parse(parser: &mut Parser<'src>) -> Option<SimpleType<'src>> {
        let is_optional = parser.skip_if(TokenVariant::Question);
        let base = Self::parse_base(parser)?;
        Some(match is_optional {
            true => SimpleType::Optional(Box::new(base)),
            false => base,
        })
    }

    fn parse_base(parser: &mut Parser<'src>) -> Option<SimpleType<'src>> {
        let t = parser.peek()?;
        match t.0 {
            TokenVariant::Identifier => {
                parser.step_forward();

                let arguments = if parser.peek_is(TokenVariant::Lt) {
                    Some(parser.parse_list(
                        TokenVariant::Lt,
                        TokenVariant::Comma,
                        Some(TokenVariant::Gt),
                        SimpleType::parse,
                    )?)
                } else {
                    None
                };

                let is_array = if parser.skip_if(TokenVariant::LBracket) {
                    parser.expect_at_current(TokenVariant::RBracket)?;
                    true
                } else {
                    false
                };

                if is_array {
                    Some(SimpleType::Array(&t.1, arguments))
                } else {
                    Some(SimpleType::Identifier(&t.1, arguments))
                }
            }

            _ => Literal::parse(parser, t)
                .map(SimpleType::Literal)
                .or_else(|| {
                    parser.diagnostics.push(XenoDiagnostic {
                        location: t.1.clone(),
                        message: format!("Unexpected token {:?} in simple type expression.", t.0),
                        severity: XenoDiagSeverity::Err,
                    });
                    None
                }),
        }
    }
}

impl<'src> Literal<'src> {
    pub fn get_last_token(&self) -> &'src TokenData<'src> {
        match self {
            Literal::Int(literal) => literal.cast.unwrap_or(literal.token),
            Literal::Float(literal) => literal.cast.unwrap_or(literal.token),
            Literal::String(_, token) | Literal::Boolean(_, token) => token,
        }
    }

    pub fn token(&self) -> &'src TokenData<'src> {
        match self {
            Literal::Int(literal) => literal.token,
            Literal::Float(literal) => literal.token,
            Literal::String(_, token) | Literal::Boolean(_, token) => token,
        }
    }

    pub fn cast_target(&self) -> Option<&'src TokenData<'src>> {
        match self {
            Literal::Int(literal) => literal.cast,
            Literal::Float(literal) => literal.cast,
            Literal::String(_, _) | Literal::Boolean(_, _) => None,
        }
    }

    /// Returns the semantic type selected by an explicit cast. Uncast
    /// literals retain their broad `integer` or `number` literal type.
    pub fn semantic_type_name(&self) -> &'src str {
        match self {
            Literal::Int(literal) => literal.cast.map_or("integer", |target| target.v),
            Literal::Float(literal) => literal.cast.map_or("number", |target| target.v),
            Literal::String(_, _) => "string",
            Literal::Boolean(_, _) => "bool",
        }
    }

    pub fn source_text(&self) -> String {
        match self.cast_target() {
            Some(target) => format!("{} as {}", self.token().v, target.v),
            None => self.token().v.to_string(),
        }
    }

    pub fn parse(parser: &mut Parser<'src>, t: &'src Token<'src>) -> Option<Literal<'src>> {
        if t.0 == TokenVariant::Number {
            parser.step_forward();
            let cast = if parser.peek_is(TokenVariant::As) {
                parser.step_forward();
                Some(parser.expect_at_current(TokenVariant::Identifier)?)
            } else {
                None
            };
            return Self::parse_number(&t.1, cast)
                .map_err(|e| parser.diagnostics.push(e))
                .ok();
        }

        let literal = match t.0 {
            TokenVariant::True => Literal::Boolean(true, &t.1),
            TokenVariant::False => Literal::Boolean(false, &t.1),
            TokenVariant::String => Literal::String(t.1.v[1..t.1.v.len() - 1].to_string(), &t.1),
            _ => return None,
        };
        parser.step_forward();
        Some(literal)
    }

    fn parse_number(
        d: &'src TokenData<'src>,
        cast: Option<&'src TokenData<'src>>,
    ) -> Result<Literal<'src>, XenoDiagnostic<'src>> {
        let has_dot = d.v.contains('.');
        if has_dot {
            d.v.parse::<BigDecimal>()
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    let (precision, scale) = decimal_shape(d.v);
                    let inferred_size = infer_float_size(&value);
                    let size = match cast.map(|target| target.v) {
                        None | Some("number") => inferred_size,
                        Some("f32") => FloatSize::F32,
                        Some("f64") => FloatSize::F64,
                        Some("decimal") => FloatSize::Decimal,
                        Some(target) => {
                            return Err(format!(
                                "Cannot cast floating-point literal to '{target}'. Expected f32, f64, decimal, or number."
                            ));
                        }
                    };

                    if matches!(size, FloatSize::F32 | FloatSize::F64)
                        && !decimal_roundtrips_as(&value, size)
                    {
                        return Err(format!(
                            "Floating-point literal '{}' cannot be represented by {} without losing decimal precision.",
                            d.v,
                            float_size_name(size)
                        ));
                    }

                    Ok(Literal::Float(FloatLiteral {
                        value,
                        representation: FloatRepresentation {
                            signed: true,
                            precision,
                            scale,
                            size,
                        },
                        token: d,
                        cast,
                    }))
                })
        } else {
            d.v.parse::<BigInt>()
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    let inferred = infer_integer_representation(&value);
                    let representation = match cast.map(|target| target.v) {
                        None | Some("integer") => inferred,
                        Some("bigint") => IntegerRepresentation {
                            signed: true,
                            size: IntegerSize::Arbitrary,
                        },
                        Some(target) => parse_fixed_integer_representation(target)
                            .ok_or_else(|| {
                                format!(
                                    "Cannot cast integer literal to '{target}'. Expected i4..i128, u4..u128, bigint, or integer."
                                )
                            })?,
                    };

                    if !integer_fits(&value, representation) {
                        return Err(format!(
                            "Integer literal '{}' is outside the range of {}.",
                            d.v,
                            cast.map_or("the selected representation", |target| target.v)
                        ));
                    }

                    Ok(Literal::Int(IntLiteral {
                        value,
                        representation,
                        token: d,
                        cast,
                    }))
                })
        }
        .map_err(|e| XenoDiagnostic {
            location: cast.unwrap_or(d).clone(),
            message: format!("Error parsing number: {}", e),
            severity: XenoDiagSeverity::Err,
        })
    }
}

fn infer_integer_representation(value: &BigInt) -> IntegerRepresentation {
    let signed = value.sign() == num_bigint::Sign::Minus;
    let bits = if signed {
        ((-value) - BigInt::from(1_u8)).bits() + 1
    } else {
        value.bits().max(1)
    };
    IntegerRepresentation {
        signed,
        size: IntegerSize::Bits(bits),
    }
}

pub(crate) fn parse_fixed_integer_representation(target: &str) -> Option<IntegerRepresentation> {
    let (signed, bits) = match target.as_bytes().first() {
        Some(b'i') => (true, target.get(1..)?.parse::<u64>().ok()?),
        Some(b'u') => (false, target.get(1..)?.parse::<u64>().ok()?),
        _ => return None,
    };
    matches!(bits, 4 | 8 | 16 | 32 | 64 | 128).then_some(IntegerRepresentation {
        signed,
        size: IntegerSize::Bits(bits),
    })
}

pub fn integer_fits(value: &BigInt, representation: IntegerRepresentation) -> bool {
    let IntegerSize::Bits(bits) = representation.size else {
        return true;
    };
    if representation.signed {
        let half_range = BigInt::from(1_u8) << (bits - 1);
        value >= &-half_range.clone() && value < &half_range
    } else {
        value.sign() != num_bigint::Sign::Minus && value < &(BigInt::from(1_u8) << bits)
    }
}

fn decimal_shape(raw: &str) -> (u64, u64) {
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let significant = integer
        .chars()
        .chain(fraction.chars())
        .skip_while(|digit| *digit == '0')
        .count()
        .max(1) as u64;
    (significant, fraction.len() as u64)
}

pub fn decimal_roundtrips_as(value: &BigDecimal, size: FloatSize) -> bool {
    let rendered = match size {
        FloatSize::F32 => match value.to_string().parse::<f32>() {
            Ok(value) if value.is_finite() => value.to_string(),
            _ => return false,
        },
        FloatSize::F64 => match value.to_string().parse::<f64>() {
            Ok(value) if value.is_finite() => value.to_string(),
            _ => return false,
        },
        FloatSize::Decimal => return true,
    };
    BigDecimal::from_str(&rendered).is_ok_and(|roundtrip| roundtrip == *value)
}

fn infer_float_size(value: &BigDecimal) -> FloatSize {
    if decimal_roundtrips_as(value, FloatSize::F32) {
        FloatSize::F32
    } else if decimal_roundtrips_as(value, FloatSize::F64) {
        FloatSize::F64
    } else {
        FloatSize::Decimal
    }
}

fn float_size_name(size: FloatSize) -> &'static str {
    match size {
        FloatSize::F32 => "f32",
        FloatSize::F64 => "f64",
        FloatSize::Decimal => "decimal",
    }
}

impl<'src> Expr<'src> {
    pub fn get_last_token(&self) -> Option<&'src TokenData<'src>> {
        match self {
            Expr::Regex(d) => Some(*d),
            Expr::Annotation(a) => Some(a.get_last_token()),
            Expr::Type(t) => t.get_last_token(),
        }
    }

    pub fn parse(parser: &mut Parser<'src>) -> Option<Expr<'src>> {
        let t = parser.peek()?;
        match t.0 {
            TokenVariant::Regex => {
                parser.step_forward();
                Some(Expr::Regex(&t.1))
            }
            TokenVariant::At => Annotation::parse_annotation(parser).map(Expr::Annotation),
            _ => Type::parse(parser).map(Expr::Type),
        }
    }
}
