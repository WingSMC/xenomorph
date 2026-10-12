use crate::{parser as ast, TokenData};
use xenomorph_plugin_api::syntax as wire;

pub fn encode(declarations: &[ast::Declaration<'_>]) -> Result<wire::Syntax, String> {
    encode_with_tokens(declarations).map(|(syntax, _)| syntax)
}

pub fn encode_with_tokens<'src>(
    declarations: &[ast::Declaration<'src>],
) -> Result<(wire::Syntax, Vec<TokenData<'src>>), String> {
    let mut encoder = Encoder {
        tokens: Vec::new(),
        source_tokens: Vec::new(),
    };
    let declarations = declarations
        .iter()
        .map(|declaration| encoder.declaration(declaration))
        .collect::<Result<_, _>>()?;
    Ok((
        wire::Syntax {
            tokens: encoder.tokens,
            declarations,
        },
        encoder.source_tokens,
    ))
}

struct Encoder<'src> {
    tokens: Vec<wire::Token>,
    source_tokens: Vec<TokenData<'src>>,
}

impl<'src> Encoder<'src> {
    fn token(&mut self, token: &TokenData<'src>) -> usize {
        let index = self.tokens.len();
        self.source_tokens.push(token.clone());
        self.tokens.push(wire::Token {
            value: token.v.to_string(),
            line: token.l,
            column: token.c,
        });
        index
    }

    fn declaration(
        &mut self,
        declaration: &ast::Declaration<'src>,
    ) -> Result<wire::Declaration, String> {
        Ok(match declaration {
            ast::Declaration::Import { path, location } => wire::Declaration::Import {
                path: path.iter().map(|part| part.to_string()).collect(),
                location: self.token(location),
            },
            ast::Declaration::Type { docs, name, generics, ty, from, to } => wire::Declaration::Type {
                docs: docs.map(str::to_string),
                name: self.token(name),
                generics: generics.as_ref().map(|parameters| parameters.iter().map(|(name, constraint)| {
                    (self.token(name), constraint.map(|token| self.token(token)))
                }).collect()),
                ty: self.ty(&ty.0),
                annotations: ty.1.iter().map(|annotation| self.annotation(annotation)).collect(),
                from: self.token(from),
                to: self.token(to),
            },
            ast::Declaration::Custom { .. } => return Err(
                "Custom declarations require a versioned plugin payload; opaque Rust values cannot cross the plugin ABI.".into()
            ),
        })
    }

    fn annotation(&mut self, annotation: &ast::Annotation<'src>) -> wire::Annotation {
        wire::Annotation {
            ident: self.token(annotation.ident),
            params: annotation
                .params
                .iter()
                .map(|expr| match expr {
                    ast::Expr::Regex(token) => wire::Expr::Regex(self.token(token)),
                    ast::Expr::Annotation(annotation) => {
                        wire::Expr::Annotation(self.annotation(annotation))
                    }
                    ast::Expr::Type(ty) => wire::Expr::Type(self.ty(ty)),
                })
                .collect(),
        }
    }

    fn fields(&mut self, fields: &[ast::KeyValExpr<'src>]) -> Vec<wire::Field> {
        fields
            .iter()
            .map(|(key, value, docs)| {
                (
                    self.token(key),
                    self.simple(value),
                    docs.map(|token| self.token(token)),
                )
            })
            .collect()
    }

    fn ty(&mut self, ty: &ast::Type<'src>) -> wire::Type {
        match ty {
            ast::Type::Simple(simple) => wire::Type::Simple(self.simple(simple)),
            ast::Type::Tuple(items) => {
                wire::Type::Tuple(items.iter().map(|item| self.simple(item)).collect())
            }
            ast::Type::Sum(items) => {
                wire::Type::Sum(items.iter().map(|item| self.simple(item)).collect())
            }
            ast::Type::Intersection(items) => {
                wire::Type::Intersection(items.iter().map(|item| self.simple(item)).collect())
            }
            ast::Type::Struct(fields) => wire::Type::Struct(self.fields(fields)),
            ast::Type::Enum(fields) => wire::Type::Enum(self.fields(fields)),
            ast::Type::Set(set) => wire::Type::Set {
                keyword: self.token(set.keyword),
                element_type: set.element_type.as_ref().map(|item| self.simple(item)),
                values: set
                    .values
                    .as_ref()
                    .map(|items| items.iter().map(|item| self.literal(item)).collect()),
                last_token: self.token(set.last_token),
            },
        }
    }

    fn simple(&mut self, simple: &ast::SimpleType<'src>) -> wire::Simple {
        match simple {
            ast::SimpleType::Literal(literal) => wire::Simple::Literal(self.literal(literal)),
            ast::SimpleType::Identifier(token, arguments) => wire::Simple::Identifier(
                self.token(token),
                arguments
                    .as_ref()
                    .map(|items| items.iter().map(|item| self.simple(item)).collect()),
            ),
            ast::SimpleType::Array(token, arguments) => wire::Simple::Array(
                self.token(token),
                arguments
                    .as_ref()
                    .map(|items| items.iter().map(|item| self.simple(item)).collect()),
            ),
            ast::SimpleType::Optional(inner) => {
                wire::Simple::Optional(Box::new(self.simple(inner)))
            }
        }
    }

    fn literal(&mut self, literal: &ast::Literal<'src>) -> wire::Literal {
        match literal {
            ast::Literal::Int(integer) => wire::Literal::Integer {
                value: integer.value.to_string(),
                signed: integer.representation.signed,
                bits: match integer.representation.size {
                    ast::IntegerSize::Bits(bits) => Some(bits),
                    ast::IntegerSize::Arbitrary => None,
                },
                token: self.token(integer.token),
                cast: integer.cast.map(|token| self.token(token)),
            },
            ast::Literal::Float(float) => wire::Literal::Float {
                value: float.value.to_string(),
                signed: float.representation.signed,
                precision: float.representation.precision,
                scale: float.representation.scale,
                size: match float.representation.size {
                    ast::FloatSize::F32 => wire::FloatSize::F32,
                    ast::FloatSize::F64 => wire::FloatSize::F64,
                    ast::FloatSize::Decimal => wire::FloatSize::Decimal,
                },
                token: self.token(float.token),
                cast: float.cast.map(|token| self.token(token)),
            },
            ast::Literal::String(value, token) => {
                wire::Literal::String(value.clone(), self.token(token))
            }
            ast::Literal::Boolean(value, token) => {
                wire::Literal::Boolean(*value, self.token(token))
            }
        }
    }
}

pub fn tokens(syntax: &wire::Syntax) -> Vec<TokenData<'_>> {
    syntax
        .tokens
        .iter()
        .map(|token| TokenData {
            v: &token.value,
            l: token.line,
            c: token.column,
        })
        .collect()
}

pub fn decode<'a>(
    syntax: &'a wire::Syntax,
    tokens: &'a [TokenData<'a>],
) -> Result<Vec<ast::Declaration<'a>>, String> {
    let decoder = Decoder { tokens };
    syntax
        .declarations
        .iter()
        .map(|declaration| decoder.declaration(declaration))
        .collect()
}

struct Decoder<'a> {
    tokens: &'a [TokenData<'a>],
}

impl<'a> Decoder<'a> {
    fn token(&self, index: usize) -> Result<&'a TokenData<'a>, String> {
        self.tokens
            .get(index)
            .ok_or_else(|| format!("Invalid plugin syntax token index: {index}"))
    }

    fn declaration(
        &self,
        declaration: &'a wire::Declaration,
    ) -> Result<ast::Declaration<'a>, String> {
        Ok(match declaration {
            wire::Declaration::Import { path, location } => ast::Declaration::Import {
                path: path.iter().map(String::as_str).collect(),
                location: self.token(*location)?,
            },
            wire::Declaration::Type {
                docs,
                name,
                generics,
                ty,
                annotations,
                from,
                to,
            } => ast::Declaration::Type {
                docs: docs.as_deref(),
                name: self.token(*name)?,
                generics: generics
                    .as_ref()
                    .map(|items| {
                        items
                            .iter()
                            .map(|(name, constraint)| {
                                Ok((
                                    self.token(*name)?,
                                    constraint.map(|index| self.token(index)).transpose()?,
                                ))
                            })
                            .collect::<Result<_, String>>()
                    })
                    .transpose()?,
                ty: (
                    self.ty(ty)?,
                    annotations
                        .iter()
                        .map(|annotation| self.annotation(annotation))
                        .collect::<Result<_, _>>()?,
                ),
                from: self.token(*from)?,
                to: self.token(*to)?,
            },
        })
    }

    fn annotation(&self, annotation: &'a wire::Annotation) -> Result<ast::Annotation<'a>, String> {
        Ok(ast::Annotation {
            ident: self.token(annotation.ident)?,
            params: annotation
                .params
                .iter()
                .map(|expr| {
                    Ok(match expr {
                        wire::Expr::Regex(index) => ast::Expr::Regex(self.token(*index)?),
                        wire::Expr::Annotation(annotation) => {
                            ast::Expr::Annotation(self.annotation(annotation)?)
                        }
                        wire::Expr::Type(ty) => ast::Expr::Type(self.ty(ty)?),
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }

    fn fields(&self, fields: &'a [wire::Field]) -> Result<Vec<ast::KeyValExpr<'a>>, String> {
        fields
            .iter()
            .map(|(key, value, docs)| {
                Ok((
                    self.token(*key)?.clone(),
                    self.simple(value)?,
                    docs.map(|index| self.token(index)).transpose()?,
                ))
            })
            .collect()
    }

    fn ty(&self, ty: &'a wire::Type) -> Result<ast::Type<'a>, String> {
        Ok(match ty {
            wire::Type::Simple(simple) => ast::Type::Simple(self.simple(simple)?),
            wire::Type::Tuple(items) => ast::Type::Tuple(self.simples(items)?),
            wire::Type::Sum(items) => ast::Type::Sum(self.simples(items)?),
            wire::Type::Intersection(items) => ast::Type::Intersection(self.simples(items)?),
            wire::Type::Struct(fields) => ast::Type::Struct(self.fields(fields)?),
            wire::Type::Enum(fields) => ast::Type::Enum(self.fields(fields)?),
            wire::Type::Set {
                keyword,
                element_type,
                values,
                last_token,
            } => ast::Type::Set(ast::SetType {
                keyword: self.token(*keyword)?,
                element_type: element_type
                    .as_ref()
                    .map(|item| self.simple(item))
                    .transpose()?,
                values: values
                    .as_ref()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| self.literal(item))
                            .collect::<Result<_, _>>()
                    })
                    .transpose()?,
                last_token: self.token(*last_token)?,
            }),
        })
    }

    fn simples(&self, items: &'a [wire::Simple]) -> Result<Vec<ast::SimpleType<'a>>, String> {
        items.iter().map(|item| self.simple(item)).collect()
    }

    fn simple(&self, simple: &'a wire::Simple) -> Result<ast::SimpleType<'a>, String> {
        Ok(match simple {
            wire::Simple::Literal(literal) => ast::SimpleType::Literal(self.literal(literal)?),
            wire::Simple::Identifier(token, arguments) => ast::SimpleType::Identifier(
                self.token(*token)?,
                arguments
                    .as_ref()
                    .map(|items| self.simples(items))
                    .transpose()?,
            ),
            wire::Simple::Array(token, arguments) => ast::SimpleType::Array(
                self.token(*token)?,
                arguments
                    .as_ref()
                    .map(|items| self.simples(items))
                    .transpose()?,
            ),
            wire::Simple::Optional(inner) => {
                ast::SimpleType::Optional(Box::new(self.simple(inner)?))
            }
        })
    }

    fn literal(&self, literal: &'a wire::Literal) -> Result<ast::Literal<'a>, String> {
        Ok(match literal {
            wire::Literal::Integer {
                value,
                signed,
                bits,
                token,
                cast,
            } => ast::Literal::Int(ast::IntLiteral {
                value: value
                    .parse()
                    .map_err(|error| format!("Invalid integer payload: {error}"))?,
                representation: ast::IntegerRepresentation {
                    signed: *signed,
                    size: bits
                        .map(ast::IntegerSize::Bits)
                        .unwrap_or(ast::IntegerSize::Arbitrary),
                },
                token: self.token(*token)?,
                cast: cast.map(|index| self.token(index)).transpose()?,
            }),
            wire::Literal::Float {
                value,
                signed,
                precision,
                scale,
                size,
                token,
                cast,
            } => ast::Literal::Float(ast::FloatLiteral {
                value: value
                    .parse()
                    .map_err(|error| format!("Invalid decimal payload: {error}"))?,
                representation: ast::FloatRepresentation {
                    signed: *signed,
                    precision: *precision,
                    scale: *scale,
                    size: match size {
                        wire::FloatSize::F32 => ast::FloatSize::F32,
                        wire::FloatSize::F64 => ast::FloatSize::F64,
                        wire::FloatSize::Decimal => ast::FloatSize::Decimal,
                    },
                },
                token: self.token(*token)?,
                cast: cast.map(|index| self.token(index)).transpose()?,
            }),
            wire::Literal::String(value, token) => {
                ast::Literal::String(value.clone(), self.token(*token)?)
            }
            wire::Literal::Boolean(value, token) => {
                ast::Literal::Boolean(*value, self.token(*token)?)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::Lexer, parser::Parser};

    #[test]
    fn lossless_syntax_roundtrip() {
        let source = "import other;\n/// Docs\ntype Example<T: string> = { \"quoted key\": ?T, huge: 123456789012345678901234567890, decimal: 1.234567890123456789 } @Lombok(Data);";
        let tokens = Lexer::tokenize(source).unwrap();
        let (ast, errors) = Parser::parse(&tokens);
        assert!(errors.is_empty(), "{errors:?}");
        let encoded = encode(&ast).unwrap();
        let json = serde_json::to_vec(&encoded).unwrap();
        let wire = serde_json::from_slice(&json).unwrap();
        let decoded_tokens = super::tokens(&wire);
        let decoded = decode(&wire, &decoded_tokens).unwrap();
        assert_eq!(format!("{ast:?}"), format!("{decoded:?}"));
    }

    #[test]
    fn rejects_invalid_token_reference() {
        let wire = wire::Syntax {
            tokens: Vec::new(),
            declarations: vec![wire::Declaration::Import {
                path: vec![],
                location: 1,
            }],
        };
        assert!(decode(&wire, &[]).is_err());
    }
}
