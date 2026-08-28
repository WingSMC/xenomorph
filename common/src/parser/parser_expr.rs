use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use std::fmt;

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
            Self::parse_annotation(parser).map(|a| annotations.push(a));
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
    A set which can contain any number of types (or literals)
    where types must be partially ordered to ensure uniqueness,
    try to not rely on ordering.
    The inferred set type will be the lowest common supertype of all the types in the set.

    e.g. set ["Blue", "Red", "Green"] // if this is translated to TS this will become a Set<string>
    */
    Set(Vec<SimpleType<'src>>),

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

/** Used for struct fields, sets, tuples */
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleType<'src> {
    /** e.g. 42, 3.141592653, true, "hello" */
    Literal(Literal<'src>),
    OptionalLiteral(Literal<'src>),

    /** name of a type */
    Identifier(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),
    OptionalIdentifier(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),

    /** Postfix array syntax, e.g. uint32[] */
    Array(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),
    OptionalArray(&'src TokenData<'src>, Option<Vec<SimpleType<'src>>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal<'src> {
    Int(BigInt, &'src TokenData<'src>),
    Float(BigDecimal, &'src TokenData<'src>),
    String(String, &'src TokenData<'src>),
    Boolean(bool, &'src TokenData<'src>),
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
    /// e.g. uint32, "asd", [Type1, Type2], { field1: Type1, field2: Type2 }
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
            Type::Tuple(ts) | Type::Set(ts) | Type::Intersection(ts) | Type::Sum(ts) => {
                ts.last().map(|t| t.get_last_token())
            }
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
        parser.expect(TokenVariant::Set)?;

        let fs = Self::parse_tuple(parser)?;
        Some(Type::Set(fs))
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
    pub fn get_last_token(&self) -> &'src TokenData<'src> {
        match self {
            SimpleType::Literal(l) | SimpleType::OptionalLiteral(l) => l.get_last_token(),
            SimpleType::Identifier(d, arguments)
            | SimpleType::OptionalIdentifier(d, arguments)
            | SimpleType::Array(d, arguments)
            | SimpleType::OptionalArray(d, arguments) => arguments
                .as_deref()
                .and_then(|arguments| arguments.last())
                .map(SimpleType::get_last_token)
                .unwrap_or(d),
        }
    }

    pub fn parse(parser: &mut Parser<'src>) -> Option<SimpleType<'src>> {
        let is_optional = parser.skip_if(TokenVariant::Question);

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
                    return if is_optional {
                        Some(SimpleType::OptionalArray(&t.1, arguments))
                    } else {
                        Some(SimpleType::Array(&t.1, arguments))
                    };
                }

                if is_optional {
                    Some(SimpleType::OptionalIdentifier(&t.1, arguments))
                } else {
                    Some(SimpleType::Identifier(&t.1, arguments))
                }
            }

            _ => Literal::parse(parser, &t)
                .map(|l| {
                    if is_optional {
                        SimpleType::OptionalLiteral(l)
                    } else {
                        SimpleType::Literal(l)
                    }
                })
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
            Literal::Int(_, td)
            | Literal::Float(_, td)
            | Literal::String(_, td)
            | Literal::Boolean(_, td) => *td,
        }
    }

    pub fn parse(parser: &mut Parser<'src>, t: &'src Token<'src>) -> Option<Literal<'src>> {
        let res = match t.0 {
            TokenVariant::True => Literal::Boolean(true, &t.1),
            TokenVariant::False => Literal::Boolean(false, &t.1),
            TokenVariant::String => Literal::String(t.1.v[1..t.1.v.len() - 1].to_string(), &t.1),
            TokenVariant::Number => Self::parse_number(&t.1)
                .map_err(|e| parser.diagnostics.push(e))
                .ok()?,
            _ => return None,
        };
        parser.step_forward();
        Some(res)
    }

    fn parse_number(d: &'src TokenData<'src>) -> Result<Literal<'src>, XenoDiagnostic<'src>> {
        let has_dot = d.v.contains('.');
        if has_dot {
            d.v.parse::<BigDecimal>()
                .map(|num| Literal::Float(num, d))
                .map_err(|e| e.to_string())
        } else {
            d.v.parse::<BigInt>()
                .map(|num| Literal::Int(num, d))
                .map_err(|e| e.to_string())
        }
        .map_err(|e| XenoDiagnostic {
            location: d.clone(),
            message: format!("Error parsing number: {}", e),
            severity: XenoDiagSeverity::Err,
        })
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
