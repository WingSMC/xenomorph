use std::collections::{HashMap, HashSet};

use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend,
};
use xenomorph_common::{
    lexer::{Token, TokenVariant},
    parser::{Annotation, Declaration, Expr, Literal, SimpleType, Type, XenoType},
    TokenData,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
enum TokenKind {
    Namespace,
    Type,
    TypeParameter,
    Property,
    EnumMember,
    Keyword,
    Comment,
    String,
    Number,
    Regexp,
    Operator,
    Function,
}

const DECLARATION: u32 = 1 << 0;
const DOCUMENTATION: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TokenStyle {
    kind: TokenKind,
    modifiers: u32,
}

impl TokenStyle {
    const fn new(kind: TokenKind) -> Self {
        Self { kind, modifiers: 0 }
    }

    const fn declaration(kind: TokenKind) -> Self {
        Self {
            kind,
            modifiers: DECLARATION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TokenPosition {
    line: u32,
    column: u32,
}

impl From<&TokenData<'_>> for TokenPosition {
    fn from(data: &TokenData<'_>) -> Self {
        Self {
            line: data.l,
            column: data.c,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AbsoluteToken {
    line: u32,
    start: u32,
    length: u32,
    style: TokenStyle,
}

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::NAMESPACE,
            SemanticTokenType::TYPE,
            SemanticTokenType::TYPE_PARAMETER,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::ENUM_MEMBER,
            SemanticTokenType::KEYWORD,
            SemanticTokenType::COMMENT,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::REGEXP,
            SemanticTokenType::OPERATOR,
            SemanticTokenType::FUNCTION,
        ],
        token_modifiers: vec![
            SemanticTokenModifier::DECLARATION,
            SemanticTokenModifier::DOCUMENTATION,
        ],
    }
}

pub fn encode(source: &str, tokens: &[Token<'_>], ast: &[Declaration<'_>]) -> Vec<SemanticToken> {
    let roles = ast_roles(ast);
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut absolute = Vec::new();

    for (variant, data) in tokens {
        let style = roles
            .get(&TokenPosition::from(data))
            .copied()
            .or_else(|| lexical_style(*variant));
        let Some(style) = style else {
            continue;
        };

        append_token_segments(&mut absolute, &source_lines, data, style);
    }

    absolute.sort_by_key(|token| (token.line, token.start));
    encode_relative(absolute)
}

fn lexical_style(variant: TokenVariant) -> Option<TokenStyle> {
    let kind = match variant {
        TokenVariant::Identifier => TokenKind::Type,
        TokenVariant::Type
        | TokenVariant::Import
        | TokenVariant::Validator
        | TokenVariant::Set
        | TokenVariant::Enum
        | TokenVariant::As
        | TokenVariant::True
        | TokenVariant::False => TokenKind::Keyword,
        TokenVariant::Number => TokenKind::Number,
        TokenVariant::String => TokenKind::String,
        TokenVariant::Regex => TokenKind::Regexp,
        TokenVariant::Not
        | TokenVariant::Or
        | TokenVariant::And
        | TokenVariant::Dot
        | TokenVariant::Minus
        | TokenVariant::Question
        | TokenVariant::At
        | TokenVariant::Eq
        | TokenVariant::Neq
        | TokenVariant::Gt
        | TokenVariant::Lt => TokenKind::Operator,
        TokenVariant::Documentation => {
            return Some(TokenStyle {
                kind: TokenKind::Comment,
                modifiers: DOCUMENTATION,
            });
        }
        TokenVariant::Path => TokenKind::Namespace,
        TokenVariant::Comma
        | TokenVariant::Colon
        | TokenVariant::Semicolon
        | TokenVariant::LParen
        | TokenVariant::RParen
        | TokenVariant::LCurly
        | TokenVariant::RCurly
        | TokenVariant::LBracket
        | TokenVariant::RBracket => return None,
    };

    Some(TokenStyle::new(kind))
}

fn ast_roles(ast: &[Declaration<'_>]) -> HashMap<TokenPosition, TokenStyle> {
    let mut roles = HashMap::new();

    for declaration in ast {
        match declaration {
            Declaration::Import { .. } => {}
            Declaration::Type {
                name, generics, ty, ..
            } => {
                roles.insert(
                    TokenPosition::from(*name),
                    TokenStyle::declaration(TokenKind::Type),
                );

                let type_parameters: HashSet<&str> = generics
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|(parameter, _)| parameter.v)
                    .collect();
                if let Some(generics) = generics {
                    for (parameter, constraint) in generics {
                        roles.insert(
                            TokenPosition::from(*parameter),
                            TokenStyle::declaration(TokenKind::TypeParameter),
                        );
                        if let Some(constraint) = constraint {
                            mark_type_reference(&mut roles, constraint, &type_parameters);
                        }
                    }
                }
                visit_xeno_type(&mut roles, ty, &type_parameters);
            }
            Declaration::Custom { name, .. } => {
                if let Some(name) = name {
                    roles.insert(
                        TokenPosition::from(*name),
                        TokenStyle::declaration(TokenKind::Type),
                    );
                }
            }
        }
    }

    roles
}

fn visit_xeno_type(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    (ty, annotations): &XenoType<'_>,
    type_parameters: &HashSet<&str>,
) {
    visit_type(roles, ty, type_parameters);
    for annotation in annotations {
        visit_annotation(roles, annotation, type_parameters);
    }
}

fn visit_type(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    ty: &Type<'_>,
    type_parameters: &HashSet<&str>,
) {
    match ty {
        Type::Simple(simple) => visit_simple_type(roles, simple, type_parameters),
        Type::Tuple(types) | Type::Sum(types) | Type::Intersection(types) => {
            for ty in types {
                visit_simple_type(roles, ty, type_parameters);
            }
        }
        Type::Set(set) => {
            if let Some(element_type) = &set.element_type {
                visit_simple_type(roles, element_type, type_parameters);
            }
            for literal in set.values.as_deref().unwrap_or_default() {
                visit_literal(roles, literal, type_parameters);
            }
        }
        Type::Struct(fields) => {
            for (name, ty, _) in fields {
                roles.insert(
                    TokenPosition::from(name),
                    TokenStyle::declaration(TokenKind::Property),
                );
                visit_simple_type(roles, ty, type_parameters);
            }
        }
        Type::Enum(fields) => {
            for (name, ty, _) in fields {
                roles.insert(
                    TokenPosition::from(name),
                    TokenStyle::declaration(TokenKind::EnumMember),
                );
                visit_simple_type(roles, ty, type_parameters);
            }
        }
    }
}

fn visit_simple_type(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    ty: &SimpleType<'_>,
    type_parameters: &HashSet<&str>,
) {
    match ty {
        SimpleType::Optional(inner) => visit_simple_type(roles, inner, type_parameters),
        SimpleType::Literal(literal) => {
            visit_literal(roles, literal, type_parameters);
        }
        SimpleType::Identifier(name, arguments) | SimpleType::Array(name, arguments) => {
            mark_type_reference(roles, name, type_parameters);
            for argument in arguments.as_deref().unwrap_or_default() {
                visit_simple_type(roles, argument, type_parameters);
            }
        }
    }
}

fn visit_literal(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    literal: &Literal<'_>,
    type_parameters: &HashSet<&str>,
) {
    if let Some(cast_target) = literal.cast_target() {
        mark_type_reference(roles, cast_target, type_parameters);
    }
}

fn visit_annotation(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    annotation: &Annotation<'_>,
    type_parameters: &HashSet<&str>,
) {
    roles.insert(
        TokenPosition::from(annotation.ident),
        TokenStyle::new(TokenKind::Function),
    );
    for parameter in &annotation.params {
        visit_expr(roles, parameter, type_parameters);
    }
}

fn visit_expr(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    expr: &Expr<'_>,
    type_parameters: &HashSet<&str>,
) {
    match expr {
        Expr::Regex(_) => {}
        Expr::Annotation(annotation) => visit_annotation(roles, annotation, type_parameters),
        Expr::Type(ty) => visit_type(roles, ty, type_parameters),
    }
}

fn mark_type_reference(
    roles: &mut HashMap<TokenPosition, TokenStyle>,
    data: &TokenData<'_>,
    type_parameters: &HashSet<&str>,
) {
    let kind = if type_parameters.contains(data.v) {
        TokenKind::TypeParameter
    } else {
        TokenKind::Type
    };
    roles.insert(TokenPosition::from(data), TokenStyle::new(kind));
}

fn append_token_segments(
    result: &mut Vec<AbsoluteToken>,
    source_lines: &[&str],
    data: &TokenData<'_>,
    style: TokenStyle,
) {
    for (line_offset, raw_segment) in data.v.split('\n').enumerate() {
        let segment = raw_segment.strip_suffix('\r').unwrap_or(raw_segment);
        let length = segment.encode_utf16().count() as u32;
        if length == 0 {
            continue;
        }

        let line = data.l + line_offset as u32;
        let scalar_column = if line_offset == 0 { data.c } else { 0 };
        let start = source_lines
            .get(line as usize)
            .map_or(scalar_column, |source_line| {
                source_line
                    .chars()
                    .take(scalar_column as usize)
                    .map(char::len_utf16)
                    .sum::<usize>() as u32
            });
        result.push(AbsoluteToken {
            line,
            start,
            length,
            style,
        });
    }
}

fn encode_relative(tokens: Vec<AbsoluteToken>) -> Vec<SemanticToken> {
    let mut previous_line = 0;
    let mut previous_start = 0;

    tokens
        .into_iter()
        .map(|token| {
            let delta_line = token.line - previous_line;
            let delta_start = if delta_line == 0 {
                token.start - previous_start
            } else {
                token.start
            };
            previous_line = token.line;
            previous_start = token.start;

            SemanticToken {
                delta_line,
                delta_start,
                length: token.length,
                token_type: token.style.kind as u32,
                token_modifiers_bitset: token.style.modifiers,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::{lexer::Lexer, parser::Parser};

    fn classified(source: &str) -> Vec<(String, TokenStyle)> {
        let tokens = Lexer::tokenize(source).expect("semantic-token fixture should lex");
        let (ast, _) = Parser::parse(&tokens);
        let roles = ast_roles(&ast);

        tokens
            .iter()
            .filter_map(|(variant, data)| {
                roles
                    .get(&TokenPosition::from(data))
                    .copied()
                    .or_else(|| lexical_style(*variant))
                    .map(|style| (data.v.to_string(), style))
            })
            .collect()
    }

    #[test]
    fn syntax_roles_come_from_the_rust_tokens_and_ast() {
        let source = r#"/** docs */
import shared/types;
type Box<T: number> = {
    value: T,
};
type Choice = enum { yes: string };
type Checked = string @minlen(2) @match(/a+/);
type Cast = 1 as u64;
"#;
        let classified = classified(source);

        let contains = |text: &str, kind: TokenKind, modifiers: u32| {
            classified
                .iter()
                .any(|token| token == &(text.to_string(), TokenStyle { kind, modifiers }))
        };

        assert!(contains("/** docs */", TokenKind::Comment, DOCUMENTATION));
        assert!(contains("import", TokenKind::Keyword, 0));
        assert!(contains("shared/types", TokenKind::Namespace, 0));
        assert!(contains("Box", TokenKind::Type, DECLARATION));
        assert!(contains("T", TokenKind::TypeParameter, DECLARATION));
        assert!(contains("T", TokenKind::TypeParameter, 0));
        assert!(contains("value", TokenKind::Property, DECLARATION));
        assert!(contains("yes", TokenKind::EnumMember, DECLARATION));
        assert!(contains("minlen", TokenKind::Function, 0));
        assert!(contains("/a+/", TokenKind::Regexp, 0));
        assert!(contains("2", TokenKind::Number, 0));
        assert!(contains("as", TokenKind::Keyword, 0));
        assert!(contains("u64", TokenKind::Type, 0));
    }

    #[test]
    fn lexer_roles_still_highlight_incomplete_syntax() {
        let classified = classified("type Broken = Unknown");

        assert!(classified.contains(&("type".to_string(), TokenStyle::new(TokenKind::Keyword))));
        assert!(classified.contains(&("Broken".to_string(), TokenStyle::new(TokenKind::Type))));
        assert!(classified.contains(&("Unknown".to_string(), TokenStyle::new(TokenKind::Type))));
    }

    #[test]
    fn encoded_positions_and_lengths_use_utf16_units() {
        let source = "type A = \"😀\" @check;";
        let tokens = Lexer::tokenize(source).expect("UTF-16 fixture should lex");
        let (ast, _) = Parser::parse(&tokens);
        let encoded = encode(source, &tokens, &ast);

        let mut line = 0;
        let mut start = 0;
        let absolute: Vec<_> = encoded
            .iter()
            .map(|token| {
                line += token.delta_line;
                start = if token.delta_line == 0 {
                    start + token.delta_start
                } else {
                    token.delta_start
                };
                (line, start, token.length, token.token_type)
            })
            .collect();

        let string_start = source.find('"').unwrap();
        let string_start_utf16 = source[..string_start].encode_utf16().count() as u32;
        assert!(absolute.contains(&(
            0,
            string_start_utf16,
            "\"😀\"".encode_utf16().count() as u32,
            TokenKind::String as u32,
        )));

        let decorator_start = source.find("check").unwrap();
        let decorator_start_utf16 = source[..decorator_start].encode_utf16().count() as u32;
        assert!(absolute.contains(&(0, decorator_start_utf16, 5, TokenKind::Function as u32,)));
    }

    #[test]
    fn multiline_tokens_are_split_into_legal_lsp_tokens() {
        let source = "type A = \"first\nsecond\";";
        let tokens = Lexer::tokenize(source).expect("multiline fixture should lex");
        let (ast, _) = Parser::parse(&tokens);
        let encoded = encode(source, &tokens, &ast);

        let string_tokens: Vec<_> = encoded
            .iter()
            .filter(|token| token.token_type == TokenKind::String as u32)
            .collect();
        assert_eq!(string_tokens.len(), 2);
        assert_eq!(string_tokens[0].length, 6);
        assert_eq!(string_tokens[1].delta_line, 1);
        assert_eq!(string_tokens[1].delta_start, 0);
        assert_eq!(string_tokens[1].length, 7);
    }
}
