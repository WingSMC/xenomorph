use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use xenomorph_common::config::{ConfigValue, PluginConfigs};
use xenomorph_common::parser::{
    Annotation, Declaration, Expr, KeyValExpr, Literal, SimpleType, Type, XenoType,
};
use xenomorph_common::plugins::{PluginCompletion, XenoPlugin};
use xenomorph_common::semantic::{AnalyzerListener, ScopeInfo};
use xenomorph_common::utils::extract_documentation;

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "java";
static VERSION: &str = "0.1.0";
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    version: VERSION,
    initialize: None,
    provide_types: None,
    provide_annotations: Some(provide_annotations),
    provide_config_schema: Some(provide_config_schema),
    register_generator: Some(create_generator),
    register_analyzer: None,
};

/// The `@Lombok(...)` annotation lets authors attach Lombok decorators to a
/// type or field, e.g. `@Lombok(Data)` or `@Lombok(ToString, EqualsAndHashCode)`.
static LOMBOK_COMPLETION: &[PluginCompletion] = &[PluginCompletion {
    label: "Lombok",
    detail: Some("Java Lombok decorators"),
    documentation: Some(
        "Attaches Lombok class/field decorators to the generated Java DTO, e.g. `@Lombok(Data)`.",
    ),
}];

fn provide_annotations() -> &'static [PluginCompletion] {
    LOMBOK_COMPLETION
}

fn provide_config_schema() -> &'static str {
    r#"{
        "type": "object",
        "description": "Java + Lombok DTO generator.",
        "properties": {
            "output": {
                "type": "string",
                "description": "Target directory for generated .java files, relative to the workspace root. Package sub-directories are created inside it. If omitted, files are written next to their .xen sources."
            },
            "package": {
                "type": "string",
                "description": "Java package the generated DTOs belong to, e.g. `com.xyz.model`."
            },
            "value": {
                "type": "boolean",
                "description": "Annotate every generated class with Lombok's `@Value` (immutable DTOs)."
            },
            "builder": {
                "type": "boolean",
                "description": "Annotate every generated class with Lombok's `@Builder`."
            },
            "data": {
                "type": "boolean",
                "description": "Annotate every generated class with Lombok's `@Data`."
            }
        },
        "additionalProperties": false
    }"#
}

#[no_mangle]
fn load() -> &'static XenoPlugin<'static> {
    &PLUGIN
}

// ── Generator listener ──────────────────────────────────────────────

struct JavaGenerator {
    abs_path: PathBuf,
    module_path: String,
    /// Output directory override from `[plugins.java].output`.
    /// If None, writes `.java` files next to the `.xen` source files.
    output_dir: Option<PathBuf>,
    /// Target Java package, e.g. `com.xyz.model`.
    package: String,
    /// Blanket `@Value` on every class.
    blanket_value: bool,
    /// Blanket `@Builder` on every class.
    blanket_builder: bool,
    /// Blanket `@Data` on every class.
    blanket_data: bool,
    /// Generated files for the current module: (type name, file contents).
    files: Vec<(String, String)>,
}

impl JavaGenerator {
    fn new() -> Self {
        Self {
            abs_path: PathBuf::new(),
            module_path: String::new(),
            output_dir: None,
            package: String::new(),
            blanket_value: false,
            blanket_builder: false,
            blanket_data: false,
            files: Vec::new(),
        }
    }
}

fn create_generator(plugin_configs: &PluginConfigs) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    let mut generator = JavaGenerator::new();

    if let Some(ConfigValue::Table(cfg)) = plugin_configs.get("java") {
        if let Some(ConfigValue::String(output)) = cfg.get("output") {
            generator.output_dir = Some(PathBuf::from(output));
        }
        if let Some(ConfigValue::String(package)) = cfg.get("package") {
            generator.package = package.clone();
        }
        if let Some(ConfigValue::Boolean(value)) = cfg.get("value") {
            generator.blanket_value = *value;
        }
        if let Some(ConfigValue::Boolean(builder)) = cfg.get("builder") {
            generator.blanket_builder = *builder;
        }
        if let Some(ConfigValue::Boolean(data)) = cfg.get("data") {
            generator.blanket_data = *data;
        }
    }

    Box::new(generator)
}

impl<'src> AnalyzerListener<'src> for JavaGenerator {
    fn on_before_module(&mut self, scope: &ScopeInfo) {
        self.abs_path = scope.abs_path.clone();
        self.module_path = scope.module_path.clone();
        self.files.clear();
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
                if let Some(content) = self.generate_type_decl(docs, name.v, t) {
                    self.files.push((name.v.to_string(), content));
                }
            }
        }
    }

    fn on_after_module(&mut self, scope: &ScopeInfo) {
        let base = match &self.output_dir {
            Some(dir) => dir.clone(),
            None => self
                .abs_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
        };
        let package_dir = base.join(package_to_path(&self.package));
        if let Err(e) = fs::create_dir_all(&package_dir) {
            eprintln!(
                "✗ {} — failed to create output dir: {}",
                scope.module_path, e
            );
            return;
        }

        for (name, content) in &self.files {
            let path = package_dir.join(format!("{name}.java"));
            match fs::write(&path, content) {
                Ok(_) => println!("✓ {} → {}", scope.module_path, path.display()),
                Err(e) => eprintln!("✗ {} — failed to write: {}", scope.module_path, e),
            }
        }
    }
}

// ── Type declaration generation ─────────────────────────────────────

impl JavaGenerator {
    // TODO include from-to locations
    fn generate_type_decl(&self, docs: &Option<&str>, name: &str, t: &XenoType) -> Option<String> {
        let lombok_decorators = collect_lombok_decorators(&t.1);

        match &t.0 {
            Type::Struct(fields) => {
                Some(self.generate_class(docs, name, fields, &lombok_decorators))
            }
            Type::Enum(variants) => Some(self.generate_enum(docs, name, variants)),
            _ => None,
        }
    }

    // ── Class (struct) generation ───────────────────────────────────

    fn generate_class(
        &self,
        docs: &Option<&str>,
        name: &str,
        fields: &[KeyValExpr],
        type_lombok_decorators: &[String],
    ) -> String {
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let class_annotations = self.class_annotations(type_lombok_decorators);

        // Render fields first so their imports are collected before the header.
        let mut field_block = String::new();
        for (key, value, docs) in fields {
            let java_type = self.type_to_java(value, &mut imports);
            let nullable = is_optional(value);

            if let Some(docs) = docs {
                push_indented_javadoc(&mut field_block, extract_documentation(docs), "    ");
            }
            if !nullable {
                field_block.push_str("    @NonNull\n");
                register_lombok_import(&mut imports, "NonNull");
            }
            field_block.push_str(&format!("    private {java_type} {};\n", key.v));
        }

        for annotation in &class_annotations {
            register_lombok_import(&mut imports, annotation);
        }

        let mut out = String::new();
        out.push_str("// Auto-generated by xenomorph-java — do not edit.\n");
        if !self.package.is_empty() {
            out.push_str(&format!("package {};\n", self.package));
        }
        out.push('\n');

        if !imports.is_empty() {
            for import in &imports {
                out.push_str(&format!("import {import};\n"));
            }
            out.push('\n');
        }

        push_javadoc(&mut out, docs);
        for annotation in &class_annotations {
            out.push_str(&format!("@{annotation}\n"));
        }
        out.push_str(&format!("public class {name} {{\n"));
        out.push_str(&field_block);
        out.push_str("}\n");
        out
    }

    /// Determines the Lombok class-level annotations for a generated class,
    /// combining blanket settings and per-type `@Lombok(...)` decorators.
    fn class_annotations(&self, type_lombok_decorators: &[String]) -> Vec<String> {
        let mut annotations: Vec<String> = Vec::new();

        if self.blanket_data {
            push_unique(&mut annotations, "Data");
        }
        if self.blanket_value {
            push_unique(&mut annotations, "Value");
        }
        if self.blanket_builder {
            push_unique(&mut annotations, "Builder");
        }
        for decorator in type_lombok_decorators {
            push_unique(&mut annotations, decorator);
        }

        // `@Data` and `@Value` already bundle getters (and setters for
        // `@Data`), so only add explicit accessors when neither is present.
        let has_accessor_bundle = annotations
            .iter()
            .any(|a| a == "Data" || a == "Value" || a == "Getter" || a == "Setter");
        if !has_accessor_bundle {
            push_unique(&mut annotations, "Getter");
            push_unique(&mut annotations, "Setter");
        }

        annotations
    }

    // ── Enum generation ─────────────────────────────────────────────

    fn generate_enum(&self, docs: &Option<&str>, name: &str, variants: &[KeyValExpr]) -> String {
        let all_int = !variants.is_empty()
            && variants
                .iter()
                .all(|(_, value, _)| matches!(value, SimpleType::Literal(Literal::Int(_, _))));

        let mut out = String::new();
        out.push_str("// Auto-generated by xenomorph-java — do not edit.\n");
        if !self.package.is_empty() {
            out.push_str(&format!("package {};\n", self.package));
        }
        out.push('\n');

        push_javadoc(&mut out, docs);
        out.push_str(&format!("public enum {name} {{\n"));

        if all_int {
            for (i, (key, value, docs)) in variants.iter().enumerate() {
                let sep = if i + 1 == variants.len() { ";" } else { "," };
                let n = match value {
                    SimpleType::Literal(Literal::Int(n, _)) => n.to_string(),
                    _ => "0".to_string(),
                };
                if let Some(docs) = docs {
                    push_indented_javadoc(&mut out, extract_documentation(docs), "    ");
                }
                out.push_str(&format!("    {}({n}){sep}\n", key.v));
            }
            out.push('\n');
            out.push_str("    private final int value;\n\n");
            out.push_str(&format!("    {name}(int value) {{\n"));
            out.push_str("        this.value = value;\n");
            out.push_str("    }\n\n");
            out.push_str("    public int getValue() {\n");
            out.push_str("        return value;\n");
            out.push_str("    }\n");
        } else {
            for (i, (key, _, docs)) in variants.iter().enumerate() {
                let sep = if i + 1 == variants.len() { "" } else { "," };
                if let Some(docs) = docs {
                    push_indented_javadoc(&mut out, extract_documentation(docs), "    ");
                }
                out.push_str(&format!("    {}{sep}\n", key.v));
            }
        }

        out.push_str("}\n");
        out
    }

    // ── Type mapping ────────────────────────────────────────────────

    fn type_to_java(&self, ty: &SimpleType, imports: &mut BTreeSet<String>) -> String {
        match ty {
            SimpleType::Identifier(identifier) | SimpleType::OptionalIdentifier(identifier) => {
                builtin_to_java(identifier.v, imports)
            }
            SimpleType::Array(identifier) | SimpleType::OptionalArray(identifier) => {
                imports.insert("java.util.List".to_string());
                format!("List<{}>", builtin_to_java(identifier.v, imports))
            }
            SimpleType::Literal(Literal::String(_, _))
            | SimpleType::OptionalLiteral(Literal::String(_, _)) => "String".to_string(),
            SimpleType::Literal(Literal::Boolean(_, _))
            | SimpleType::OptionalLiteral(Literal::Boolean(_, _)) => "Boolean".to_string(),
            SimpleType::Literal(Literal::Int(_, _))
            | SimpleType::OptionalLiteral(Literal::Int(_, _)) => "Integer".to_string(),
            SimpleType::Literal(Literal::Float(_, _))
            | SimpleType::OptionalLiteral(Literal::Float(_, _)) => "Double".to_string(),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Maps a Xenomorph builtin type name to a Java type, registering the import
/// for types that live outside `java.lang`. Unknown names are assumed to be
/// user-defined types in the same package and are returned unchanged.
fn builtin_to_java(name: &str, imports: &mut BTreeSet<String>) -> String {
    match name {
        "string" | "char" | "email" | "url" | "hostname" | "ip" | "ipv4" | "ipv6" | "semver"
        | "strong_password" | "json" | "xml" | "yaml" | "toml" | "csv" | "tsv" => {
            "String".to_string()
        }
        "bool" => "Boolean".to_string(),
        "i4" | "i8" | "i16" | "i32" | "u4" | "u8" | "u16" => "Integer".to_string(),
        "i64" | "u32" => "Long".to_string(),
        "u64" | "u128" | "i128" | "bigint" | "integer" => {
            imports.insert("java.math.BigInteger".to_string());
            "BigInteger".to_string()
        }
        "f32" => "Float".to_string(),
        "f64" | "number" => "Double".to_string(),
        "decimal" => {
            imports.insert("java.math.BigDecimal".to_string());
            "BigDecimal".to_string()
        }
        "uuid" => {
            imports.insert("java.util.UUID".to_string());
            "UUID".to_string()
        }
        "date" => {
            imports.insert("java.time.LocalDate".to_string());
            "LocalDate".to_string()
        }
        "datetime" => {
            imports.insert("java.time.OffsetDateTime".to_string());
            "OffsetDateTime".to_string()
        }
        "duration" => {
            imports.insert("java.time.Duration".to_string());
            "Duration".to_string()
        }
        "regex" => {
            imports.insert("java.util.regex.Pattern".to_string());
            "Pattern".to_string()
        }
        "binary" => "byte[]".to_string(),
        "dict" => {
            imports.insert("java.util.Map".to_string());
            "Map<String, Object>".to_string()
        }
        "any" | "null" => "Object".to_string(),
        other => other.to_string(),
    }
}

/// Collects the decorator names from all `@Lombok(...)` annotations attached to
/// a type or field, e.g. `@Lombok(ToString, EqualsAndHashCode)` → `["ToString",
/// "EqualsAndHashCode"]`.
fn collect_lombok_decorators(annotations: &[Annotation]) -> Vec<String> {
    let mut decorators = Vec::new();
    for annotation in annotations {
        if annotation.ident.v != "Lombok" {
            continue;
        }
        for arg in &annotation.params {
            if let Some(decorator) = decorator_name(arg) {
                push_unique(&mut decorators, &decorator);
            }
        }
    }
    decorators
}

/// Renders a single `@Lombok(...)` argument as a Java decorator name.
fn decorator_name(arg: &Expr) -> Option<String> {
    match arg {
        Expr::Type(Type::Simple(SimpleType::Identifier(identifier))) => {
            Some(identifier.v.to_string())
        }
        _ => None,
    }
}

/// Registers the `lombok.*` import for a decorator, using its top-level name so
/// nested decorators like `ToString.Exclude` import `lombok.ToString`.
fn register_lombok_import(imports: &mut BTreeSet<String>, decorator: &str) {
    let top = decorator.split('.').next().unwrap_or(decorator);
    if !top.is_empty() {
        imports.insert(format!("lombok.{top}"));
    }
}

fn is_optional(ty: &SimpleType) -> bool {
    matches!(
        ty,
        SimpleType::OptionalLiteral(_)
            | SimpleType::OptionalIdentifier(_)
            | SimpleType::OptionalArray(_)
    )
}

fn push_javadoc(out: &mut String, docs: &Option<&str>) {
    if let Some(doc) = docs {
        out.push_str("/**\n");
        for line in doc.lines() {
            out.push_str(&format!(" * {line}\n"));
        }
        out.push_str(" */\n");
    }
}

fn push_indented_javadoc(out: &mut String, docs: &str, indent: &str) {
    out.push_str(&format!("{indent}/**\n"));
    for line in docs.lines() {
        out.push_str(&format!("{indent} * {line}\n"));
    }
    out.push_str(&format!("{indent} */\n"));
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

fn package_to_path(package: &str) -> PathBuf {
    package.split('.').filter(|part| !part.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imports() -> BTreeSet<String> {
        BTreeSet::new()
    }

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

        let output = JavaGenerator::new()
            .generate_type_decl(&Some("A user-facing profile."), "Profile", &ty)
            .expect("structs generate Java classes");

        assert!(output.contains(" * A user-facing profile."));
        assert!(output.contains("     * Name shown to users."));
        assert!(output.contains("    private String displayName;"));
    }

    #[test]
    fn test_builtin_mappings() {
        let mut i = imports();
        assert_eq!(builtin_to_java("string", &mut i), "String");
        assert_eq!(builtin_to_java("bool", &mut i), "Boolean");
        assert_eq!(builtin_to_java("u8", &mut i), "Integer");
        assert_eq!(builtin_to_java("i64", &mut i), "Long");
        assert_eq!(builtin_to_java("u64", &mut i), "BigInteger");
        assert_eq!(builtin_to_java("f32", &mut i), "Float");
        assert_eq!(builtin_to_java("f64", &mut i), "Double");
        assert_eq!(builtin_to_java("decimal", &mut i), "BigDecimal");
        assert_eq!(builtin_to_java("uuid", &mut i), "UUID");
        assert_eq!(builtin_to_java("binary", &mut i), "byte[]");
        assert_eq!(builtin_to_java("MyType", &mut i), "MyType");
        assert!(i.contains("java.math.BigInteger"));
        assert!(i.contains("java.util.UUID"));
    }

    #[test]
    fn test_package_to_path() {
        assert_eq!(
            package_to_path("com.xyz.model"),
            PathBuf::from("com").join("xyz").join("model")
        );
        assert_eq!(package_to_path(""), PathBuf::new());
    }

    #[test]
    fn test_default_accessors_when_no_blanket_settings() {
        let generator = JavaGenerator::new();
        assert_eq!(
            generator.class_annotations(&[]),
            vec!["Getter".to_string(), "Setter".to_string()]
        );
    }

    #[test]
    fn test_blanket_data_skips_explicit_accessors() {
        let mut generator = JavaGenerator::new();
        generator.blanket_data = true;
        generator.blanket_builder = true;
        assert_eq!(
            generator.class_annotations(&[]),
            vec!["Data".to_string(), "Builder".to_string()]
        );
    }

    #[test]
    fn test_type_level_lombok_decorators_are_used() {
        let generator = JavaGenerator::new();
        let decorators = vec!["Value".to_string()];
        assert_eq!(
            generator.class_annotations(&decorators),
            vec!["Value".to_string()]
        );
    }

    #[test]
    fn test_register_lombok_import_uses_top_level() {
        let mut i = imports();
        register_lombok_import(&mut i, "ToString.Exclude");
        register_lombok_import(&mut i, "NonNull");
        assert!(i.contains("lombok.ToString"));
        assert!(i.contains("lombok.NonNull"));
    }
}
