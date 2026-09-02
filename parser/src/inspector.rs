use serde::Serialize;
use std::io::{self, Read};
use xenomorph_common::lexer::{Lexer, Token, TokenVariant};
use xenomorph_common::parser::{
    Annotation, Declaration, Expr, FloatSize, IntegerSize, Literal, Parser, SimpleType, Type,
};
use xenomorph_common::{TokenData, XenoDiagSeverity, XenoDiagnostic};

#[derive(Clone, Copy, Debug, Serialize)]
struct InspectPosition {
    line: u32,
    character: u32,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct InspectRange {
    start: InspectPosition,
    end: InspectPosition,
}

#[derive(Debug, Serialize)]
struct InspectToken<'src> {
    kind: String,
    lexeme: &'src str,
    range: InspectRange,
}

#[derive(Debug, Serialize)]
struct InspectDiagnostic {
    severity: &'static str,
    message: String,
    range: InspectRange,
}

#[derive(Debug, Serialize)]
struct AstNode {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<InspectRange>,
    children: Vec<AstNode>,
}

#[derive(Debug, Serialize)]
struct InspectResult<'src> {
    tokens: Vec<InspectToken<'src>>,
    ast: Vec<AstNode>,
    diagnostics: Vec<InspectDiagnostic>,
}

/// Reads standalone Xenomorph source from stdin and writes one JSON object to
/// stdout. Inspection intentionally performs syntax parsing only: imports and
/// semantic references require workspace context and remain the LSP's job.
pub(super) fn run_inspector() {
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("Failed to read Xenomorph source from stdin: {error}");
        std::process::exit(1);
    }

    let result = inspect_source(&source);
    if let Err(error) = serde_json::to_writer_pretty(io::stdout().lock(), &result) {
        eprintln!("Failed to serialize inspection result: {error}");
        std::process::exit(1);
    }
    println!();
}

fn inspect_source(source: &str) -> InspectResult<'_> {
    let tokens = match Lexer::tokenize(source) {
        Ok(tokens) => tokens,
        Err(diagnostic) => {
            return InspectResult {
                tokens: Vec::new(),
                ast: Vec::new(),
                diagnostics: vec![inspect_diagnostic(&diagnostic)],
            };
        }
    };

    let inspect_tokens = tokens
        .iter()
        .map(|(kind, data)| InspectToken {
            kind: format!("{kind:?}"),
            lexeme: data.v,
            range: token_range(data),
        })
        .collect();
    let (ast, diagnostics) = Parser::parse(&tokens);
    let inspect_ast = ast
        .iter()
        .map(|declaration| declaration_node(declaration, &tokens))
        .collect();

    InspectResult {
        tokens: inspect_tokens,
        ast: inspect_ast,
        diagnostics: diagnostics.iter().map(inspect_diagnostic).collect(),
    }
}

fn inspect_diagnostic(diagnostic: &XenoDiagnostic<'_>) -> InspectDiagnostic {
    let severity = match diagnostic.severity {
        XenoDiagSeverity::Err => "error",
        XenoDiagSeverity::Warn => "warning",
        XenoDiagSeverity::Info => "information",
    };

    InspectDiagnostic {
        severity,
        message: diagnostic.message.clone(),
        range: token_range(&diagnostic.location),
    }
}

fn token_range(token: &TokenData<'_>) -> InspectRange {
    let start = InspectPosition {
        line: token.l,
        character: token.c,
    };
    let mut end = start;
    for character in token.v.chars() {
        if character == '\n' {
            end.line += 1;
            end.character = 0;
        } else {
            end.character += 1;
        }
    }
    InspectRange { start, end }
}

fn range_from_to(start: &TokenData<'_>, end: &TokenData<'_>) -> InspectRange {
    InspectRange {
        start: token_range(start).start,
        end: token_range(end).end,
    }
}

fn range_covering_nodes(children: &[AstNode]) -> Option<InspectRange> {
    let mut ranges = children.iter().filter_map(|child| child.range);
    let first = ranges.next()?;
    Some(ranges.fold(first, |range, next| InspectRange {
        start: range.start,
        end: next.end,
    }))
}

fn declaration_range(
    tokens: &[Token<'_>],
    start: &TokenData<'_>,
    end_hint: &TokenData<'_>,
) -> InspectRange {
    let start = tokens
        .iter()
        .position(|(_, data)| (data.l, data.c) == (start.l, start.c))
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| tokens.get(index))
        .filter(|(kind, _)| *kind == TokenVariant::Documentation)
        .map(|(_, data)| data)
        .unwrap_or(start);
    let end = tokens
        .iter()
        .find(|(kind, data)| {
            *kind == TokenVariant::Semicolon && (data.l, data.c) >= (end_hint.l, end_hint.c)
        })
        .map(|(_, data)| data)
        .unwrap_or(end_hint);
    range_from_to(start, end)
}

fn node(
    kind: &'static str,
    label: Option<String>,
    range: Option<InspectRange>,
    children: Vec<AstNode>,
) -> AstNode {
    AstNode {
        kind,
        label,
        range: range.or_else(|| range_covering_nodes(&children)),
        children,
    }
}

fn token_node(kind: &'static str, token: &TokenData<'_>) -> AstNode {
    node(
        kind,
        Some(token.v.to_string()),
        Some(token_range(token)),
        Vec::new(),
    )
}

fn declaration_node(declaration: &Declaration<'_>, tokens: &[Token<'_>]) -> AstNode {
    match declaration {
        Declaration::Import { path, location } => node(
            "ImportDeclaration",
            Some(path.join("/")),
            Some(declaration_range(tokens, location, location)),
            Vec::new(),
        ),
        Declaration::Type {
            docs,
            name,
            generics,
            ty,
            from,
            to,
        } => {
            let mut children = Vec::new();
            if let Some(docs) = docs {
                children.push(node(
                    "Documentation",
                    Some((*docs).to_string()),
                    None,
                    Vec::new(),
                ));
            }
            children.push(token_node("Name", name));
            if let Some(generics) = generics {
                children.push(node(
                    "Generics",
                    None,
                    None,
                    generics
                        .iter()
                        .map(|(generic, constraint)| {
                            let constraint_nodes = constraint
                                .iter()
                                .map(|constraint| token_node("Constraint", constraint))
                                .collect();
                            node(
                                "GenericParameter",
                                Some(generic.v.to_string()),
                                Some(token_range(generic)),
                                constraint_nodes,
                            )
                        })
                        .collect(),
                ));
            }
            children.push(type_node(&ty.0));
            if !ty.1.is_empty() {
                children.push(node(
                    "Annotations",
                    None,
                    None,
                    ty.1.iter().map(annotation_node).collect(),
                ));
            }

            node(
                "TypeDeclaration",
                Some(name.v.to_string()),
                Some(declaration_range(tokens, from, to)),
                children,
            )
        }
        Declaration::Custom {
            plugin_id,
            decl_id,
            docs,
            name,
            ..
        } => {
            let mut children = Vec::new();
            if let Some(docs) = docs {
                children.push(node(
                    "Documentation",
                    Some((*docs).to_string()),
                    None,
                    Vec::new(),
                ));
            }
            node(
                "CustomDeclaration",
                Some(match name {
                    Some(name) => format!("{} ({plugin_id}/{decl_id})", name.v),
                    None => format!("{plugin_id}/{decl_id}"),
                }),
                name.map(|name| token_range(name)),
                children,
            )
        }
    }
}

fn type_node(ty: &Type<'_>) -> AstNode {
    match ty {
        Type::Simple(simple) => simple_type_node(simple),
        Type::Tuple(types) => node(
            "TupleType",
            None,
            None,
            types.iter().map(simple_type_node).collect(),
        ),
        Type::Set(set) => {
            let mut children = Vec::new();
            if let Some(element_type) = &set.element_type {
                children.push(node(
                    "ElementType",
                    None,
                    None,
                    vec![simple_type_node(element_type)],
                ));
            }
            if let Some(values) = &set.values {
                children.push(node(
                    "Values",
                    None,
                    None,
                    values
                        .iter()
                        .map(|literal| literal_node("Literal", literal))
                        .collect(),
                ));
            }
            node("SetType", None, None, children)
        }
        Type::Struct(fields) => node(
            "StructType",
            None,
            None,
            fields.iter().map(field_node).collect(),
        ),
        Type::Enum(fields) => node(
            "EnumType",
            None,
            None,
            fields.iter().map(field_node).collect(),
        ),
        Type::Sum(types) => node(
            "SumType",
            None,
            None,
            types.iter().map(simple_type_node).collect(),
        ),
        Type::Intersection(types) => node(
            "IntersectionType",
            None,
            None,
            types.iter().map(simple_type_node).collect(),
        ),
    }
}

fn simple_type_node(ty: &SimpleType<'_>) -> AstNode {
    match ty {
        SimpleType::Optional(inner) => node(
            "OptionalType",
            None,
            Some(token_range(ty.get_last_token())),
            vec![simple_type_node(inner)],
        ),
        SimpleType::Literal(literal) => literal_node("Literal", literal),
        SimpleType::Identifier(token, arguments) => node(
            "IdentifierType",
            Some(token.v.to_string()),
            Some(InspectRange {
                start: token_range(token).start,
                end: token_range(ty.get_last_token()).end,
            }),
            arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(simple_type_node)
                .collect(),
        ),
        SimpleType::Array(token, arguments) => node(
            "ArrayType",
            Some(token.v.to_string()),
            Some(InspectRange {
                start: token_range(token).start,
                end: token_range(ty.get_last_token()).end,
            }),
            arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(simple_type_node)
                .collect(),
        ),
    }
}

fn literal_node(kind: &'static str, literal: &Literal<'_>) -> AstNode {
    let (label, children) = match literal {
        Literal::Int(value) => {
            let size = match value.representation.size {
                IntegerSize::Bits(bits) => {
                    format!("{bits} bit{}", if bits == 1 { "" } else { "s" })
                }
                IntegerSize::Arbitrary => "arbitrary precision".to_string(),
            };
            let source = if value.cast.is_some() {
                "explicit"
            } else {
                "inferred"
            };
            let signedness = if value.representation.signed {
                "signed"
            } else {
                "unsigned"
            };
            (
                value.value.to_string(),
                vec![node(
                    "IntegerRepresentation",
                    Some(format!("{signedness}, {size}, {source}")),
                    value.cast.map(token_range),
                    Vec::new(),
                )],
            )
        }
        Literal::Float(value) => {
            let size = match value.representation.size {
                FloatSize::F32 => "f32",
                FloatSize::F64 => "f64",
                FloatSize::Decimal => "decimal",
            };
            let source = if value.cast.is_some() {
                "explicit"
            } else {
                "inferred"
            };
            (
                value.value.to_string(),
                vec![node(
                    "FloatRepresentation",
                    Some(format!(
                        "signed, {size}, precision {}, scale {}, {source}",
                        value.representation.precision, value.representation.scale
                    )),
                    value.cast.map(token_range),
                    Vec::new(),
                )],
            )
        }
        Literal::String(value, _) => (format!("\"{value}\""), Vec::new()),
        Literal::Boolean(value, _) => (value.to_string(), Vec::new()),
    };
    node(
        kind,
        Some(label),
        Some(InspectRange {
            start: token_range(literal.token()).start,
            end: token_range(literal.get_last_token()).end,
        }),
        children,
    )
}

fn field_node(field: &xenomorph_common::parser::KeyValExpr<'_>) -> AstNode {
    let (name, ty, docs) = field;
    let mut children = Vec::new();
    if let Some(docs) = docs {
        children.push(token_node("Documentation", docs));
    }
    children.push(simple_type_node(ty));
    node(
        "Field",
        Some(name.v.to_string()),
        Some(InspectRange {
            start: token_range(name).start,
            end: token_range(ty.get_last_token()).end,
        }),
        children,
    )
}

fn annotation_node(annotation: &Annotation<'_>) -> AstNode {
    node(
        "Annotation",
        Some(annotation.ident.v.to_string()),
        Some(range_from_to(annotation.ident, annotation.get_last_token())),
        annotation.params.iter().map(expression_node).collect(),
    )
}

fn expression_node(expression: &Expr<'_>) -> AstNode {
    match expression {
        Expr::Regex(token) => token_node("RegexExpression", token),
        Expr::Annotation(annotation) => annotation_node(annotation),
        Expr::Type(ty) => type_node(ty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_emits_tokens_ast_and_declaration_ranges() {
        let result = inspect_source("type User = { name: string };");

        assert!(result.diagnostics.is_empty());
        assert_eq!(result.tokens.first().unwrap().kind, "Type");
        assert_eq!(result.tokens.first().unwrap().lexeme, "type");
        assert_eq!(result.ast.len(), 1);
        assert_eq!(result.ast[0].kind, "TypeDeclaration");
        assert_eq!(result.ast[0].label.as_deref(), Some("User"));
        assert_eq!(result.ast[0].range.unwrap().start.character, 0);
        assert_eq!(result.ast[0].range.unwrap().end.character, 29);
    }

    #[test]
    fn inspector_returns_lexer_diagnostics_as_json_data() {
        let result = inspect_source("type Broken = #;");

        assert!(result.tokens.is_empty());
        assert!(result.ast.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].severity, "error");
        assert_eq!(result.diagnostics[0].range.start.character, 14);
    }

    #[test]
    fn inspector_exposes_numeric_literal_representation() {
        let result = inspect_source("type Id = 1 as u64;");

        assert!(result.diagnostics.is_empty());
        let literal = &result.ast[0].children[1];
        assert_eq!(literal.kind, "Literal");
        assert_eq!(literal.range.unwrap().end.character, 18);
        assert_eq!(literal.children[0].kind, "IntegerRepresentation");
        assert_eq!(
            literal.children[0].label.as_deref(),
            Some("unsigned, 64 bits, explicit")
        );
    }

    #[test]
    fn declaration_range_includes_leading_documentation() {
        let result = inspect_source("/** User docs */\ntype User = string;");

        assert!(result.diagnostics.is_empty());
        assert_eq!(result.ast[0].range.unwrap().start.line, 0);
        assert_eq!(result.ast[0].range.unwrap().start.character, 0);
        assert_eq!(result.ast[0].range.unwrap().end.line, 1);
    }
}
