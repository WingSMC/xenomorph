use serde::Serialize;
use std::io::{self, Read};
use xenomorph_common::config::{write_rc_schema, Config, LogLevel, RC_SCHEMA_RELATIVE_PATH};
use xenomorph_common::lexer::{Lexer, Token, TokenVariant};
use xenomorph_common::module::{types::ModuleDiagnostic, XenoRegistry};
use xenomorph_common::parser::{Annotation, Declaration, Expr, Literal, Parser, SimpleType, Type};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::{TokenData, XenoDiagSeverity, XenoDiagnostic};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("schema") => generate_rc_schema(),
        Some("inspect") => run_inspector(),
        _ => run_parser(),
    }
}

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
fn run_inspector() {
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
        Type::Set(types) => node(
            "SetType",
            None,
            None,
            types.iter().map(simple_type_node).collect(),
        ),
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
        SimpleType::Literal(literal) => literal_node("Literal", literal),
        SimpleType::OptionalLiteral(literal) => literal_node("OptionalLiteral", literal),
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
        SimpleType::OptionalIdentifier(token, arguments) => node(
            "OptionalIdentifierType",
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
        SimpleType::OptionalArray(token, arguments) => node(
            "OptionalArrayType",
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
    let (label, token) = match literal {
        Literal::Int(value, token) => (value.to_string(), *token),
        Literal::Float(value, token) => (value.to_string(), *token),
        Literal::String(value, token) => (format!("\"{value}\""), *token),
        Literal::Boolean(value, token) => (value.to_string(), *token),
    };
    node(kind, Some(label), Some(token_range(token)), Vec::new())
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

/// Generates the `xenomorph.toml` JSON Schema (base + plugin contributions) and
/// writes it to `.xenomorph/xenomorph.schema.json` in the workspace root.
fn generate_rc_schema() {
    let plugins = XenoPlugin::get_plugins();
    let out_path = Config::get().workdir.join(RC_SCHEMA_RELATIVE_PATH);

    match write_rc_schema(plugins, &out_path) {
        Ok(()) => println!("✓ Wrote xenomorph.toml schema → {}", out_path.display()),
        Err(e) => {
            eprintln!("✗ Failed to write xenomorph.toml schema: {}", e);
            std::process::exit(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DiagnosticCounts {
    errors: usize,
    warnings: usize,
    infos: usize,
}

impl DiagnosticCounts {
    fn add(&mut self, severity: XenoDiagSeverity) {
        match severity {
            XenoDiagSeverity::Err => self.errors += 1,
            XenoDiagSeverity::Warn => self.warnings += 1,
            XenoDiagSeverity::Info => self.infos += 1,
        }
    }

    fn include(&mut self, other: Self) {
        self.errors += other.errors;
        self.warnings += other.warnings;
        self.infos += other.infos;
    }

    fn is_empty(self) -> bool {
        self.errors == 0 && self.warnings == 0 && self.infos == 0
    }

    fn marker(self) -> &'static str {
        if self.errors > 0 {
            "✗"
        } else {
            "✓"
        }
    }

    fn summary(self, loglevel: LogLevel) -> String {
        let mut parts = vec![format!("{} error(s)", self.errors)];
        if loglevel != LogLevel::Error {
            parts.push(format!("{} warning(s)", self.warnings));
        }
        if loglevel == LogLevel::Info {
            parts.push(format!("{} info(s)", self.infos));
        }
        parts.join(", ")
    }
}

fn count_diagnostics<'a>(
    diagnostics: impl IntoIterator<Item = &'a ModuleDiagnostic>,
) -> DiagnosticCounts {
    let mut counts = DiagnosticCounts::default();
    for diagnostic in diagnostics {
        counts.add(diagnostic.severity);
    }
    counts
}

fn severity_name(severity: XenoDiagSeverity) -> &'static str {
    match severity {
        XenoDiagSeverity::Err => "error",
        XenoDiagSeverity::Warn => "warning",
        XenoDiagSeverity::Info => "info",
    }
}

fn format_cli_diagnostic(diagnostic: &ModuleDiagnostic) -> String {
    let module_path = if diagnostic.module_path.is_empty() {
        "unknown module"
    } else {
        &diagnostic.module_path
    };

    format!(
        "[{}] [{}] {}",
        severity_name(diagnostic.severity),
        module_path,
        diagnostic.message
    )
}

fn run_parser() {
    let loglevel = Config::get().debug.loglevel;
    let reg = match XenoRegistry::load_workspace(true) {
        Ok(r) => r,
        Err(e) => {
            for diagnostic in e
                .iter()
                .filter(|diagnostic| loglevel.allows(diagnostic.severity))
            {
                eprintln!("{}", format_cli_diagnostic(diagnostic));
            }
            std::process::exit(1);
        }
    };

    let cache = reg.module_cache.read().unwrap();
    let module_count = cache.len();
    let mut total_counts = DiagnosticCounts::default();

    for module in cache.values() {
        let path = module.borrow_module_path();
        let decl_count = module.borrow_declarations().len();
        let diagnostics: Vec<_> = module
            .borrow_analyzer_errors()
            .iter()
            .chain(module.borrow_parser_errors())
            .chain(module.borrow_lexer_errors())
            .chain(module.borrow_module_errors())
            .filter(|diagnostic| loglevel.allows(diagnostic.severity))
            .collect();
        let counts = count_diagnostics(diagnostics.iter().copied());
        total_counts.include(counts);

        if counts.is_empty() {
            println!("✓ {} ({} declarations)", path, decl_count);
        } else {
            eprintln!(
                "{} {} ({} declarations; {})",
                counts.marker(),
                path,
                decl_count,
                counts.summary(loglevel)
            );
            for diagnostic in diagnostics {
                eprintln!("  └ {}", format_cli_diagnostic(diagnostic));
            }
        }
    }

    println!(
        "\n{} module(s) processed, {}",
        module_count,
        total_counts.summary(loglevel)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::module::types::ErrorPhase;

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
    fn declaration_range_includes_leading_documentation() {
        let result = inspect_source("/** User docs */\ntype User = string;");

        assert!(result.diagnostics.is_empty());
        assert_eq!(result.ast[0].range.unwrap().start.line, 0);
        assert_eq!(result.ast[0].range.unwrap().start.character, 0);
        assert_eq!(result.ast[0].range.unwrap().end.line, 1);
    }

    #[test]
    fn cli_diagnostic_format_includes_severity_and_module() {
        let diagnostic = ModuleDiagnostic {
            module_path: "models/user".to_string(),
            message: "Unknown annotation '@example'".to_string(),
            location: None,
            phase: ErrorPhase::Analyzer,
            severity: XenoDiagSeverity::Warn,
        };

        assert_eq!(
            format_cli_diagnostic(&diagnostic),
            "[warning] [models/user] Unknown annotation '@example'"
        );
    }

    #[test]
    fn cli_diagnostic_format_has_a_module_fallback() {
        let diagnostic = ModuleDiagnostic {
            module_path: String::new(),
            message: "diagnostic".to_string(),
            location: None,
            phase: ErrorPhase::Module,
            severity: XenoDiagSeverity::Err,
        };

        assert_eq!(
            format_cli_diagnostic(&diagnostic),
            "[error] [unknown module] diagnostic"
        );
    }

    #[test]
    fn diagnostic_summary_matches_loglevel() {
        let counts = DiagnosticCounts {
            errors: 1,
            warnings: 2,
            infos: 3,
        };

        assert_eq!(counts.summary(LogLevel::Error), "1 error(s)");
        assert_eq!(
            counts.summary(LogLevel::Warning),
            "1 error(s), 2 warning(s)"
        );
        assert_eq!(
            counts.summary(LogLevel::Info),
            "1 error(s), 2 warning(s), 3 info(s)"
        );
    }

    #[test]
    fn only_errors_use_the_failure_marker() {
        assert_eq!(
            DiagnosticCounts {
                errors: 1,
                warnings: 0,
                infos: 0,
            }
            .marker(),
            "✗"
        );
        assert_eq!(
            DiagnosticCounts {
                errors: 0,
                warnings: 1,
                infos: 1,
            }
            .marker(),
            "✓"
        );
    }
}
