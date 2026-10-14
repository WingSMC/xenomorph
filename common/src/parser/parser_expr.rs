use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use std::str::FromStr;

use crate::{
    parser::Parser, utils::extract_documentation, TokenData, XenoDiagSeverity, XenoDiagnostic,
};

pub use xenomorph_plugin_api::syntax::*;

pub trait AnnotationMethods<'src>: Sized {
    fn parse_annotations(parser: &mut Parser<'src>) -> Vec<Annotation>;
    fn parse_annotation(parser: &mut Parser<'src>) -> Option<Annotation>;
}
impl<'src> AnnotationMethods<'src> for Annotation {
    fn parse_annotations(parser: &mut Parser<'src>) -> Vec<Annotation> {
        let mut annotations = Vec::new();
        while let Some((TokenVariant::At, _)) = parser.peek() {
            if let Some(a) = Self::parse_annotation(parser) {
                annotations.push(a)
            }
        }
        annotations
    }
    fn parse_annotation(parser: &mut Parser<'src>) -> Option<Annotation> {
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
pub trait DeclarationMethods<'src>: Sized {
    fn parse(parser: &mut Parser<'src>) -> Option<Declaration>;
    fn parse_type_declaration(
        parser: &mut Parser<'src>,
        from: TokenData,
        docs: Option<String>,
    ) -> Option<Declaration>;
    fn parse_import_declaration(
        parser: &mut Parser<'src>,
        location: TokenData,
        docs: Option<String>,
    ) -> Option<Declaration>;
}
impl<'src> DeclarationMethods<'src> for Declaration {
    fn parse(parser: &mut Parser<'src>) -> Option<Declaration> {
        let token = parser.need_next()?;
        let docs = match token {
            (TokenVariant::Documentation, data) => {
                Some(extract_documentation(data, parser.source).to_string())
            }
            _ => None,
        };

        let (var, d) = if docs.is_some() {
            parser.need_next()?
        } else {
            token
        };

        let public = if *var == TokenVariant::Pub {
            Some(*d)
        } else {
            None
        };
        let (var, d) = if public.is_some() {
            let (var, d) = parser.need_next()?;
            (var, d)
        } else {
            (var, d)
        };
        let mut dec = match var {
            TokenVariant::Type => Declaration::parse_type_declaration(parser, *d, docs),
            TokenVariant::Import => Declaration::parse_import_declaration(parser, *d, docs),
            _ => {
                parser.diagnostics.push(XenoDiagnostic {
                    tags: vec![],
                    location: *d,
                    message: format!("Unknown declaration at {}, ({:?}).", d, var),
                    severity: XenoDiagSeverity::Err,
                });
                return None;
            }
        }?;

        dec.set_public(public);

        parser.expect(TokenVariant::Semicolon)?;

        Some(dec)
    }
    fn parse_type_declaration(
        parser: &mut Parser<'src>,
        from: TokenData,
        docs: Option<String>,
    ) -> Option<Declaration> {
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

                generics.push((*d, constraint));
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
            public: None,
            docs,
            name,
            generics,
            ty: t,
            from,
            to,
        })
    }
    fn parse_import_declaration(
        parser: &mut Parser<'src>,
        location: TokenData,
        docs: Option<String>,
    ) -> Option<Declaration> {
        if docs.is_some() {
            parser.diagnostics.push(XenoDiagnostic {
                tags: vec![],
                location,
                message: "Import declarations cannot have documentation comments.".to_string(),
                severity: XenoDiagSeverity::Info,
            });
        }

        let mut path_tokens = vec![parser.expect(TokenVariant::Identifier)?];
        let names;
        loop {
            parser.expect(TokenVariant::PathSeparator)?;
            if matches!(parser.peek(), Some((TokenVariant::LCurly, _))) {
                names = parser.parse_list(
                    TokenVariant::LCurly,
                    TokenVariant::Comma,
                    Some(TokenVariant::RCurly),
                    |p| p.expect(TokenVariant::Identifier),
                )?;
                if names.is_empty() {
                    parser.diagnostics.push(XenoDiagnostic {
                        tags: vec![],
                        location,
                        message: "An import must select at least one declaration".into(),
                        severity: XenoDiagSeverity::Err,
                    });
                }
                break;
            }
            let ident = parser.expect(TokenVariant::Identifier)?;
            if matches!(parser.peek(), Some((TokenVariant::PathSeparator, _))) {
                path_tokens.push(ident);
            } else {
                names = vec![ident];
                break;
            }
        }
        let path = path_tokens
            .iter()
            .map(|token| token.text(parser.source).to_string())
            .collect();
        let end = parser
            .peek()
            .map(|(_, token)| token.start)
            .unwrap_or(location.end);
        Some(Declaration::Import {
            public: None,
            path,
            path_tokens,
            names,
            location: TokenData {
                start: location.start,
                end,
            },
        })
    }
}
pub trait TypeMethods<'src>: Sized {
    fn parse(parser: &mut Parser<'src>) -> Option<Type>;
    fn parse_set(parser: &mut Parser<'src>) -> Option<Type>;
    fn parse_set_value(parser: &mut Parser<'src>) -> Option<Literal>;
    fn parse_tuple(parser: &mut Parser<'src>) -> Option<Vec<SimpleType>>;
    fn parse_enum(parser: &mut Parser<'src>) -> Option<Type>;
    fn parse_struct(parser: &mut Parser<'src>) -> Option<Vec<KeyValExpr>>;
}
impl<'src> TypeMethods<'src> for Type {
    fn parse(parser: &mut Parser<'src>) -> Option<Type> {
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
    fn parse_set(parser: &mut Parser<'src>) -> Option<Type> {
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
                tags: vec![],
                location: keyword,
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
    fn parse_set_value(parser: &mut Parser<'src>) -> Option<Literal> {
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
                tags: vec![],
                location: token.1,
                message: "Set prefills can contain only literals.".to_string(),
                severity: XenoDiagSeverity::Err,
            });
        }
        literal
    }
    fn parse_tuple(parser: &mut Parser<'src>) -> Option<Vec<SimpleType>> {
        parser.parse_list(
            TokenVariant::LBracket,
            TokenVariant::Comma,
            Some(TokenVariant::RBracket),
            SimpleType::parse,
        )
    }
    fn parse_enum(parser: &mut Parser<'src>) -> Option<Type> {
        parser.expect(TokenVariant::Enum)?;

        let fs = Self::parse_struct(parser)?;
        Some(Type::Enum(fs))
    }
    fn parse_struct(parser: &mut Parser<'src>) -> Option<Vec<KeyValExpr>> {
        parser.expect(TokenVariant::LCurly)?;

        let mut fields = Vec::new();
        while !parser.peek_is(TokenVariant::RCurly) && parser.peek().is_some() {
            let docs = if let Some((TokenVariant::Documentation, d)) = parser.peek() {
                parser.step_forward();
                Some(*d)
            } else {
                None
            };

            if parser.peek_is(TokenVariant::RCurly) {
                if let Some(docs) = docs {
                    parser.diagnostics.push(XenoDiagnostic {
                        tags: vec![],
                        location: docs,
                        message: "Documentation comment without a field.".to_string(),
                        severity: XenoDiagSeverity::Warn,
                    });
                }
                break;
            }

            let field = 'field: {
                let key = match parser.peek() {
                    Some((TokenVariant::Identifier, key)) => *key,
                    Some((TokenVariant::String, key)) => TokenData {
                        start: key.start + 1,
                        end: key.end - 1,
                    },
                    _ => {
                        let _ = parser.expect_at_current(TokenVariant::Identifier);
                        break 'field None;
                    }
                };
                parser.step_forward();
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
pub trait SimpleTypeMethods<'src>: Sized {
    fn parse(parser: &mut Parser<'src>) -> Option<SimpleType>;
    fn parse_base(parser: &mut Parser<'src>) -> Option<SimpleType>;
}
impl<'src> SimpleTypeMethods<'src> for SimpleType {
    fn parse(parser: &mut Parser<'src>) -> Option<SimpleType> {
        let is_optional = parser.skip_if(TokenVariant::Question);
        let base = Self::parse_base(parser)?;
        Some(match is_optional {
            true => SimpleType::Optional(Box::new(base)),
            false => base,
        })
    }
    fn parse_base(parser: &mut Parser<'src>) -> Option<SimpleType> {
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
                    Some(SimpleType::Array(t.1, arguments))
                } else {
                    Some(SimpleType::Identifier(t.1, arguments))
                }
            }

            _ => Literal::parse(parser, t)
                .map(SimpleType::Literal)
                .or_else(|| {
                    parser.diagnostics.push(XenoDiagnostic {
                        tags: vec![],
                        location: t.1,
                        message: format!("Unexpected token {:?} in simple type expression.", t.0),
                        severity: XenoDiagSeverity::Err,
                    });
                    None
                }),
        }
    }
}
pub trait LiteralMethods<'src>: Sized {
    fn parse(parser: &mut Parser<'src>, t: &'src Token) -> Option<Literal>;
    fn parse_number(
        d: TokenData,
        cast: Option<TokenData>,
        source: &str,
    ) -> Result<Literal, XenoDiagnostic>;
}
impl<'src> LiteralMethods<'src> for Literal {
    fn parse(parser: &mut Parser<'src>, t: &'src Token) -> Option<Literal> {
        if t.0 == TokenVariant::Number {
            parser.step_forward();
            let cast = if parser.peek_is(TokenVariant::As) {
                parser.step_forward();
                Some(parser.expect_at_current(TokenVariant::Identifier)?)
            } else {
                None
            };
            return Self::parse_number(t.1, cast, parser.source)
                .map_err(|e| parser.diagnostics.push(e))
                .ok();
        }

        let literal = match t.0 {
            TokenVariant::True => Literal::Boolean(true, t.1),
            TokenVariant::False => Literal::Boolean(false, t.1),
            TokenVariant::String => Literal::String(
                t.1.text(parser.source)[1..t.1.text(parser.source).len() - 1].to_string(),
                t.1,
            ),
            _ => return None,
        };
        parser.step_forward();
        Some(literal)
    }
    fn parse_number(
        d: TokenData,
        cast: Option<TokenData>,
        source: &str,
    ) -> Result<Literal, XenoDiagnostic> {
        let has_dot = d.text(source).contains('.');
        if has_dot {
            d.text(source).parse::<BigDecimal>()
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    let (precision, scale) = decimal_shape(d.text(source));
                    let inferred_size = infer_float_size(&value);
                    let size = match cast.map(|target| target.text(source)) {
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
                            d.text(source),
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
            d.text(source).parse::<BigInt>()
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    let inferred = infer_integer_representation(&value);
                    let representation = match cast.map(|target| target.text(source)) {
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
                            d.text(source),
                            cast.map_or("the selected representation", |target| target.text(source))
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
            tags: vec![],
            location: cast.unwrap_or(d),
            message: format!("Error parsing number: {}", e),
            severity: XenoDiagSeverity::Err,
        })
    }
}
pub trait ExprMethods<'src>: Sized {
    fn parse(parser: &mut Parser<'src>) -> Option<Expr>;
}
impl<'src> ExprMethods<'src> for Expr {
    fn parse(parser: &mut Parser<'src>) -> Option<Expr> {
        let t = parser.peek()?;
        match t.0 {
            TokenVariant::Regex => {
                parser.step_forward();
                Some(Expr::Regex(t.1))
            }
            TokenVariant::At => Annotation::parse_annotation(parser).map(Expr::Annotation),
            _ => Type::parse(parser).map(Expr::Type),
        }
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
    if !representation.signed && value.sign() == num_bigint::Sign::Minus {
        return false;
    }
    match representation.size {
        IntegerSize::Arbitrary => true,
        IntegerSize::Bits(0) => false,
        IntegerSize::Bits(bits) => {
            let required = if representation.signed {
                if value.sign() == num_bigint::Sign::Minus {
                    ((-value) - BigInt::from(1u8)).bits() + 1
                } else {
                    value.bits() + 1
                }
            } else {
                value.bits()
            };
            required <= bits
        }
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
