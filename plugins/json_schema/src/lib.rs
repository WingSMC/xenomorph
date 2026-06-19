use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use xenomorph_common::config::{ConfigValue, PluginConfigs};
use xenomorph_common::parser::{
    AnonymType, BinaryExprType, Declaration, Expr, KeyValExpr, Literal, NumberType,
};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::semantic::{AnalyzerListener, ScopeInfo};

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "json_schema";
static VERSION: &str = "0.1.0";
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    version: VERSION,
    initialize: None,
    provide_types: None,
    provide_annotations: None,
    register_generator: Some(create_generator),
    register_analyzer: None,
};

fn create_generator() -> Box<dyn for<'a> AnalyzerListener<'a>> {
    Box::new(JsonSchemaGenerator::new())
}

#[no_mangle]
fn load() -> &'static XenoPlugin<'static> {
    &PLUGIN
}

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

// ── Generator listener ──────────────────────────────────────────────

struct JsonSchemaGenerator {
    /// Accumulated `$defs` for the current module.
    defs: Map<String, Value>,
    abs_path: PathBuf,
    module_path: String,
    /// Output directory override from `[plugins.json_schema].output`.
    /// If None, writes `.schema.json` files next to the `.xen` source files.
    output_dir: Option<PathBuf>,
    /// Imported types keyed by module path, for resolving `$ref` targets.
    imported_types: HashMap<String, Vec<String>>,
}

impl JsonSchemaGenerator {
    fn new() -> Self {
        Self {
            defs: Map::new(),
            abs_path: PathBuf::new(),
            module_path: String::new(),
            output_dir: None,
            imported_types: HashMap::new(),
        }
    }

    /// Resolves the module path that provides a given (non-builtin) type name.
    fn provider_of(&self, name: &str) -> Option<&str> {
        for (module_path, names) in &self.imported_types {
            if names.iter().any(|n| n == name) {
                return Some(module_path.as_str());
            }
        }
        None
    }

    /// Builds a `$ref` value pointing at a named type, resolving cross-module
    /// references to a relative `.schema.json` file path.
    fn ref_for(&self, name: &str) -> Value {
        match self.provider_of(name) {
            Some(provider) => {
                let rel = schema_ref_path(&self.module_path, provider);
                json!({ "$ref": format!("{rel}#/$defs/{name}") })
            }
            None => json!({ "$ref": format!("#/$defs/{name}") }),
        }
    }
}

impl<'src> AnalyzerListener<'src> for JsonSchemaGenerator {
    fn on_init(&mut self, plugin_configs: &PluginConfigs) {
        if let Some(ConfigValue::Table(cfg)) = plugin_configs.get("json_schema") {
            if let Some(ConfigValue::String(output)) = cfg.get("output") {
                self.output_dir = Some(PathBuf::from(output));
            }
        }
    }

    fn on_before_module(&mut self, scope: &ScopeInfo) {
        self.abs_path = scope.abs_path.clone();
        self.module_path = scope.module_path.clone();
        self.imported_types = scope.imported_types.clone();
        self.defs.clear();
    }

    fn on_before_ast(
        &mut self,
        ast: &[Declaration<'src>],
        _errors: &mut Vec<xenomorph_common::XenoError<'src>>,
    ) {
        for decl in ast {
            if let Declaration::TypeDecl { docs, name, t } = decl {
                let schema = self.type_decl_to_schema(docs, name.v, t);
                self.defs.insert(name.v.to_string(), schema);
            }
        }
    }

    fn on_after_module(&mut self, scope: &ScopeInfo) {
        let document = json!({
            "$schema": DRAFT,
            "$id": format!("{}.schema.json", scope.module_path),
            "$defs": Value::Object(self.defs.clone()),
        });

        let out_path = match &self.output_dir {
            Some(dir) => {
                let filename = format!(
                    "{}.schema.json",
                    scope
                        .module_path
                        .replace('/', std::path::MAIN_SEPARATOR_STR)
                );
                let path = dir.join(filename);
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                path
            }
            None => with_schema_extension(&self.abs_path),
        };

        let contents = serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_string());
        match fs::write(&out_path, contents) {
            Ok(_) => println!("✓ {} → {}", scope.module_path, out_path.display()),
            Err(e) => eprintln!("✗ {} — failed to write: {}", scope.module_path, e),
        }
    }
}

// ── Type declaration → schema ───────────────────────────────────────

impl JsonSchemaGenerator {
    fn type_decl_to_schema(&self, docs: &Option<&str>, name: &str, t: &[Expr]) -> Value {
        let mut schema = self.anonym_type_to_schema(t);

        if let Value::Object(map) = &mut schema {
            map.insert("title".to_string(), json!(name));
            if let Some(doc) = docs {
                map.insert("description".to_string(), json!(doc.trim()));
            }
        }
        schema
    }

    /// Converts a list of type expressions (one "side" of a declaration or
    /// field) into a single schema, applying annotation-derived constraints.
    fn anonym_type_to_schema(&self, exprs: &[Expr]) -> Value {
        let type_schemas: Vec<Value> = exprs
            .iter()
            .filter(|e| !matches!(e, Expr::Annotation(..)))
            .filter_map(|e| self.expr_to_schema(e))
            .collect();

        let annotations: Vec<(&str, &[AnonymType])> = exprs
            .iter()
            .filter_map(|e| match e {
                Expr::Annotation(n, args) => Some((n.v, args.as_slice())),
                _ => None,
            })
            .collect();

        let mut base = combine_type_schemas(type_schemas);
        apply_annotations(&mut base, &annotations);
        base
    }

    /// Converts a single expression into a schema, or `None` if it has no
    /// meaningful JSON Schema representation (e.g. field references).
    fn expr_to_schema(&self, expr: &Expr) -> Option<Value> {
        Some(match expr {
            Expr::Identifier(id) => self.identifier_to_schema(id.v),
            Expr::Literal(lit) => json!({ "const": literal_to_json(lit) }),
            Expr::Regex(token) => json!({ "type": "string", "pattern": regex_source(token.v) }),
            Expr::FieldAccess(_) => return None,
            Expr::Not(inner) => json!({ "not": self.expr_to_schema(inner)? }),
            Expr::BinaryExpr(op, pair) => self.binary_to_schema(*op, &pair.0, &pair.1)?,
            Expr::Array(type_ident) => json!({
                "type": "array",
                "items": self.identifier_to_schema(type_ident.v),
            }),
            Expr::List(inner) => self.list_to_schema(inner),
            Expr::Set(inner) => self.set_to_schema(inner),
            Expr::Struct(fields) => self.struct_to_schema(fields),
            Expr::Enum(variants) => self.enum_to_schema(variants),
            Expr::Annotation(_, _) => return None,
        })
    }

    fn identifier_to_schema(&self, name: &str) -> Value {
        match builtin_to_schema(name) {
            Some(schema) => schema,
            None => self.ref_for(name),
        }
    }

    fn binary_to_schema(&self, op: BinaryExprType, left: &Expr, right: &Expr) -> Option<Value> {
        let left_schema = self.expr_to_schema(left);
        let right_schema = self.expr_to_schema(right);

        let (left_schema, right_schema) = match (left_schema, right_schema) {
            (Some(l), Some(r)) => (l, r),
            (Some(l), None) => return Some(l),
            (None, Some(r)) => return Some(r),
            (None, None) => return None,
        };

        Some(match op {
            BinaryExprType::Or => json!({ "anyOf": [left_schema, right_schema] }),
            BinaryExprType::Union => json!({ "allOf": [left_schema, right_schema] }),
            BinaryExprType::Intersection => json!({ "allOf": [left_schema, right_schema] }),
            BinaryExprType::Difference => {
                json!({ "allOf": [left_schema, { "not": right_schema }] })
            }
            BinaryExprType::Range
            | BinaryExprType::Add
            | BinaryExprType::Remove
            | BinaryExprType::Xor
            | BinaryExprType::SymmetricDifference => return None,
        })
    }

    fn list_to_schema(&self, inner: &[AnonymType]) -> Value {
        if inner.len() == 1 {
            json!({
                "type": "array",
                "items": self.anonym_type_to_schema(&inner[0]),
                "minItems": 1,
                "maxItems": 1,
            })
        } else {
            let items: Vec<Value> = inner
                .iter()
                .map(|a| self.anonym_type_to_schema(a))
                .collect();
            let count = items.len();
            json!({
                "type": "array",
                "prefixItems": items,
                "minItems": count,
                "maxItems": count,
            })
        }
    }

    fn set_to_schema(&self, inner: &[AnonymType]) -> Value {
        let items = if inner.len() == 1 {
            self.anonym_type_to_schema(&inner[0])
        } else {
            let schemas: Vec<Value> = inner
                .iter()
                .map(|a| self.anonym_type_to_schema(a))
                .collect();
            json!({ "anyOf": schemas })
        };
        json!({
            "type": "array",
            "uniqueItems": true,
            "items": items,
        })
    }

    fn struct_to_schema(&self, fields: &[KeyValExpr]) -> Value {
        let mut properties = Map::new();
        let mut required: Vec<Value> = Vec::new();

        for (key, value) in fields {
            properties.insert(key.v.to_string(), self.anonym_type_to_schema(value));
            if !is_nullable(value) {
                required.push(json!(key.v));
            }
        }

        let mut obj = Map::new();
        obj.insert("type".to_string(), json!("object"));
        obj.insert("properties".to_string(), Value::Object(properties));
        if !required.is_empty() {
            obj.insert("required".to_string(), Value::Array(required));
        }
        obj.insert("additionalProperties".to_string(), json!(false));
        Value::Object(obj)
    }

    fn enum_to_schema(&self, variants: &[KeyValExpr]) -> Value {
        let all_simple = variants.iter().all(|(_, v)| v.is_empty());
        let all_numeric = variants.iter().all(|(_, v)| {
            v.len() == 1 && matches!(v.first(), Some(Expr::Literal(Literal::Number(_))))
        });

        if all_simple {
            let members: Vec<Value> = variants.iter().map(|(k, _)| json!(k.v)).collect();
            json!({ "enum": members })
        } else if all_numeric {
            let members: Vec<Value> = variants
                .iter()
                .filter_map(|(_, v)| match v.first() {
                    Some(Expr::Literal(lit)) => Some(literal_to_json(lit)),
                    _ => None,
                })
                .collect();
            json!({ "enum": members })
        } else {
            // Discriminated union keyed by "kind".
            let members: Vec<Value> = variants
                .iter()
                .map(|(key, value)| {
                    if value.is_empty() {
                        json!({
                            "type": "object",
                            "properties": { "kind": { "const": key.v } },
                            "required": ["kind"],
                            "additionalProperties": false,
                        })
                    } else {
                        json!({
                            "type": "object",
                            "properties": {
                                "kind": { "const": key.v },
                                "value": self.anonym_type_to_schema(value),
                            },
                            "required": ["kind", "value"],
                            "additionalProperties": false,
                        })
                    }
                })
                .collect();
            json!({ "oneOf": members })
        }
    }
}

// ── Schema combination & annotations ────────────────────────────────

/// Combines multiple alternative schemas: 0 → permissive, 1 → itself,
/// many → `anyOf`.
fn combine_type_schemas(mut schemas: Vec<Value>) -> Value {
    match schemas.len() {
        0 => json!({}),
        1 => schemas.pop().unwrap(),
        _ => json!({ "anyOf": schemas }),
    }
}

/// Applies xenomorph validation annotations as JSON Schema keywords. The
/// keyword used for length depends on whether the base schema is a string or
/// an array.
fn apply_annotations(schema: &mut Value, annotations: &[(&str, &[AnonymType])]) {
    let is_array = schema_type_is(schema, "array");
    let map = match schema {
        Value::Object(map) => map,
        _ => return,
    };

    for (name, args) in annotations {
        let number = first_number_arg(args);
        match *name {
            "min" => insert_number(map, "minimum", number),
            "max" => insert_number(map, "maximum", number),
            "gt" => insert_number(map, "exclusiveMinimum", number),
            "lt" => insert_number(map, "exclusiveMaximum", number),
            "len" => {
                if let Some(n) = number {
                    if is_array {
                        map.insert("minItems".to_string(), json!(n));
                        map.insert("maxItems".to_string(), json!(n));
                    } else {
                        map.insert("minLength".to_string(), json!(n));
                        map.insert("maxLength".to_string(), json!(n));
                    }
                }
            }
            "minlen" => insert_number(map, if is_array { "minItems" } else { "minLength" }, number),
            "maxlen" => insert_number(map, if is_array { "maxItems" } else { "maxLength" }, number),
            _ => {}
        }
    }
}

fn insert_number(map: &mut Map<String, Value>, key: &str, number: Option<Value>) {
    if let Some(n) = number {
        map.insert(key.to_string(), n);
    }
}

fn first_number_arg(args: &[AnonymType]) -> Option<Value> {
    for arg in args {
        for expr in arg {
            if let Expr::Literal(lit @ Literal::Number(_)) = expr {
                return Some(literal_to_json(lit));
            }
        }
    }
    None
}

fn schema_type_is(schema: &Value, expected: &str) -> bool {
    schema
        .get("type")
        .and_then(Value::as_str)
        .map(|t| t == expected)
        .unwrap_or(false)
}

// ── Builtin type mapping ────────────────────────────────────────────

fn builtin_to_schema(name: &str) -> Option<Value> {
    let schema = match name {
        "string" | "strong_password" => json!({ "type": "string" }),
        "char" => json!({ "type": "string", "minLength": 1, "maxLength": 1 }),
        "uuid" => json!({ "type": "string", "format": "uuid" }),
        "email" => json!({ "type": "string", "format": "email" }),
        "url" => json!({ "type": "string", "format": "uri" }),
        "hostname" => json!({ "type": "string", "format": "hostname" }),
        "ip" => json!({ "type": "string", "anyOf": [{ "format": "ipv4" }, { "format": "ipv6" }] }),
        "ipv4" => json!({ "type": "string", "format": "ipv4" }),
        "ipv6" => json!({ "type": "string", "format": "ipv6" }),
        "date" => json!({ "type": "string", "format": "date" }),
        "datetime" => json!({ "type": "string", "format": "date-time" }),
        "duration" => json!({ "type": "string", "format": "duration" }),
        "semver" => json!({
            "type": "string",
            "pattern": "^\\d+\\.\\d+\\.\\d+(?:-[0-9A-Za-z-.]+)?(?:\\+[0-9A-Za-z-.]+)?$"
        }),
        "regex" => json!({ "type": "string", "format": "regex" }),
        "xml" | "yaml" | "json" | "toml" | "csv" | "tsv" => json!({ "type": "string" }),
        "binary" => json!({ "type": "string", "contentEncoding": "base64" }),
        "bool" => json!({ "type": "boolean" }),
        "number" | "f32" | "f64" | "decimal" => json!({ "type": "number" }),
        "bigint" => json!({ "type": "integer" }),
        "any" => json!({}),
        "null" => json!({ "type": "null" }),
        "dict" => json!({ "type": "object" }),
        _ => return integer_schema(name),
    };
    Some(schema)
}

/// Builds an `integer` schema with bounds for sized int types like `u8`/`i16`.
fn integer_schema(name: &str) -> Option<Value> {
    let bits: u32 = name.get(1..).and_then(|b| b.parse().ok())?;
    let signed = match name.as_bytes().first() {
        Some(b'i') => true,
        Some(b'u') => false,
        _ => return None,
    };

    let mut schema = Map::new();
    schema.insert("type".to_string(), json!("integer"));

    // Only emit bounds that fit safely in a JSON number (<= 32-bit width).
    if signed {
        if bits <= 32 {
            let max = (1i64 << (bits - 1)) - 1;
            schema.insert("minimum".to_string(), json!(-(max + 1)));
            schema.insert("maximum".to_string(), json!(max));
        }
    } else {
        schema.insert("minimum".to_string(), json!(0));
        if bits <= 32 {
            let max = (1i64 << bits) - 1;
            schema.insert("maximum".to_string(), json!(max));
        }
    }

    Some(Value::Object(schema))
}

// ── Literal & misc helpers ──────────────────────────────────────────

fn literal_to_json(lit: &Literal) -> Value {
    match lit {
        Literal::Number(NumberType::Int(n, _)) => json!(n),
        Literal::Number(NumberType::Float(f, _)) => json!(f),
        Literal::String(s, _) => json!(s),
        Literal::Boolean(b, _) => json!(b),
    }
}

/// Extracts the pattern body from a regex literal like `/foo/i`.
fn regex_source(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix('/') {
        if let Some(end) = stripped.rfind('/') {
            return stripped[..end].to_string();
        }
    }
    trimmed.to_string()
}

fn is_nullable(exprs: &AnonymType) -> bool {
    exprs.iter().any(|e| match e {
        Expr::Identifier(id) => id.v == "null",
        Expr::BinaryExpr(BinaryExprType::Union | BinaryExprType::Or, pair) => {
            is_null_expr(&pair.0) || is_null_expr(&pair.1)
        }
        _ => false,
    })
}

fn is_null_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Identifier(id) if id.v == "null")
}

fn with_schema_extension(path: &PathBuf) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}.schema.json"))
}

/// Builds a relative path from one module to another, used as a cross-file
/// `$ref` prefix. Returns an empty string when both are the same module.
fn schema_ref_path(from_module_path: &str, to_module_path: &str) -> String {
    if from_module_path == to_module_path {
        return String::new();
    }

    let mut from_dir = module_path_parts(from_module_path);
    from_dir.pop();

    let to_parts = module_path_parts(to_module_path);
    let common_len = from_dir
        .iter()
        .zip(&to_parts)
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative_parts = vec![".."; from_dir.len().saturating_sub(common_len)];
    relative_parts.extend(to_parts[common_len..].iter().copied());

    let path = relative_parts.join("/");
    let path = if path.starts_with("..") {
        path
    } else {
        format!("./{path}")
    };
    format!("{path}.schema.json")
}

fn module_path_parts(module_path: &str) -> Vec<&str> {
    module_path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_string_mapping() {
        assert_eq!(
            builtin_to_schema("string"),
            Some(json!({ "type": "string" }))
        );
        assert_eq!(
            builtin_to_schema("uuid"),
            Some(json!({ "type": "string", "format": "uuid" }))
        );
        assert_eq!(
            builtin_to_schema("bool"),
            Some(json!({ "type": "boolean" }))
        );
    }

    #[test]
    fn test_unsigned_integer_bounds() {
        assert_eq!(
            integer_schema("u8"),
            Some(json!({ "type": "integer", "minimum": 0, "maximum": 255 }))
        );
        assert_eq!(
            integer_schema("u16"),
            Some(json!({ "type": "integer", "minimum": 0, "maximum": 65535 }))
        );
    }

    #[test]
    fn test_signed_integer_bounds() {
        assert_eq!(
            integer_schema("i8"),
            Some(json!({ "type": "integer", "minimum": -128, "maximum": 127 }))
        );
    }

    #[test]
    fn test_large_integer_has_no_max() {
        assert_eq!(
            integer_schema("u64"),
            Some(json!({ "type": "integer", "minimum": 0 }))
        );
        assert_eq!(integer_schema("i128"), Some(json!({ "type": "integer" })));
    }

    #[test]
    fn test_unknown_identifier_is_not_builtin() {
        assert_eq!(builtin_to_schema("MyCustomType"), None);
    }

    #[test]
    fn test_regex_source_extraction() {
        assert_eq!(regex_source("/foo.*/i"), "foo.*");
        assert_eq!(regex_source("/^a$/"), "^a$");
    }

    #[test]
    fn test_schema_ref_path_sibling() {
        assert_eq!(
            schema_ref_path("models/user", "models/address"),
            "./address.schema.json"
        );
    }

    #[test]
    fn test_schema_ref_path_parent() {
        assert_eq!(
            schema_ref_path("models/user/profile", "models/address"),
            "../address.schema.json"
        );
    }

    #[test]
    fn test_schema_ref_path_same_module_is_empty() {
        assert_eq!(schema_ref_path("models/user", "models/user"), "");
    }
}
