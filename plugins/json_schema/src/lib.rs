use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use xenomorph_common::config::{ConfigValue, PluginConfigs};
use xenomorph_common::parser::{
    Annotation, Declaration, Expr, KeyValExpr, Literal, SimpleType, Type, XenoType,
};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::semantic::{AnalyzerListener, ScopeInfo};
use xenomorph_common::utils::extract_documentation;

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "json_schema";
static VERSION: &str = "0.1.0";
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    version: VERSION,
    initialize: None,
    provide_types: None,
    provide_annotations: None,
    provide_config_schema: Some(provide_config_schema),
    register_generator: Some(create_generator),
    register_analyzer: None,
};

fn provide_config_schema() -> &'static str {
    r#"{
        "type": "object",
        "description": "JSON Schema generator.",
        "properties": {
            "output": {
                "type": "string",
                "description": "Output directory for generated .schema.json files, relative to the workspace root. If omitted, files are written next to their .xen sources."
            }
        },
        "additionalProperties": false
    }"#
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

fn create_generator(plugin_configs: &PluginConfigs) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    let mut generator = JsonSchemaGenerator::new();

    if let Some(ConfigValue::Table(cfg)) = plugin_configs.get("json_schema") {
        if let Some(ConfigValue::String(output)) = cfg.get("output") {
            generator.output_dir = Some(PathBuf::from(output));
        }
    }

    Box::new(generator)
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

    fn type_decl_to_schema(&self, docs: &Option<&str>, name: &str, t: &XenoType) -> Value {
        let mut schema = self.anonym_type_to_schema(t);

        if let Value::Object(map) = &mut schema {
            map.insert("title".to_string(), json!(name));
            if let Some(doc) = docs {
                map.insert("description".to_string(), json!(doc.trim()));
            }
        }
        schema
    }

    fn anonym_type_to_schema(&self, ty: &XenoType) -> Value {
        let mut base = self.type_to_schema(&ty.0);
        apply_annotations(&mut base, &ty.1);
        base
    }

    fn type_to_schema(&self, ty: &Type) -> Value {
        match ty {
            Type::Simple(simple) => self.simple_type_to_schema(simple),
            Type::Tuple(items) => self.list_to_schema(items),
            Type::Set(items) => self.set_to_schema(items),
            Type::Struct(fields) => self.struct_to_schema(fields),
            Type::Enum(variants) => self.enum_to_schema(variants),
            Type::Sum(items) => json!({
                "anyOf": items
                    .iter()
                    .map(|item| self.simple_type_to_schema(item))
                    .collect::<Vec<_>>()
            }),
            Type::Intersection(items) => json!({
                "allOf": items
                    .iter()
                    .map(|item| self.simple_type_to_schema(item))
                    .collect::<Vec<_>>()
            }),
        }
    }

    fn simple_type_to_schema(&self, ty: &SimpleType) -> Value {
        let (base, optional) = match ty {
            SimpleType::Literal(literal) => (json!({ "const": literal_to_json(literal) }), false),
            SimpleType::OptionalLiteral(literal) => {
                (json!({ "const": literal_to_json(literal) }), true)
            }
            SimpleType::Identifier(identifier) => (self.identifier_to_schema(identifier.v), false),
            SimpleType::OptionalIdentifier(identifier) => {
                (self.identifier_to_schema(identifier.v), true)
            }
            SimpleType::Array(identifier) => (
                json!({
                    "type": "array",
                    "items": self.identifier_to_schema(identifier.v),
                }),
                false,
            ),
            SimpleType::OptionalArray(identifier) => (
                json!({
                    "type": "array",
                    "items": self.identifier_to_schema(identifier.v),
                }),
                true,
            ),
        };

        if optional {
            json!({ "anyOf": [base, { "type": "null" }] })
        } else {
            base
        }
    }

    fn identifier_to_schema(&self, name: &str) -> Value {
        match builtin_to_schema(name) {
            Some(schema) => schema,
            None => self.ref_for(name),
        }
    }

    fn list_to_schema(&self, inner: &[SimpleType]) -> Value {
        let items: Vec<Value> = inner
            .iter()
            .map(|item| self.simple_type_to_schema(item))
            .collect();
        let count = items.len();
        json!({
            "type": "array",
            "prefixItems": items,
            "minItems": count,
            "maxItems": count,
        })
    }

    fn set_to_schema(&self, inner: &[SimpleType]) -> Value {
        let schemas: Vec<Value> = inner
            .iter()
            .map(|item| self.simple_type_to_schema(item))
            .collect();
        let items = combine_type_schemas(schemas);
        json!({
            "type": "array",
            "uniqueItems": true,
            "items": items,
        })
    }

    fn struct_to_schema(&self, fields: &[KeyValExpr]) -> Value {
        let mut properties = Map::new();
        let mut required: Vec<Value> = Vec::new();

        for (key, value, docs) in fields {
            let schema = with_description(self.simple_type_to_schema(value), *docs);
            properties.insert(key.v.to_string(), schema);
            if !is_optional(value) {
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
        let all_numeric = variants.iter().all(|(_, value, _)| {
            matches!(
                value,
                SimpleType::Literal(Literal::Int(_, _) | Literal::Float(_, _))
            )
        });

        let has_docs = variants.iter().any(|(_, _, docs)| docs.is_some());

        if all_numeric && !has_docs {
            let members: Vec<Value> = variants
                .iter()
                .filter_map(|(_, value, _)| match value {
                    SimpleType::Literal(literal) => Some(literal_to_json(literal)),
                    _ => None,
                })
                .collect();
            json!({ "enum": members })
        } else if all_numeric {
            let members: Vec<Value> = variants
                .iter()
                .filter_map(|(_, value, docs)| match value {
                    SimpleType::Literal(literal) => Some(with_description(
                        json!({ "const": literal_to_json(literal) }),
                        *docs,
                    )),
                    _ => None,
                })
                .collect();
            json!({ "oneOf": members })
        } else {
            // Discriminated union keyed by "kind".
            let members: Vec<Value> = variants
                .iter()
                .map(|(key, value, docs)| {
                    with_description(
                        json!({
                            "type": "object",
                            "properties": {
                                "kind": { "const": key.v },
                                "value": self.simple_type_to_schema(value),
                            },
                            "required": ["kind", "value"],
                            "additionalProperties": false,
                        }),
                        *docs,
                    )
                })
                .collect();
            json!({ "oneOf": members })
        }
    }
}

impl<'src> AnalyzerListener<'src> for JsonSchemaGenerator {
    fn on_before_module(&mut self, scope: &ScopeInfo) {
        self.abs_path = scope.abs_path.clone();
        self.module_path = scope.module_path.clone();
        self.imported_types = scope.imported_types.clone();
        self.defs.clear();
    }

    fn on_before_ast(
        &mut self,
        ast: &[Declaration<'src>],
        _errors: &mut Vec<xenomorph_common::XenoDiagnostic<'src>>,
    ) {
        for decl in ast {
            if let Declaration::Type {
                docs, name, ty: t, ..
            } = decl
            {
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
fn apply_annotations(schema: &mut Value, annotations: &[Annotation]) {
    let is_array = schema_type_is(schema, "array");
    let map = match schema {
        Value::Object(map) => map,
        _ => return,
    };

    for annotation in annotations {
        let number = first_number_arg(&annotation.params);
        match annotation.ident.v {
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

fn first_number_arg(args: &[Expr]) -> Option<Value> {
    for arg in args {
        if let Expr::Type(Type::Simple(SimpleType::Literal(
            literal @ (Literal::Int(_, _) | Literal::Float(_, _)),
        ))) = arg
        {
            return Some(literal_to_json(literal));
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
        Literal::Int(n, _) => n
            .to_string()
            .parse::<serde_json::Number>()
            .map(Value::Number)
            .unwrap_or_else(|_| json!(n.to_string())),
        Literal::Float(f, _) => f
            .to_string()
            .parse::<serde_json::Number>()
            .map(Value::Number)
            .unwrap_or_else(|_| json!(f.to_string())),
        Literal::String(s, _) => json!(s),
        Literal::Boolean(b, _) => json!(b),
    }
}

/// Extracts the pattern body from a regex literal like `/foo/i`.
#[cfg(test)]
fn regex_source(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix('/') {
        if let Some(end) = stripped.rfind('/') {
            return stripped[..end].to_string();
        }
    }
    trimmed.to_string()
}

fn is_optional(ty: &SimpleType) -> bool {
    matches!(
        ty,
        SimpleType::OptionalLiteral(_)
            | SimpleType::OptionalIdentifier(_)
            | SimpleType::OptionalArray(_)
    )
}

fn with_description(mut schema: Value, docs: Option<&xenomorph_common::TokenData>) -> Value {
    if let (Value::Object(map), Some(docs)) = (&mut schema, docs) {
        map.insert(
            "description".to_string(),
            json!(extract_documentation(docs)),
        );
    }
    schema
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
    fn test_type_and_field_docs_are_preserved() {
        let key = xenomorph_common::TokenData {
            v: "displayName",
            l: 0,
            c: 0,
        };
        let field_type = xenomorph_common::TokenData {
            v: "string",
            l: 0,
            c: 0,
        };
        let field_docs = xenomorph_common::TokenData {
            v: "/** Name shown to users. */",
            l: 0,
            c: 0,
        };
        let ty = (
            Type::Struct(vec![(
                &key,
                SimpleType::Identifier(&field_type),
                Some(&field_docs),
            )]),
            vec![],
        );

        let schema = JsonSchemaGenerator::new().type_decl_to_schema(
            &Some("A user-facing profile."),
            "Profile",
            &ty,
        );

        assert_eq!(schema["title"], json!("Profile"));
        assert_eq!(schema["description"], json!("A user-facing profile."));
        assert_eq!(
            schema["properties"]["displayName"]["description"],
            json!("Name shown to users.")
        );
    }

    #[test]
    fn test_numeric_enum_member_docs_are_preserved() {
        let key = xenomorph_common::TokenData {
            v: "Active",
            l: 0,
            c: 0,
        };
        let literal = xenomorph_common::TokenData { v: "1", l: 0, c: 0 };
        let docs = xenomorph_common::TokenData {
            v: "/** The active state. */",
            l: 0,
            c: 0,
        };
        let variants = vec![(
            &key,
            SimpleType::Literal(Literal::Int(1.into(), &literal)),
            Some(&docs),
        )];

        let schema = JsonSchemaGenerator::new().enum_to_schema(&variants);

        assert_eq!(schema["oneOf"][0]["const"], json!(1));
        assert_eq!(
            schema["oneOf"][0]["description"],
            json!("The active state.")
        );
    }

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
