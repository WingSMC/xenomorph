//! Pydantic v2 model generator for Xenomorph schemas.
//!
//! Structs become `BaseModel` classes, other declarations become PEP 695
//! `type` aliases, and Xenomorph's native validators become Pydantic
//! constraints so generated models enforce the same rules as the schema.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use xenomorph_common::config::{Config, ConfigValue};
use xenomorph_common::parser::{
    Annotation, Declaration, Expr, KeyValExpr, Literal, SimpleType, Type,
};
use xenomorph_common::plugins::protocol::CustomDeclarationRequest;
use xenomorph_common::plugins::{CustomDeclarationMatcher, CustomDeclarationResult, XenoPlugin};
use xenomorph_common::semantic::{
    simple_to_owned_type, type_to_owned_type, unsupported_target_type_diagnostic, AnalyzerListener,
    OwnedField, OwnedLiteral, OwnedType, ScopeInfo, TypeHierarchy, XenoAnnotation,
    XenoAnnotationKind, XenoConstraint, XenoParam, ANY_TARGET_PARAM, LITERAL_VALUE_PARAM,
    STRING_LITERAL, STRING_VALUE_PARAM, STRUCT,
};
use xenomorph_common::utils::extract_documentation;
use xenomorph_common::{TokenData, XenoDiagSeverity, XenoDiagnostic};

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "pydantic";
static NAMESPACE: &str = "pydantic";
static CUSTOM_IDENTIFIER: &str = "pydantic";

static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    namespace: NAMESPACE,
    version: env!("CARGO_PKG_VERSION"),
    initialize: None,
    shutdown: None,
    provide_types: None,
    provide_annotations: Some(provide_annotations),
    provide_custom_declarations: Some(provide_custom_declarations),
    provide_config_schema: Some(provide_config_schema),
    register_generator: Some(create_generator),
    register_analyzer: Some(create_analyzer),
};

xenomorph_common::export_plugin!(PLUGIN);

static MODEL_TARGET_PARAM: XenoParam = XenoParam {
    name: "self",
    constraint: XenoConstraint::Trait(&STRUCT),
    optional: false,
};
static CONFIG_KEY_PARAM: XenoParam = XenoParam {
    name: "key",
    constraint: XenoConstraint::Trait(&STRING_LITERAL),
    optional: false,
};
static FUNCTION_PARAM: XenoParam = XenoParam {
    name: "function",
    constraint: XenoConstraint::Trait(&STRING_LITERAL),
    optional: false,
};

macro_rules! annotation {
    ($ident:ident, $name:literal, $kind:ident, [$($param:ident),*], $doc:literal) => {
        static $ident: XenoAnnotation = XenoAnnotation {
            name: $name,
            documentation: Some($doc),
            kind: XenoAnnotationKind::$kind,
            params: &[$(&$param),*],
            variadic: false,
        };
    };
}

annotation!(
    EXTRA,
    "extra",
    Meta,
    [MODEL_TARGET_PARAM, STRING_VALUE_PARAM],
    "Sets the model's `extra` behaviour: `\"allow\"`, `\"ignore\"`, or `\"forbid\"`."
);
annotation!(
    FROZEN,
    "frozen",
    Meta,
    [MODEL_TARGET_PARAM],
    "Makes generated model instances immutable and hashable (`frozen=True`)."
);
annotation!(
    STRICT,
    "strict",
    Validation,
    [ANY_TARGET_PARAM],
    "Disables Pydantic's lax type coercion for the model or type alias (`strict=True`)."
);
annotation!(POPULATE_BY_NAME, "populate_by_name", Meta, [MODEL_TARGET_PARAM],
    "Accepts Python attribute names in addition to aliases when validating (`populate_by_name=True`).");
annotation!(
    VALIDATE_ASSIGNMENT,
    "validate_assignment",
    Meta,
    [MODEL_TARGET_PARAM],
    "Re-validates fields when they are assigned after construction (`validate_assignment=True`)."
);
annotation!(
    USE_ENUM_VALUES,
    "use_enum_values",
    Meta,
    [MODEL_TARGET_PARAM],
    "Stores enum members as their raw values (`use_enum_values=True`)."
);
annotation!(
    STR_STRIP_WHITESPACE,
    "str_strip_whitespace",
    Meta,
    [MODEL_TARGET_PARAM],
    "Strips leading and trailing whitespace from string fields (`str_strip_whitespace=True`)."
);
annotation!(ALIAS_GENERATOR, "alias_generator", Meta, [MODEL_TARGET_PARAM, STRING_VALUE_PARAM],
    "Generates field aliases with Pydantic's `to_camel`, `to_pascal`, or `to_snake`: `\"camel\"`, `\"pascal\"`, or `\"snake\"`.");
annotation!(
    TITLE,
    "title",
    Meta,
    [ANY_TARGET_PARAM, STRING_VALUE_PARAM],
    "Sets the JSON Schema title of the model or type alias."
);
annotation!(CONFIG, "config", Meta, [MODEL_TARGET_PARAM, CONFIG_KEY_PARAM, LITERAL_VALUE_PARAM],
    "Sets an arbitrary `ConfigDict` entry, for example `@pydantic::config(\"regex_engine\", \"python-re\")`.");
annotation!(
    DISCRIMINATOR,
    "discriminator",
    Validation,
    [ANY_TARGET_PARAM, STRING_VALUE_PARAM],
    "Validates a union of models as a tagged union keyed by the named field."
);
annotation!(AFTER_VALIDATOR, "after_validator", Validation, [ANY_TARGET_PARAM, FUNCTION_PARAM],
    "Runs a Python function after Pydantic's validation. Takes a dotted import path such as `\"app.checks.normalize\"`.");
annotation!(BEFORE_VALIDATOR, "before_validator", Validation, [ANY_TARGET_PARAM, FUNCTION_PARAM],
    "Runs a Python function on the raw input before Pydantic's validation. Takes a dotted import path.");
annotation!(PLAIN_VALIDATOR, "plain_validator", Validation, [ANY_TARGET_PARAM, FUNCTION_PARAM],
    "Replaces Pydantic's validation of a type alias with a Python function. Takes a dotted import path.");
annotation!(WRAP_VALIDATOR, "wrap_validator", Validation, [ANY_TARGET_PARAM, FUNCTION_PARAM],
    "Wraps Pydantic's validation with a Python function receiving `(value, handler)`. Takes a dotted import path.");

static ANNOTATIONS: &[&XenoAnnotation] = &[
    &EXTRA,
    &FROZEN,
    &STRICT,
    &POPULATE_BY_NAME,
    &VALIDATE_ASSIGNMENT,
    &USE_ENUM_VALUES,
    &STR_STRIP_WHITESPACE,
    &ALIAS_GENERATOR,
    &TITLE,
    &CONFIG,
    &DISCRIMINATOR,
    &AFTER_VALIDATOR,
    &BEFORE_VALIDATOR,
    &PLAIN_VALIDATOR,
    &WRAP_VALIDATOR,
];

fn provide_annotations() -> &'static [&'static XenoAnnotation] {
    ANNOTATIONS
}

static CUSTOM_DECLARATIONS: &[CustomDeclarationMatcher] = &[CustomDeclarationMatcher {
    identifier: CUSTOM_IDENTIFIER,
    parse: parse_custom_declaration,
}];

fn provide_custom_declarations() -> &'static [CustomDeclarationMatcher] {
    CUSTOM_DECLARATIONS
}

/// `#pydantic ... #;` blocks are copied verbatim into the generated module.
fn parse_custom_declaration(
    _config: &Config,
    request: &CustomDeclarationRequest,
) -> CustomDeclarationResult {
    if request.content().trim().is_empty() {
        return Err(XenoDiagnostic {
            tags: vec![],
            location: request.location,
            message: "Empty #pydantic block has no Python code to emit.".into(),
            severity: XenoDiagSeverity::Warn,
        });
    }
    Ok(())
}

fn provide_config_schema() -> &'static str {
    r#"{
        "type": "object",
        "description": "Pydantic v2 model generator.",
        "properties": {
            "output": {
                "type": "string",
                "description": "Output directory for generated .py files, relative to the workspace root. If omitted, files are written next to their .xen sources."
            },
            "package": {
                "type": "string",
                "description": "Dotted Python package that contains the generated modules. When set, cross-module imports are absolute (`from <package>.models import User`); otherwise they are relative (`from .models import User`)."
            },
            "extra": {
                "type": "string",
                "enum": ["allow", "ignore", "forbid"],
                "description": "Default `extra` behaviour for every generated model. `@pydantic::extra` overrides it per model."
            }
        },
        "additionalProperties": false
    }"#
}

fn create_generator(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    let mut generator = PydanticGenerator::new();
    generator.workdir = config.workdir.clone();
    if let Some(ConfigValue::Table(table)) = config.plugins.config.get(NAME) {
        if let Some(ConfigValue::String(output)) = table.get("output") {
            generator.output_dir = Some(PathBuf::from(output));
        }
        if let Some(ConfigValue::String(package)) = table.get("package") {
            generator.package = Some(package.trim_matches('.').to_string());
        }
        if let Some(ConfigValue::String(extra)) = table.get("extra") {
            generator.default_extra = Some(extra.clone());
        }
    }
    Box::new(generator)
}

fn create_analyzer(_config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    Box::new(PydanticAnalyzer::default())
}

// ── Annotation helpers ──────────────────────────────────────────────

const EXTRA_VALUES: &[&str] = &["allow", "ignore", "forbid"];
const ALIAS_GENERATORS: &[(&str, &str)] = &[
    ("camel", "to_camel"),
    ("pascal", "to_pascal"),
    ("snake", "to_snake"),
];

fn annotation_name<'a>(annotation: &Annotation, source: &'a str) -> &'a str {
    annotation.ident.text(source)
}

fn string_param(annotation: &Annotation, index: usize) -> Option<(&str, TokenData)> {
    match annotation.params.get(index) {
        Some(Expr::Type(Type::Simple(SimpleType::Literal(Literal::String(value, token))))) => {
            Some((value.as_str(), *token))
        }
        _ => None,
    }
}

fn param_location(annotation: &Annotation, index: usize) -> TokenData {
    string_param(annotation, index).map_or(annotation.ident, |(_, token)| token)
}

fn number_param(annotation: &Annotation, index: usize) -> Option<String> {
    match annotation.params.get(index) {
        Some(Expr::Type(Type::Simple(SimpleType::Literal(literal)))) => match literal {
            Literal::Int(integer) => Some(integer.value.to_string()),
            Literal::Float(float) => Some(float.value.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn literal_param(annotation: &Annotation, index: usize) -> Option<String> {
    match annotation.params.get(index) {
        Some(Expr::Type(Type::Simple(SimpleType::Literal(literal)))) => Some(match literal {
            Literal::String(value, _) => python_string(value),
            Literal::Int(integer) => integer.value.to_string(),
            Literal::Float(float) => float.value.to_string(),
            Literal::Boolean(value, _) => python_bool(*value).to_string(),
        }),
        _ => None,
    }
}

fn deprecation_message(annotation: &Annotation) -> String {
    string_param(annotation, 0)
        .map(|(reason, _)| reason.to_string())
        .unwrap_or_else(|| "Deprecated.".to_string())
}

/// Splits `"package.module.function"` into an import module and a symbol.
fn split_function_path(path: &str) -> Option<(&str, &str)> {
    let (module, function) = path.rsplit_once('.')?;
    let module_ok = !module.trim_start_matches('.').is_empty()
        && module
            .trim_start_matches('.')
            .split('.')
            .all(is_python_identifier)
        || module.chars().all(|character| character == '.') && !module.is_empty();
    (module_ok && is_python_identifier(function) && !is_python_keyword(function))
        .then_some((module, function))
}

fn diagnostic(location: TokenData, severity: XenoDiagSeverity, message: String) -> XenoDiagnostic {
    XenoDiagnostic {
        tags: vec![],
        location,
        message,
        severity,
    }
}

// ── Analyzer listener ───────────────────────────────────────────────

/// Validates Pydantic annotation arguments and Python naming constraints
/// while editing, so mistakes surface before generation.
#[derive(Default)]
struct PydanticAnalyzer {
    source: String,
}

impl<'src> AnalyzerListener<'src> for PydanticAnalyzer {
    fn set_source(&mut self, source: &str) {
        self.source.clear();
        self.source.push_str(source);
    }

    fn on_before_decl(&mut self, decl: &Declaration, errors: &mut Vec<XenoDiagnostic>) {
        let Declaration::Type {
            name, generics, ty, ..
        } = decl
        else {
            return;
        };
        report_python_name(name, "type", &self.source, errors);
        for (generic, _) in generics.as_deref().unwrap_or_default() {
            report_python_name(generic, "generic parameter", &self.source, errors);
        }
        for annotation in &ty.1 {
            validate_annotation(annotation, &ty.0, &self.source, errors);
        }
    }
}

fn report_python_name(
    identifier: &TokenData,
    kind: &str,
    source: &str,
    errors: &mut Vec<XenoDiagnostic>,
) {
    let text = identifier.text(source);
    if is_python_keyword(text) || PYTHON_BUILTIN_NAMES.contains(&text) {
        errors.push(diagnostic(
            *identifier,
            XenoDiagSeverity::Err,
            format!("Pydantic {kind} name '{text}' conflicts with a Python keyword or builtin."),
        ));
    }
}

fn validate_annotation(
    annotation: &Annotation,
    target: &Type,
    source: &str,
    errors: &mut Vec<XenoDiagnostic>,
) {
    let name = annotation_name(annotation, source);
    let Some(local) = name.strip_prefix("pydantic::") else {
        return;
    };
    let mut error = |index: usize, message: String| {
        errors.push(diagnostic(
            param_location(annotation, index),
            XenoDiagSeverity::Err,
            message,
        ));
    };
    match local {
        "extra" => {
            if let Some((value, _)) = string_param(annotation, 0) {
                if !EXTRA_VALUES.contains(&value) {
                    error(0, format!("@{name} expects \"allow\", \"ignore\", or \"forbid\", not \"{value}\"."));
                }
            }
        }
        "alias_generator" => {
            if let Some((value, _)) = string_param(annotation, 0) {
                if !ALIAS_GENERATORS.iter().any(|(key, _)| *key == value) {
                    error(
                        0,
                        format!(
                            "@{name} expects \"camel\", \"pascal\", or \"snake\", not \"{value}\"."
                        ),
                    );
                }
            }
        }
        "config" => {
            if let Some((key, _)) = string_param(annotation, 0) {
                if !is_python_identifier(key) {
                    error(
                        0,
                        format!("@{name} key '{key}' is not a valid ConfigDict entry name."),
                    );
                }
            }
        }
        "discriminator" => {
            if let Some((value, _)) = string_param(annotation, 0) {
                if value.is_empty() {
                    error(0, format!("@{name} requires a field name."));
                }
            }
            if matches!(target, Type::Struct(_)) {
                error(
                    0,
                    format!("@{name} applies to unions of models, not to a struct."),
                );
            }
        }
        "after_validator" | "before_validator" | "plain_validator" | "wrap_validator" => {
            if let Some((path, _)) = string_param(annotation, 0) {
                if split_function_path(path).is_none() {
                    error(0, format!(
                        "@{name} expects a dotted Python import path such as \"app.validators.check\", not \"{path}\"."
                    ));
                }
            }
            if local == "plain_validator" && matches!(target, Type::Struct(_)) {
                error(0, format!("@{name} cannot replace model validation; use @pydantic::before_validator or @pydantic::wrap_validator."));
            }
        }
        _ => {}
    }
}

// ── Python naming ───────────────────────────────────────────────────

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Builtins used in generated annotations; shadowing them breaks the module.
const PYTHON_BUILTIN_NAMES: &[&str] = &[
    "bool", "bytes", "dict", "float", "int", "list", "object", "set", "str", "tuple", "type",
];

/// Attributes inherited from `BaseModel` that fields must not shadow.
const BASE_MODEL_ATTRIBUTES: &[&str] = &[
    "construct",
    "copy",
    "dict",
    "fields",
    "from_orm",
    "json",
    "parse_file",
    "parse_obj",
    "parse_raw",
    "schema",
    "schema_json",
    "update_forward_refs",
    "validate",
];

const STDLIB_MODULES: &[&str] = &[
    "__future__",
    "datetime",
    "decimal",
    "enum",
    "ipaddress",
    "re",
    "typing",
    "uuid",
];

fn is_python_keyword(candidate: &str) -> bool {
    PYTHON_KEYWORDS.contains(&candidate)
}

fn is_python_identifier(candidate: &str) -> bool {
    let mut characters = candidate.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

/// Python attribute for a wire name. Non-identifiers, keywords, private
/// (underscore) names, and `BaseModel` attributes are renamed; the caller
/// keeps the wire name as the field's alias.
fn python_field_name(wire: &str, used: &mut HashSet<String>) -> String {
    let mut name: String = wire
        .chars()
        .map(
            |character| match character.is_alphanumeric() || character == '_' {
                true => character,
                false => '_',
            },
        )
        .collect();
    name = name.trim_start_matches('_').to_string();
    if name.is_empty() {
        name = "field".to_string();
    }
    if name.starts_with(|character: char| character.is_ascii_digit()) || name.starts_with("model_")
    {
        name = format!("f_{name}");
    }
    if is_python_keyword(&name) || BASE_MODEL_ATTRIBUTES.contains(&name.as_str()) {
        name.push('_');
    }
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

fn python_enum_member(key: &str, used: &mut HashSet<String>) -> String {
    let mut name: String = key
        .chars()
        .map(
            |character| match character.is_alphanumeric() || character == '_' {
                true => character,
                false => '_',
            },
        )
        .collect();
    name = name.trim_start_matches('_').to_string();
    if name.is_empty() || name.starts_with(|character: char| character.is_ascii_digit()) {
        name = format!("V_{name}");
    }
    if is_python_keyword(&name) || matches!(name.as_str(), "name" | "value" | "mro") {
        name.push('_');
    }
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

fn python_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                output.push_str(&format!("\\u{:04x}", control as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn python_bool(value: bool) -> &'static str {
    match value {
        true => "True",
        false => "False",
    }
}

fn python_docstring(docs: &str, indent: &str) -> String {
    let escaped = docs
        .trim()
        .replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"");
    if !escaped.contains('\n') {
        return format!("{indent}\"\"\"{escaped}\"\"\"\n");
    }
    let mut output = format!("{indent}\"\"\"\n");
    for line in escaped.lines() {
        match line.trim().is_empty() {
            true => output.push('\n'),
            false => output.push_str(&format!("{indent}{}\n", line.trim_end())),
        }
    }
    output.push_str(&format!("{indent}\"\"\"\n"));
    output
}

fn python_comment(text: &str, indent: &str) -> String {
    text.trim()
        .lines()
        .map(|line| match line.trim_end() {
            "" => format!("{indent}#\n"),
            line => format!("{indent}# {line}\n"),
        })
        .collect()
}

/// Converts a `/pattern/flags` regex literal into a Python string usable as
/// Pydantic's `pattern`. Flags become an inline group, as Pydantic accepts
/// only the pattern text.
fn python_regex(token_text: &str) -> String {
    let end = token_text
        .rfind('/')
        .filter(|end| *end > 0)
        .unwrap_or(token_text.len());
    let body = token_text.get(1..end).unwrap_or_default();
    let flags: String = token_text
        .get(end + 1..)
        .unwrap_or_default()
        .chars()
        .filter(|flag| matches!(flag, 'i' | 'm' | 's' | 'x'))
        .collect();
    let mut pattern = String::new();
    if !flags.is_empty() {
        pattern.push_str(&format!("(?{flags})"));
    }
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && characters.peek() == Some(&'/') {
            continue;
        }
        pattern.push(character);
    }
    python_string(&pattern)
}

fn python_import_path(from_module: &str, to_module: &str, package: Option<&str>) -> String {
    let target: Vec<&str> = to_module.split('/').collect();
    if let Some(package) = package.filter(|package| !package.is_empty()) {
        return format!("{package}.{}", target.join("."));
    }
    let source: Vec<&str> = from_module.split('/').collect();
    let source_package = &source[..source.len().saturating_sub(1)];
    let common = source_package
        .iter()
        .zip(&target[..target.len().saturating_sub(1)])
        .take_while(|(left, right)| left == right)
        .count();
    format!(
        "{}{}",
        ".".repeat(1 + source_package.len() - common),
        target[common..].join(".")
    )
}

fn dedent_block(content: &str) -> String {
    let mut lines: Vec<&str> = content.lines().collect();
    let first = lines
        .first()
        .map(|line| line.trim_start())
        .unwrap_or_default();
    let first_inline = !first.is_empty();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let rest = if lines.is_empty() {
        &lines[..]
    } else {
        &lines[1..]
    };
    let indent = rest
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut output = Vec::new();
    if first_inline {
        output.push(first.to_string());
    }
    for line in rest {
        output.push(
            line.get(indent..)
                .unwrap_or_else(|| line.trim_start())
                .to_string(),
        );
    }
    while output.first().is_some_and(|line| line.trim().is_empty()) {
        output.remove(0);
    }
    output.join("\n")
}

// ── Builtin type mapping ────────────────────────────────────────────

const HOSTNAME_PATTERN: &str = r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*\.?$";
const SEMVER_PATTERN: &str = r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$";
const STRUCT_UTILITIES: &[&str] = &["Pick", "Omit", "Partial", "Complete"];
const TYPE_UTILITIES: &[&str] = &["Keyof", "Required", "Pick", "Omit", "Partial", "Complete"];

struct Builtin {
    /// Python expression, or `None` for the `None` type.
    base: &'static str,
    import: Option<(&'static str, &'static str)>,
    constraints: Vec<(&'static str, String)>,
}

fn builtin(name: &str) -> Option<Builtin> {
    let simple = |base, import| Builtin {
        base,
        import,
        constraints: vec![],
    };
    Some(match name {
        "string" | "strong_password" | "xml" | "yaml" | "json" | "toml" | "csv" | "tsv" => {
            simple("str", None)
        }
        "char" => Builtin {
            base: "str",
            import: None,
            constraints: vec![("min_length", "1".into()), ("max_length", "1".into())],
        },
        "hostname" => Builtin {
            base: "str",
            import: None,
            constraints: vec![
                ("max_length", "253".into()),
                ("pattern", python_string(HOSTNAME_PATTERN)),
            ],
        },
        "semver" => Builtin {
            base: "str",
            import: None,
            constraints: vec![("pattern", python_string(SEMVER_PATTERN))],
        },
        "bool" => simple("bool", None),
        "number" | "f32" | "f64" => simple("float", None),
        "integer" | "bigint" => simple("int", None),
        "decimal" => simple("Decimal", Some(("decimal", "Decimal"))),
        "uuid" => simple("UUID", Some(("uuid", "UUID"))),
        "email" => simple("EmailStr", Some(("pydantic", "EmailStr"))),
        "url" => simple("AnyUrl", Some(("pydantic", "AnyUrl"))),
        "ip" => simple("IPvAnyAddress", Some(("pydantic", "IPvAnyAddress"))),
        "ipv4" => simple("IPv4Address", Some(("ipaddress", "IPv4Address"))),
        "ipv6" => simple("IPv6Address", Some(("ipaddress", "IPv6Address"))),
        "date" => simple("date", Some(("datetime", "date"))),
        "datetime" => simple("datetime", Some(("datetime", "datetime"))),
        "duration" => simple("timedelta", Some(("datetime", "timedelta"))),
        "binary" => simple("Base64Bytes", Some(("pydantic", "Base64Bytes"))),
        "regex" => simple("Pattern", Some(("re", "Pattern"))),
        "any" => simple("Any", Some(("typing", "Any"))),
        "null" => simple("None", None),
        "NEVER" => simple("Never", Some(("typing", "Never"))),
        _ => return sized_integer(name),
    })
}

fn sized_integer(name: &str) -> Option<Builtin> {
    let signed = match name.as_bytes().first() {
        Some(b'i') => true,
        Some(b'u') => false,
        _ => return None,
    };
    let bits: u32 = name.get(1..)?.parse().ok()?;
    if !matches!(bits, 4 | 8 | 16 | 32 | 64 | 128) {
        return None;
    }
    let (minimum, maximum) = match signed {
        true => {
            let half = BigInt::from(1_u8) << (bits - 1);
            (-half.clone(), half - 1_u8)
        }
        false => (BigInt::from(0_u8), (BigInt::from(1_u8) << bits) - 1_u8),
    };
    Some(Builtin {
        base: "int",
        import: None,
        constraints: vec![("ge", minimum.to_string()), ("le", maximum.to_string())],
    })
}

fn has_native_python_mapping(name: &str) -> bool {
    name == "Dict" || builtin(name).is_some()
}

// ── Generated code fragments ────────────────────────────────────────

/// Ordered `Field(...)` keyword arguments.
#[derive(Default)]
struct FieldArgs(Vec<(String, String)>);

impl FieldArgs {
    fn set(&mut self, key: &str, value: String) {
        match self.0.iter_mut().find(|(existing, _)| existing == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// Keeps the stricter of two numeric bounds, so a builtin range such as
    /// `u8`'s `le=255` is never loosened by a user annotation.
    fn tighten(&mut self, key: &str, value: String, keep_greater: bool) {
        if let Some((_, existing)) = self.0.iter().find(|(existing, _)| existing == key) {
            if let (Ok(old), Ok(new)) =
                (BigDecimal::from_str(existing), BigDecimal::from_str(&value))
            {
                if (keep_greater && new <= old) || (!keep_greater && new >= old) {
                    return;
                }
            }
        }
        self.set(key, value);
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Default)]
struct Imports {
    modules: BTreeMap<String, BTreeSet<String>>,
    /// Module-level names and where they come from; `None` marks a name
    /// declared by the generated module itself.
    locals: HashMap<String, Option<(String, String)>>,
}

impl Imports {
    fn reserve(&mut self, name: &str) {
        self.locals.entry(name.to_string()).or_insert(None);
    }

    /// Imports `symbol` from `module` and returns its local name, aliasing it
    /// when another declaration or import already owns that name.
    fn symbol(&mut self, module: &str, symbol: &str) -> String {
        let key = Some((module.to_string(), symbol.to_string()));
        let mut local = symbol.to_string();
        let mut suffix = 2;
        loop {
            match self.locals.get(&local) {
                None => {
                    self.locals.insert(local.clone(), key);
                    break;
                }
                Some(existing) if *existing == key => break,
                Some(_) => {
                    local = format!("{symbol}_{suffix}");
                    suffix += 1;
                }
            }
        }
        let entry = match local == symbol {
            true => symbol.to_string(),
            false => format!("{symbol} as {local}"),
        };
        self.modules
            .entry(module.to_string())
            .or_default()
            .insert(entry);
        local
    }

    fn render(&self, package: Option<&str>) -> String {
        let is_local = |module: &str| {
            module.starts_with('.')
                || package.is_some_and(|package| {
                    module == package || module.starts_with(&format!("{package}."))
                })
        };
        let mut groups: [Vec<String>; 3] = Default::default();
        for (module, names) in &self.modules {
            let group = match () {
                _ if STDLIB_MODULES.contains(&module.as_str()) => 0,
                _ if is_local(module) => 2,
                _ => 1,
            };
            groups[group].push(format!(
                "from {module} import {}\n",
                names.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        groups
            .iter()
            .filter(|group| !group.is_empty())
            .map(|group| group.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── Generator listener ──────────────────────────────────────────────

struct PydanticGenerator {
    source: String,
    workdir: PathBuf,
    output_dir: Option<PathBuf>,
    package: Option<String>,
    default_extra: Option<String>,
    abs_path: PathBuf,
    module_path: String,
    imported_types: HashMap<String, Vec<String>>,
    import_selections: HashMap<String, Vec<String>>,
    type_hierarchy: TypeHierarchy,
    imports: RefCell<Imports>,
    diagnostics: RefCell<Vec<XenoDiagnostic>>,
    /// Declaration currently being generated, used to locate diagnostics.
    location: Cell<TokenData>,
    /// Local aliases marked `@deprecated`, keyed by name, so model fields
    /// that use them can carry Pydantic's field-level `deprecated`.
    deprecated_aliases: HashMap<String, String>,
    body: Vec<String>,
    exports: Vec<String>,
    annotation_depth: usize,
}

impl PydanticGenerator {
    fn new() -> Self {
        Self {
            source: String::new(),
            workdir: PathBuf::new(),
            output_dir: None,
            package: None,
            default_extra: None,
            abs_path: PathBuf::new(),
            module_path: String::new(),
            imported_types: HashMap::new(),
            import_selections: HashMap::new(),
            type_hierarchy: TypeHierarchy::default(),
            imports: RefCell::new(Imports::default()),
            diagnostics: RefCell::new(Vec::new()),
            location: Cell::new(TokenData::new(0, 0)),
            deprecated_aliases: HashMap::new(),
            body: Vec::new(),
            exports: Vec::new(),
            annotation_depth: 0,
        }
    }

    fn symbol(&self, module: &str, symbol: &str) -> String {
        self.imports.borrow_mut().symbol(module, symbol)
    }

    fn report(&self, severity: XenoDiagSeverity, message: String) {
        self.diagnostics
            .borrow_mut()
            .push(diagnostic(self.location.get(), severity, message));
    }

    fn render_file(&self) -> String {
        let mut output = String::from(
            "# Auto-generated by xenomorph-pydantic — do not edit.\nfrom __future__ import annotations\n",
        );
        let imports = self.imports.borrow().render(self.package.as_deref());
        if !imports.is_empty() {
            output.push('\n');
            output.push_str(&imports);
        }
        if !self.exports.is_empty() {
            output.push_str("\n__all__ = [\n");
            for name in &self.exports {
                output.push_str(&format!("    {},\n", python_string(name)));
            }
            output.push_str("]\n");
        }
        for block in &self.body {
            output.push_str("\n\n");
            output.push_str(block.trim_end());
            output.push('\n');
        }
        output
    }

    // ── Declarations ──

    fn generate_declaration(&mut self, declaration: &Declaration) {
        match declaration {
            Declaration::Type {
                public,
                docs,
                name,
                generics,
                ty,
                ..
            } => {
                self.location.set(*name);
                let name = name.text(&self.source).to_string();
                let params: Vec<String> = generics
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|(param, _)| param.text(&self.source).to_string())
                    .collect();
                let (code, extra_exports) =
                    self.type_declaration(&name, &params, docs.as_deref(), &ty.0, &ty.1);
                if public.is_some() {
                    self.exports.push(name);
                    self.exports.extend(extra_exports);
                }
                self.body.push(code);
            }
            Declaration::Custom {
                decl_id, content, ..
            } if decl_id == CUSTOM_IDENTIFIER => {
                let code = dedent_block(content.text(&self.source));
                if !code.trim().is_empty() {
                    self.body.push(code);
                }
            }
            Declaration::Import {
                public: Some(_),
                path,
                names,
                ..
            } => {
                for name in names {
                    match name.text(&self.source) {
                        "*" => self.exports.extend(
                            self.import_selections
                                .get(&path.join("/"))
                                .cloned()
                                .unwrap_or_default(),
                        ),
                        name => self.exports.push(name.to_string()),
                    }
                }
            }
            _ => {}
        }
    }

    /// Returns the generated code and any additional public class names.
    fn type_declaration(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        ty: &Type,
        annotations: &[Annotation],
    ) -> (String, Vec<String>) {
        match ty {
            Type::Struct(fields) => {
                let fields = self.ast_fields(fields);
                (self.model(name, params, docs, &fields, annotations), vec![])
            }
            Type::Enum(variants) => self.enumeration(name, params, docs, variants, annotations),
            Type::Intersection(_) => self.struct_shaped(name, params, docs, ty, annotations),
            Type::Simple(SimpleType::Identifier(identifier, Some(_)))
                if STRUCT_UTILITIES.contains(&identifier.text(&self.source)) =>
            {
                self.struct_shaped(name, params, docs, ty, annotations)
            }
            _ => (self.alias(name, params, docs, ty, annotations), vec![]),
        }
    }

    /// Intersections and struct utilities become models with their resolved
    /// fields; anything else falls back to an alias.
    fn struct_shaped(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        ty: &Type,
        annotations: &[Annotation],
    ) -> (String, Vec<String>) {
        let owned = type_to_owned_type(ty, &self.source);
        match self.type_hierarchy.struct_fields(&owned) {
            Ok(Some(fields)) => (self.model(name, params, docs, &fields, annotations), vec![]),
            Ok(None) => (self.alias(name, params, docs, ty, annotations), vec![]),
            Err(error) => {
                self.report(XenoDiagSeverity::Err, error.to_string());
                (self.alias(name, params, docs, ty, annotations), vec![])
            }
        }
    }

    fn ast_fields(&self, fields: &[KeyValExpr]) -> Vec<OwnedField> {
        fields
            .iter()
            .map(|(key, value, docs)| OwnedField {
                name: key.text(&self.source).to_string(),
                ty: simple_to_owned_type(value, &self.source),
                documentation: docs.map(|docs| extract_documentation(&docs, &self.source)),
            })
            .collect()
    }

    fn class_header(&self, name: &str, params: &[String], base: &str) -> String {
        match params.is_empty() {
            true => format!("class {name}({base}):\n"),
            false => format!("class {name}[{}]({base}):\n", params.join(", ")),
        }
    }

    fn model(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        fields: &[OwnedField],
        annotations: &[Annotation],
    ) -> String {
        let mut config = FieldArgs::default();
        let mut validators: Vec<(&str, String)> = Vec::new();
        let mut deprecated = None;
        for annotation in annotations {
            let annotation_name = annotation_name(annotation, &self.source);
            let flag = |config: &mut FieldArgs, key: &str| config.set(key, "True".into());
            match annotation_name {
                "deprecated" => deprecated = Some(deprecation_message(annotation)),
                "pydantic::extra" => {
                    if let Some((value, _)) = string_param(annotation, 0) {
                        config.set("extra", python_string(value));
                    }
                }
                "pydantic::frozen" => flag(&mut config, "frozen"),
                "pydantic::strict" => flag(&mut config, "strict"),
                "pydantic::populate_by_name" => flag(&mut config, "populate_by_name"),
                "pydantic::validate_assignment" => flag(&mut config, "validate_assignment"),
                "pydantic::use_enum_values" => flag(&mut config, "use_enum_values"),
                "pydantic::str_strip_whitespace" => flag(&mut config, "str_strip_whitespace"),
                "pydantic::alias_generator" => {
                    let generator = string_param(annotation, 0).and_then(|(value, _)| {
                        ALIAS_GENERATORS.iter().find(|(key, _)| *key == value)
                    });
                    if let Some((_, function)) = generator {
                        let function = self.symbol("pydantic.alias_generators", function);
                        config.set("alias_generator", function);
                    }
                }
                "pydantic::title" => {
                    if let Some((value, _)) = string_param(annotation, 0) {
                        config.set("title", python_string(value));
                    }
                }
                "pydantic::config" => {
                    if let (Some((key, _)), Some(value)) =
                        (string_param(annotation, 0), literal_param(annotation, 1))
                    {
                        if is_python_identifier(key) {
                            config.set(key, value);
                        }
                    }
                }
                "pydantic::after_validator"
                | "pydantic::before_validator"
                | "pydantic::wrap_validator" => {
                    if let Some(function) = self.validator_function(annotation) {
                        let mode = annotation_name
                            .trim_start_matches("pydantic::")
                            .trim_end_matches("_validator");
                        validators.push((mode, function));
                    }
                }
                "pydantic::plain_validator" => self.report(
                    XenoDiagSeverity::Err,
                    format!("@{annotation_name} cannot replace validation of model '{name}'."),
                ),
                "if" | "elseif" | "else" => self.report_conditional(annotation_name),
                _ => {}
            }
        }
        if !config.0.iter().any(|(key, _)| key == "extra") {
            if let Some(extra) = &self.default_extra {
                config.set("extra", python_string(extra));
            }
        }

        let mut used = HashSet::new();
        let mut lines = Vec::new();
        let mut aliased = false;
        for field in fields {
            let python_name = python_field_name(&field.name, &mut used);
            aliased |= python_name != field.name;
            lines.push(self.field_line(
                &python_name,
                &field.name,
                &field.ty,
                field.documentation.as_deref(),
            ));
        }
        if aliased && !config.0.iter().any(|(key, _)| key == "populate_by_name") {
            config.set("populate_by_name", "True".into());
        }

        let base_model = self.symbol("pydantic", "BaseModel");
        let mut output = String::new();
        if let Some(message) = &deprecated {
            let decorator = self.symbol("typing_extensions", "deprecated");
            output.push_str(&format!("@{decorator}({})\n", python_string(message)));
        }
        output.push_str(&self.class_header(name, params, &base_model));
        let mut sections = Vec::new();
        if let Some(docs) = docs.filter(|docs| !docs.trim().is_empty()) {
            sections.push(python_docstring(docs, "    "));
        }
        if !config.is_empty() {
            let config_dict = self.symbol("pydantic", "ConfigDict");
            sections.push(format!(
                "    model_config = {config_dict}({})\n",
                config.render()
            ));
        }
        if !lines.is_empty() {
            sections.push(lines.concat());
        }
        for (index, (mode, function)) in validators.iter().enumerate() {
            let decorator = self.symbol("pydantic", "model_validator");
            sections.push(match *mode {
                "after" => format!(
                    "    @{decorator}(mode=\"after\")\n    def _xeno_after_validator_{index}(self):\n        return {function}(self)\n"
                ),
                "before" => format!(
                    "    @{decorator}(mode=\"before\")\n    @classmethod\n    def _xeno_before_validator_{index}(cls, data):\n        return {function}(data)\n"
                ),
                _ => format!(
                    "    @{decorator}(mode=\"wrap\")\n    @classmethod\n    def _xeno_wrap_validator_{index}(cls, data, handler):\n        return {function}(data, handler)\n"
                ),
            });
        }
        if sections.is_empty() {
            sections.push("    pass\n".into());
        }
        output.push_str(&sections.join("\n"));
        output
    }

    fn field_line(
        &self,
        python_name: &str,
        wire_name: &str,
        ty: &OwnedType,
        docs: Option<&str>,
    ) -> String {
        let rendered = self.render(ty);
        let optional = self.is_optional(ty, 0);
        let mut args = FieldArgs::default();
        if optional {
            args.set("default", "None".into());
        }
        if python_name != wire_name {
            args.set("alias", python_string(wire_name));
        }
        if let Some(docs) = docs.map(str::trim).filter(|docs| !docs.is_empty()) {
            args.set("description", python_string(docs));
        }
        if let Some(reason) = self.deprecated_reference(ty) {
            args.set("deprecated", python_string(reason));
        }
        match args.0.as_slice() {
            [] => format!("    {python_name}: {rendered}\n"),
            [(key, _)] if key == "default" => format!("    {python_name}: {rendered} = None\n"),
            _ => {
                let field = self.symbol("pydantic", "Field");
                format!(
                    "    {python_name}: {rendered} = {field}({})\n",
                    args.render()
                )
            }
        }
    }

    fn deprecated_reference(&self, ty: &OwnedType) -> Option<&str> {
        match ty {
            OwnedType::Optional(inner) => self.deprecated_reference(inner),
            OwnedType::Named { name, .. } => self.deprecated_aliases.get(name).map(String::as_str),
            _ => None,
        }
    }

    /// Whether a field type admits `None` at its top level, following
    /// aliases such as `type A = ?SomeType;`. Such fields default to `None`
    /// so they may be absent from the input.
    fn is_optional(&self, ty: &OwnedType, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match ty {
            OwnedType::Optional(_) => true,
            OwnedType::Sum(items) => items.iter().any(|item| self.is_optional(item, depth + 1)),
            OwnedType::Named { name, arguments } if name == "null" && arguments.is_empty() => true,
            OwnedType::Named { name, arguments }
            | OwnedType::Qualified {
                name, arguments, ..
            } => {
                let Some(definition) = self.type_hierarchy.get_type(name) else {
                    return false;
                };
                let Some(body) = &definition.body else {
                    return false;
                };
                let substituted = match body {
                    OwnedType::Named { name, .. } | OwnedType::Generic { name, .. } => definition
                        .generic_params
                        .iter()
                        .position(|param| param.name == *name)
                        .and_then(|index| arguments.get(index)),
                    _ => None,
                };
                self.is_optional(substituted.unwrap_or(body), depth + 1)
            }
            _ => false,
        }
    }

    fn validator_function(&self, annotation: &Annotation) -> Option<String> {
        let (path, _) = string_param(annotation, 0)?;
        match split_function_path(path) {
            Some((module, function)) => Some(self.symbol(module, function)),
            None => {
                self.report(
                    XenoDiagSeverity::Err,
                    format!("Validator path \"{path}\" is not a dotted Python import path."),
                );
                None
            }
        }
    }

    fn report_conditional(&self, annotation_name: &str) {
        self.report(
            XenoDiagSeverity::Warn,
            format!("Pydantic output does not enforce conditional @{annotation_name} validation."),
        );
    }

    /// Applies native and Pydantic annotations of a non-model declaration.
    /// Returns extra `Annotated` metadata and an optional deprecation reason.
    fn alias_metadata(
        &self,
        annotations: &[Annotation],
        args: &mut FieldArgs,
    ) -> (Vec<String>, Option<String>) {
        let mut metadata = Vec::new();
        let mut deprecated = None;
        for annotation in annotations {
            let name = annotation_name(annotation, &self.source);
            let number = || number_param(annotation, 0);
            match name {
                "min" => number()
                    .into_iter()
                    .for_each(|value| args.tighten("ge", value, true)),
                "max" => number()
                    .into_iter()
                    .for_each(|value| args.tighten("le", value, false)),
                "gt" => number().into_iter().for_each(|value| args.set("gt", value)),
                "lt" => number().into_iter().for_each(|value| args.set("lt", value)),
                "multipleof" => number()
                    .into_iter()
                    .for_each(|value| args.set("multiple_of", value)),
                "len" => number().into_iter().for_each(|value| {
                    args.set("min_length", value.clone());
                    args.set("max_length", value);
                }),
                "minlen" => number()
                    .into_iter()
                    .for_each(|value| args.tighten("min_length", value, true)),
                "maxlen" => number()
                    .into_iter()
                    .for_each(|value| args.tighten("max_length", value, false)),
                "match" => {
                    if let Some(Expr::Regex(token)) = annotation.params.first() {
                        args.set("pattern", python_regex(token.text(&self.source)));
                    }
                }
                "deprecated" => deprecated = Some(deprecation_message(annotation)),
                "if" | "elseif" | "else" => self.report_conditional(name),
                "pydantic::strict" => args.set("strict", "True".into()),
                "pydantic::title" => {
                    if let Some((value, _)) = string_param(annotation, 0) {
                        args.set("title", python_string(value));
                    }
                }
                "pydantic::discriminator" => {
                    if let Some((value, _)) = string_param(annotation, 0) {
                        args.set("discriminator", python_string(value));
                    }
                }
                "pydantic::after_validator"
                | "pydantic::before_validator"
                | "pydantic::plain_validator"
                | "pydantic::wrap_validator" => {
                    if let Some(function) = self.validator_function(annotation) {
                        let class = match name {
                            "pydantic::after_validator" => "AfterValidator",
                            "pydantic::before_validator" => "BeforeValidator",
                            "pydantic::plain_validator" => "PlainValidator",
                            _ => "WrapValidator",
                        };
                        let class = self.symbol("pydantic", class);
                        metadata.push(format!("{class}({function})"));
                    }
                }
                _ => {}
            }
        }
        (metadata, deprecated)
    }

    fn annotated(&self, base: String, args: &FieldArgs, metadata: Vec<String>) -> String {
        if args.is_empty() && metadata.is_empty() {
            return base;
        }
        let mut items = vec![base];
        if !args.is_empty() {
            let field = self.symbol("pydantic", "Field");
            items.push(format!("{field}({})", args.render()));
        }
        items.extend(metadata);
        let annotated = self.symbol("typing", "Annotated");
        format!("{annotated}[{}]", items.join(", "))
    }

    fn alias_header(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        deprecated: Option<&str>,
    ) -> String {
        let mut output = String::new();
        if let Some(docs) = docs {
            output.push_str(&python_comment(docs, ""));
        }
        if let Some(reason) = deprecated {
            output.push_str(&python_comment(&format!("Deprecated: {reason}"), ""));
        }
        match params.is_empty() {
            true => output.push_str(&format!("type {name} = ")),
            false => output.push_str(&format!("type {name}[{}] = ", params.join(", "))),
        }
        output
    }

    fn alias(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        ty: &Type,
        annotations: &[Annotation],
    ) -> String {
        // Constraints of `?T` apply to `T`, keeping the alias `T | None`.
        let (inner, optional) = match ty {
            Type::Simple(SimpleType::Optional(inner)) => (Type::Simple((**inner).clone()), true),
            other => (other.clone(), false),
        };
        let mut args = FieldArgs::default();
        let base = match &inner {
            Type::Simple(SimpleType::Identifier(identifier, None)) => {
                match builtin(identifier.text(&self.source)) {
                    Some(builtin) => {
                        for (key, value) in builtin.constraints.iter().cloned() {
                            args.set(key, value);
                        }
                        self.builtin_base(&builtin)
                    }
                    None => self.render(&type_to_owned_type(&inner, &self.source)),
                }
            }
            _ => self.render(&type_to_owned_type(&inner, &self.source)),
        };
        let (metadata, deprecated) = self.alias_metadata(annotations, &mut args);
        let mut expression = self.annotated(base, &args, metadata);
        if optional {
            expression.push_str(" | None");
        }
        format!(
            "{}{expression}\n",
            self.alias_header(name, params, docs, deprecated.as_deref())
        )
    }

    fn enumeration(
        &self,
        name: &str,
        params: &[String],
        docs: Option<&str>,
        variants: &[KeyValExpr],
        annotations: &[Annotation],
    ) -> (String, Vec<String>) {
        let numeric = !variants.is_empty()
            && variants.iter().all(|(_, value, _)| {
                matches!(
                    value,
                    SimpleType::Literal(Literal::Int(_) | Literal::Float(_))
                )
            });
        if numeric {
            return (self.value_enum(name, docs, variants, annotations), vec![]);
        }

        // Algebraic enums use the `{ "kind": ..., "value": ... }` wire format.
        let base_model = self.symbol("pydantic", "BaseModel");
        let config_dict = self.symbol("pydantic", "ConfigDict");
        let literal = self.symbol("typing", "Literal");
        let arguments = match params.is_empty() {
            true => String::new(),
            false => format!("[{}]", params.join(", ")),
        };
        let mut classes = Vec::new();
        let mut output = String::new();
        for (key, value, variant_docs) in variants {
            let key = key.text(&self.source);
            let class_name = format!("{name}_{}", python_enum_member(key, &mut HashSet::new()));
            output.push_str(&self.class_header(&class_name, params, &base_model));
            if let Some(docs) = variant_docs {
                output.push_str(&python_docstring(
                    &extract_documentation(docs, &self.source),
                    "    ",
                ));
            }
            output.push_str(&format!(
                "    model_config = {config_dict}(extra=\"forbid\")\n"
            ));
            output.push_str(&format!(
                "    kind: {literal}[{key}] = {key}\n",
                key = python_string(key)
            ));
            output.push_str(&self.field_line(
                "value",
                "value",
                &simple_to_owned_type(value, &self.source),
                None,
            ));
            output.push_str("\n\n");
            classes.push(format!("{class_name}{arguments}"));
        }
        let mut args = FieldArgs::default();
        if classes.len() > 1 {
            args.set("discriminator", python_string("kind"));
        }
        let (metadata, deprecated) = self.alias_metadata(annotations, &mut args);
        let union = match classes.is_empty() {
            true => self.symbol("typing", "Never"),
            false => classes.join(" | "),
        };
        output.push_str(&self.alias_header(name, params, docs, deprecated.as_deref()));
        output.push_str(&self.annotated(union, &args, metadata));
        output.push('\n');
        let exports = classes
            .iter()
            .map(|class| class.split('[').next().unwrap_or(class).to_string())
            .collect();
        (output, exports)
    }

    fn value_enum(
        &self,
        name: &str,
        docs: Option<&str>,
        variants: &[KeyValExpr],
        annotations: &[Annotation],
    ) -> String {
        let integers = variants
            .iter()
            .all(|(_, value, _)| matches!(value, SimpleType::Literal(Literal::Int(_))));
        let base = match integers {
            true => self.symbol("enum", "IntEnum"),
            false => self.symbol("enum", "Enum"),
        };
        let mut output = String::new();
        for annotation in annotations {
            if annotation_name(annotation, &self.source) == "deprecated" {
                output.push_str(&python_comment(
                    &format!("Deprecated: {}", deprecation_message(annotation)),
                    "",
                ));
            }
        }
        output.push_str(&format!("class {name}({base}):\n"));
        if let Some(docs) = docs.filter(|docs| !docs.trim().is_empty()) {
            output.push_str(&python_docstring(docs, "    "));
        }
        let mut used = HashSet::new();
        for (key, value, member_docs) in variants {
            if let Some(member_docs) = member_docs {
                output.push_str(&python_comment(
                    &extract_documentation(member_docs, &self.source),
                    "    ",
                ));
            }
            let value = match value {
                SimpleType::Literal(Literal::Int(integer)) => integer.value.to_string(),
                SimpleType::Literal(Literal::Float(float)) => float.value.to_string(),
                _ => unreachable!("value enums contain only numeric literals"),
            };
            output.push_str(&format!(
                "    {} = {value}\n",
                python_enum_member(key.text(&self.source), &mut used)
            ));
        }
        output
    }

    // ── Type expressions ──

    fn builtin_base(&self, builtin: &Builtin) -> String {
        match builtin.import {
            Some((module, symbol)) if symbol == "Pattern" => {
                format!("{}[str]", self.symbol(module, symbol))
            }
            Some((module, symbol)) => self.symbol(module, symbol),
            None => builtin.base.to_string(),
        }
    }

    fn render(&self, ty: &OwnedType) -> String {
        match ty {
            OwnedType::Named { name, arguments } => self.render_named(ty, name, arguments),
            OwnedType::Qualified {
                name, arguments, ..
            } => self.render_reference(name, arguments),
            OwnedType::Array(inner) => format!("list[{}]", self.render(inner)),
            OwnedType::Optional(inner) => format!("{} | None", self.render(inner)),
            OwnedType::Literal(literal) => {
                self.render_union(&[OwnedType::Literal(literal.clone())])
            }
            OwnedType::Tuple(items) if items.is_empty() => "tuple[()]".to_string(),
            OwnedType::Tuple(items) => format!(
                "tuple[{}]",
                items
                    .iter()
                    .map(|item| self.render(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            OwnedType::Set(set) => match (&set.values, &set.element_type) {
                (Some(values), _) if values.is_empty() => self.symbol("typing", "Never"),
                (Some(values), _) => self.render_union(
                    &values
                        .iter()
                        .cloned()
                        .map(OwnedType::Literal)
                        .collect::<Vec<_>>(),
                ),
                (None, Some(element)) => format!("set[{}]", self.render(element)),
                (None, None) => self.symbol("typing", "Any"),
            },
            OwnedType::Sum(items) => self.render_union(items),
            OwnedType::Struct(_) | OwnedType::Enum(_) => {
                self.report(
                    XenoDiagSeverity::Err,
                    "Pydantic cannot inline an anonymous struct or enum; declare it as a named type."
                        .into(),
                );
                format!("dict[str, {}]", self.symbol("typing", "Any"))
            }
            OwnedType::Intersection(items) => {
                self.report(
                    XenoDiagSeverity::Warn,
                    "Pydantic has no intersection type; only the first member is enforced. Declare struct intersections as named types to merge their fields."
                        .into(),
                );
                items
                    .first()
                    .map(|item| self.render(item))
                    .unwrap_or_else(|| self.symbol("typing", "Any"))
            }
            OwnedType::Generic { name, .. } => name.clone(),
        }
    }

    fn render_named(&self, ty: &OwnedType, name: &str, arguments: &[OwnedType]) -> String {
        if let ("Dict", [key, value]) = (name, arguments) {
            return format!("dict[{}, {}]", self.render(key), self.render(value));
        }
        if TYPE_UTILITIES.contains(&name) && !arguments.is_empty() {
            let resolved = self.type_hierarchy.resolve_for_target(ty);
            if resolved != *ty {
                return self.render(&resolved);
            }
            self.report(
                XenoDiagSeverity::Err,
                format!(
                    "Pydantic cannot evaluate type utility '{}'.",
                    ty.display_name()
                ),
            );
            return self.symbol("typing", "Any");
        }
        if arguments.is_empty() {
            if let Some(builtin) = builtin(name) {
                let base = self.builtin_base(&builtin);
                let mut args = FieldArgs::default();
                for (key, value) in builtin.constraints {
                    args.set(key, value);
                }
                return self.annotated(base, &args, vec![]);
            }
        }
        self.render_reference(name, arguments)
    }

    fn render_reference(&self, name: &str, arguments: &[OwnedType]) -> String {
        match arguments.is_empty() {
            true => name.to_string(),
            false => format!(
                "{name}[{}]",
                arguments
                    .iter()
                    .map(|argument| self.render(argument))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Renders a union, merging literal members into a single `Literal[...]`
    /// and moving every optional marker into one trailing `None`.
    fn render_union(&self, items: &[OwnedType]) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut literals: Vec<String> = Vec::new();
        let mut literal_position = None;
        let mut nullable = false;
        for item in items {
            let item = match item {
                OwnedType::Optional(inner) => {
                    nullable = true;
                    inner.as_ref()
                }
                item => item,
            };
            match item {
                OwnedType::Literal(OwnedLiteral::Float { value, .. }) => {
                    let mut args = FieldArgs::default();
                    args.set("ge", value.to_string());
                    args.set("le", value.to_string());
                    parts.push(self.annotated("float".into(), &args, vec![]));
                }
                OwnedType::Literal(literal) => {
                    literal_position.get_or_insert(parts.len());
                    literals.push(match literal {
                        OwnedLiteral::String(value) => python_string(value),
                        OwnedLiteral::Integer { value, .. } => value.to_string(),
                        OwnedLiteral::Boolean(value) => python_bool(*value).to_string(),
                        OwnedLiteral::Float { .. } => unreachable!("handled above"),
                    });
                }
                OwnedType::Named { name, arguments } if name == "null" && arguments.is_empty() => {
                    nullable = true;
                }
                item => {
                    let rendered = self.render(item);
                    if let Some(stripped) = rendered.strip_suffix(" | None") {
                        nullable = true;
                        parts.push(stripped.to_string());
                    } else {
                        parts.push(rendered);
                    }
                }
            }
        }
        if let Some(position) = literal_position {
            let literal = self.symbol("typing", "Literal");
            parts.insert(position, format!("{literal}[{}]", literals.join(", ")));
        }
        if nullable || parts.is_empty() {
            parts.push("None".into());
        }
        parts.join(" | ")
    }
}

impl<'src> AnalyzerListener<'src> for PydanticGenerator {
    fn set_source(&mut self, source: &str) {
        self.source.clear();
        self.source.push_str(source);
    }

    fn on_before_module(&mut self, scope: &ScopeInfo) {
        self.abs_path = scope.abs_path.clone();
        self.module_path = scope.module_path.clone();
        self.imported_types = scope.imported_types.clone();
        self.import_selections = scope.import_selections.clone();
        self.type_hierarchy = scope.type_hierarchy.clone();
        self.annotation_depth = 0;
        self.body.clear();
        self.exports.clear();
        self.deprecated_aliases.clear();
        self.diagnostics.borrow_mut().clear();

        let mut imports = Imports::default();
        for name in &scope.own_types {
            imports.reserve(name);
        }
        let mut modules: Vec<_> = self.imported_types.iter().collect();
        modules.sort_by_key(|(module, _)| *module);
        for (module, names) in modules {
            let path = python_import_path(&self.module_path, module, self.package.as_deref());
            for name in names {
                imports.reserve(name);
                imports
                    .modules
                    .entry(path.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
        self.imports = RefCell::new(imports);
    }

    fn on_before_ast(&mut self, ast: &[Declaration], errors: &mut Vec<XenoDiagnostic>) {
        for declaration in ast {
            if let Declaration::Type { name, ty, .. } = declaration {
                self.imports.borrow_mut().reserve(name.text(&self.source));
                let is_alias = !matches!(ty.0, Type::Struct(_));
                let deprecated = ty
                    .1
                    .iter()
                    .find(|annotation| annotation_name(annotation, &self.source) == "deprecated");
                if let (true, Some(annotation)) = (is_alias, deprecated) {
                    self.deprecated_aliases.insert(
                        name.text(&self.source).to_string(),
                        deprecation_message(annotation),
                    );
                }
            }
        }
        for declaration in ast {
            self.generate_declaration(declaration);
        }
        errors.append(&mut self.diagnostics.borrow_mut());
    }

    fn on_simple_type(&mut self, ty: &SimpleType, errors: &mut Vec<XenoDiagnostic>) {
        if self.annotation_depth > 0 {
            return;
        }
        if let Some(diagnostic) = unsupported_target_type_diagnostic(
            "Pydantic",
            ty,
            &self.type_hierarchy,
            &self.source,
            has_native_python_mapping,
        ) {
            errors.push(diagnostic);
        }
    }

    fn on_before_annotation(
        &mut self,
        _name: &TokenData,
        _args: &[Expr],
        _errors: &mut Vec<XenoDiagnostic>,
    ) {
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData,
        _args: &[Expr],
        _errors: &mut Vec<XenoDiagnostic>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }

    fn on_after_module(&mut self, scope: &ScopeInfo) {
        let out_path = match &self.output_dir {
            Some(directory) => {
                let filename = format!(
                    "{}.py",
                    scope
                        .module_path
                        .replace('/', std::path::MAIN_SEPARATOR_STR)
                );
                let path = self.workdir.join(directory).join(filename);
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                path
            }
            None => self.abs_path.with_extension("py"),
        };
        match fs::write(&out_path, self.render_file()) {
            Ok(_) => println!("✓ {} → {}", scope.module_path, out_path.display()),
            Err(error) => eprintln!("✗ {} — failed to write: {}", scope.module_path, error),
        }
    }
}

#[cfg(test)]
mod tests;
