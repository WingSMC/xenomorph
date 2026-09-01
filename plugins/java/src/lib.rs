use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use xenomorph_common::config::{Config, ConfigValue, PluginConfigs};
use xenomorph_common::parser::{
    decimal_roundtrips_as, integer_fits, Annotation, Declaration, Expr, FloatSize, IntLiteral,
    IntegerRepresentation, IntegerSize, KeyValExpr, Literal, SimpleType, Type, XenoType,
};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::semantic::{
    simple_to_owned_type, AnalyzerListener, OwnedType, ScopeInfo, TypeHierarchy, XenoAnnotation,
    XenoAnnotationKind, XenoConstraint, XenoParam, XenoParent, XenoTrait, XenoTraitKind,
    XenoType as SemanticType, ANY_TARGET_PARAM,
};
use xenomorph_common::utils::extract_documentation;
use xenomorph_common::{TokenData, XenoDiagSeverity, XenoDiagnostic};

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "java";
static VERSION: &str = "0.1.2";
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    version: VERSION,
    initialize: None,
    provide_types: Some(provide_types),
    provide_annotations: Some(provide_annotations),
    provide_config_schema: Some(provide_config_schema),
    register_generator: Some(create_generator),
    register_analyzer: None,
};

static LOMBOK_TRAIT: XenoTrait = XenoTrait {
    name: "LombokDecorator",
    documentation: Some("Implemented only by supported Java Lombok decorator names."),
    kind: XenoTraitKind::Semantic,
    parents: None,
};

static LOMBOK_PARENT: &[XenoParent] = &[XenoParent::Trait(&LOMBOK_TRAIT)];

macro_rules! lombok_type {
    ($constant:ident, $name:literal) => {
        static $constant: SemanticType = SemanticType {
            name: $name,
            documentation: Some(concat!("Java Lombok `@", $name, "` decorator.")),
            generic_params: None,
            parents: Some(LOMBOK_PARENT),
        };
    };
}

lombok_type!(ACCESSORS, "Accessors");
lombok_type!(ALL_ARGS_CONSTRUCTOR, "AllArgsConstructor");
lombok_type!(BUILDER, "Builder");
lombok_type!(DATA, "Data");
lombok_type!(EQUALS_AND_HASH_CODE, "EqualsAndHashCode");
lombok_type!(GETTER, "Getter");
lombok_type!(NON_NULL, "NonNull");
lombok_type!(NO_ARGS_CONSTRUCTOR, "NoArgsConstructor");
lombok_type!(REQUIRED_ARGS_CONSTRUCTOR, "RequiredArgsConstructor");
lombok_type!(SETTER, "Setter");
lombok_type!(SINGULAR, "Singular");
lombok_type!(TO_STRING, "ToString");
lombok_type!(VALUE, "Value");
lombok_type!(WITH, "With");

static LOMBOK_TYPES: &[&SemanticType] = &[
    &ACCESSORS,
    &ALL_ARGS_CONSTRUCTOR,
    &BUILDER,
    &DATA,
    &EQUALS_AND_HASH_CODE,
    &GETTER,
    &NO_ARGS_CONSTRUCTOR,
    &NON_NULL,
    &REQUIRED_ARGS_CONSTRUCTOR,
    &SETTER,
    &SINGULAR,
    &TO_STRING,
    &VALUE,
    &WITH,
];

static LOMBOK_PARAM: XenoParam = XenoParam {
    name: "decorator",
    constraint: XenoConstraint::Trait(&LOMBOK_TRAIT),
};

static LOMBOK_ANNOTATION: XenoAnnotation = XenoAnnotation {
    name: "Lombok",
    documentation: Some(
        "Attaches one or more supported Lombok decorators to the generated Java DTO, e.g. `@Lombok(Data, Builder)`.",
    ),
    kind: XenoAnnotationKind::Meta,
    params: &[&ANY_TARGET_PARAM, &LOMBOK_PARAM],
    variadic: true,
};

static LOMBOK_ANNOTATIONS: &[&XenoAnnotation] = &[&LOMBOK_ANNOTATION];

fn provide_types() -> &'static [&'static SemanticType] {
    LOMBOK_TYPES
}

fn provide_annotations() -> &'static [&'static XenoAnnotation] {
    LOMBOK_ANNOTATIONS
}

fn provide_config_schema() -> &'static str {
    r#"{
        "type": "object",
        "description": "Java + Lombok DTO generator with Gson support for positional tuples.",
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
    /// Complete semantic type graph used to expand transparent aliases.
    type_hierarchy: TypeHierarchy,
    /// Generated files for the current module: (type name, file contents).
    files: Vec<(String, String)>,
    /// Whether the current module contains a generated tuple declaration.
    has_tuples: bool,
    /// Annotation arguments are metadata and are not emitted as Java value types.
    annotation_depth: usize,
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
            type_hierarchy: TypeHierarchy::default(),
            files: Vec::new(),
            has_tuples: false,
            annotation_depth: 0,
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
        self.type_hierarchy = scope.type_hierarchy.clone();
        self.files.clear();
        self.has_tuples = false;
        self.annotation_depth = 0;
    }

    fn on_before_ast(
        &mut self,
        ast: &[Declaration<'src>],
        _errors: &mut Vec<xenomorph_common::XenoDiagnostic<'src>>,
    ) {
        for decl in ast {
            if let Declaration::Type {
                docs,
                name,
                generics,
                ty: t,
                ..
            } = decl
            {
                if matches!(&t.0, Type::Tuple(_)) {
                    self.has_tuples = true;
                }
                if let Some(content) = self.generate_type_decl(docs, name.v, generics.as_deref(), t)
                {
                    self.files.push((name.v.to_string(), content));
                }
            }
        }
    }

    fn on_simple_type(&mut self, ty: &SimpleType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
        if self.annotation_depth == 0 {
            if let Some(identifier) = simple_type_identifier(ty) {
                let resolved = self
                    .type_hierarchy
                    .resolve_transparent_aliases(&simple_to_owned_type(ty));
                if let Some(name) = first_unmapped_java_type(&resolved, &self.type_hierarchy) {
                    errors.push(unsupported_type_diagnostic("Java", identifier, &name));
                }
            }
        }

        let literal = match ty {
            SimpleType::Literal(literal) | SimpleType::OptionalLiteral(literal) => literal,
            _ => return,
        };
        let unrepresentable = match literal {
            Literal::Int(integer) => !integer_fits(&integer.value, integer.representation),
            Literal::Float(float) => {
                matches!(float.representation.size, FloatSize::F32 | FloatSize::F64)
                    && !decimal_roundtrips_as(&float.value, float.representation.size)
            }
            Literal::String(_, _) | Literal::Boolean(_, _) => false,
        };
        if unrepresentable {
            errors.push(XenoDiagnostic {
                location: literal.get_last_token().clone(),
                message: format!(
                    "Java cannot represent numeric literal '{}' using its requested representation.",
                    literal.token().v
                ),
                severity: XenoDiagSeverity::Err,
            });
        }
    }

    fn on_before_annotation(
        &mut self,
        _name: &TokenData<'src>,
        _args: &[Expr<'src>],
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.annotation_depth += 1;
    }

    fn on_after_annotation(
        &mut self,
        _name: &TokenData<'src>,
        _args: &[Expr<'src>],
        _errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
        self.annotation_depth = self.annotation_depth.saturating_sub(1);
    }

    fn on_after_module(&mut self, scope: &ScopeInfo) {
        let base = match &self.output_dir {
            Some(dir) => Config::get().workdir.join(dir),
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

        for (name, content) in self.tuple_support_files() {
            write_java_file(&package_dir, scope.module_path.as_str(), name, &content);
        }

        for (name, content) in &self.files {
            write_java_file(&package_dir, scope.module_path.as_str(), name, content);
        }
    }
}

// ── Type declaration generation ─────────────────────────────────────

impl JavaGenerator {
    fn tuple_support_files(&self) -> Vec<(&'static str, String)> {
        if !self.has_tuples {
            return Vec::new();
        }

        vec![
            ("JsonTuple", generate_json_tuple_annotation(&self.package)),
            (
                "TupleTypeAdapterFactory",
                generate_tuple_type_adapter_factory(&self.package),
            ),
        ]
    }

    fn generate_type_decl(
        &self,
        docs: &Option<&str>,
        name: &str,
        generics: Option<&[(&TokenData, Option<&TokenData>)]>,
        t: &XenoType,
    ) -> Option<String> {
        let lombok_decorators = collect_lombok_decorators(&t.1);

        match &t.0 {
            Type::Struct(fields) => Some(self.generate_class(
                docs,
                name,
                &format_generic_params(generics),
                fields,
                &lombok_decorators,
            )),
            Type::Tuple(items) => Some(self.generate_tuple_class(
                docs,
                name,
                &format_generic_params(generics),
                items,
                &lombok_decorators,
            )),
            Type::Enum(variants) => Some(self.generate_enum(docs, name, variants)),
            _ => None,
        }
    }

    // ── Class (struct) generation ───────────────────────────────────

    fn generate_class(
        &self,
        docs: &Option<&str>,
        name: &str,
        generic_params: &str,
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
            let literal = required_literal(value);

            if let Some(docs) = docs {
                push_indented_javadoc(&mut field_block, extract_documentation(docs), "    ");
            }
            if !nullable && literal.is_none() {
                field_block.push_str("    @NonNull\n");
                register_lombok_import(&mut imports, "NonNull");
            }
            if let Some(literal) = literal {
                field_block.push_str(&format!(
                    "    private final {java_type} {} = {};\n",
                    key.v,
                    literal_to_java(literal, &java_type)
                ));
            } else {
                field_block.push_str(&format!("    private {java_type} {};\n", key.v));
            }
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
        out.push_str(&format!("public class {name}{generic_params} {{\n"));
        out.push_str(&field_block);
        out.push_str("}\n");
        out
    }

    fn generate_tuple_class(
        &self,
        docs: &Option<&str>,
        name: &str,
        generic_params: &str,
        items: &[SimpleType],
        type_lombok_decorators: &[String],
    ) -> String {
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let class_annotations = self.class_annotations(type_lombok_decorators);
        imports.insert("com.google.gson.annotations.SerializedName".to_string());

        let mut field_block = String::new();
        for (index, item) in items.iter().enumerate() {
            let java_type = self.type_to_java(item, &mut imports);
            let literal = required_literal(item);

            field_block.push_str(&format!("    @SerializedName(\"item{index}\")\n"));
            if !is_optional(item) && literal.is_none() {
                field_block.push_str("    @NonNull\n");
                register_lombok_import(&mut imports, "NonNull");
            }
            if let Some(literal) = literal {
                field_block.push_str(&format!(
                    "    private final {java_type} item{index} = {};\n",
                    literal_to_java(literal, &java_type)
                ));
            } else {
                field_block.push_str(&format!("    private {java_type} item{index};\n"));
            }
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
        out.push_str("@JsonTuple\n");
        for annotation in &class_annotations {
            out.push_str(&format!("@{annotation}\n"));
        }
        out.push_str(&format!("public class {name}{generic_params} {{\n"));
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
                .all(|(_, value, _)| matches!(value, SimpleType::Literal(Literal::Int(_))));

        let mut imports = BTreeSet::new();
        let enum_value_type = if all_int {
            let literals = variants.iter().filter_map(|(_, value, _)| match value {
                SimpleType::Literal(Literal::Int(integer)) => Some(integer),
                _ => None,
            });
            Some(common_java_integer_type(literals, &mut imports))
        } else {
            None
        };

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
        out.push_str(&format!("public enum {name} {{\n"));

        if let Some(java_type) = enum_value_type {
            for (i, (key, value, docs)) in variants.iter().enumerate() {
                let sep = if i + 1 == variants.len() { ";" } else { "," };
                let value = match value {
                    SimpleType::Literal(literal @ Literal::Int(_)) => {
                        literal_to_java(literal, java_type)
                    }
                    _ => "0".to_string(),
                };
                if let Some(docs) = docs {
                    push_indented_javadoc(&mut out, extract_documentation(docs), "    ");
                }
                out.push_str(&format!("    {}({value}){sep}\n", key.v));
            }
            out.push('\n');
            out.push_str(&format!("    private final {java_type} value;\n\n"));
            out.push_str(&format!("    {name}({java_type} value) {{\n"));
            out.push_str("        this.value = value;\n");
            out.push_str("    }\n\n");
            out.push_str(&format!("    public {java_type} getValue() {{\n"));
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
            SimpleType::Identifier(_, _)
            | SimpleType::OptionalIdentifier(_, _)
            | SimpleType::Array(_, _)
            | SimpleType::OptionalArray(_, _) => {
                let owned = simple_to_owned_type(ty);
                let resolved = self.type_hierarchy.resolve_transparent_aliases(&owned);
                self.owned_type_to_java(&resolved, imports)
            }
            SimpleType::Literal(Literal::String(_, _))
            | SimpleType::OptionalLiteral(Literal::String(_, _)) => "String".to_string(),
            SimpleType::Literal(Literal::Boolean(_, _))
            | SimpleType::OptionalLiteral(Literal::Boolean(_, _)) => "Boolean".to_string(),
            SimpleType::Literal(Literal::Int(integer))
            | SimpleType::OptionalLiteral(Literal::Int(integer)) => {
                java_integer_type(integer.representation, imports).to_string()
            }
            SimpleType::Literal(Literal::Float(float))
            | SimpleType::OptionalLiteral(Literal::Float(float)) => {
                java_float_type(float.representation.size, imports).to_string()
            }
        }
    }

    fn owned_type_to_java(&self, ty: &OwnedType, imports: &mut BTreeSet<String>) -> String {
        match ty {
            OwnedType::Array(inner) => {
                imports.insert("java.util.List".to_string());
                format!("List<{}>", self.owned_type_to_java(inner, imports))
            }
            OwnedType::Generic { name, .. } => name.clone(),
            OwnedType::Named { name, arguments }
            | OwnedType::Qualified {
                name, arguments, ..
            } => self.named_owned_type_to_java(name, arguments, imports),
        }
    }

    fn named_owned_type_to_java(
        &self,
        name: &str,
        arguments: &[OwnedType],
        imports: &mut BTreeSet<String>,
    ) -> String {
        if name == "array" {
            imports.insert("java.util.List".to_string());
            let element = arguments
                .first()
                .map(|argument| self.owned_type_to_java(argument, imports))
                .unwrap_or_else(|| "Object".to_string());
            return format!("List<{element}>");
        }
        if name == "dict" {
            imports.insert("java.util.Map".to_string());
            return match arguments {
                [] => "Map<String, Object>".to_string(),
                _ => format!(
                    "Map<{}>",
                    arguments
                        .iter()
                        .map(|argument| self.owned_type_to_java(argument, imports))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        }

        if let Some(base) = native_type_to_java(name, imports) {
            return base;
        }
        if arguments.is_empty() {
            name.to_string()
        } else {
            format!(
                "{name}<{}>",
                arguments
                    .iter()
                    .map(|argument| self.owned_type_to_java(argument, imports))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Maps a Xenomorph semantic type to a Java type, registering imports for
/// types outside `java.lang`. `None` means Java has no native mapping.
fn native_type_to_java(name: &str, imports: &mut BTreeSet<String>) -> Option<String> {
    Some(match name {
        "string" | "char" | "email" | "url" | "hostname" | "ip" | "ipv4" | "ipv6" | "semver"
        | "strong_password" | "json" | "xml" | "yaml" | "toml" | "csv" | "tsv" => {
            "String".to_string()
        }
        "bool" => "Boolean".to_string(),
        "i4" | "i8" | "u4" => "Byte".to_string(),
        "i16" | "u8" => "Short".to_string(),
        "i32" | "u16" => "Integer".to_string(),
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
        "array" => {
            imports.insert("java.util.List".to_string());
            "List<Object>".to_string()
        }
        "dict" => {
            imports.insert("java.util.Map".to_string());
            "Map<String, Object>".to_string()
        }
        "any" | "null" => "Object".to_string(),
        _ => return None,
    })
}

fn simple_type_identifier<'src>(ty: &SimpleType<'src>) -> Option<&'src TokenData<'src>> {
    match ty {
        SimpleType::Identifier(identifier, _)
        | SimpleType::OptionalIdentifier(identifier, _)
        | SimpleType::Array(identifier, _)
        | SimpleType::OptionalArray(identifier, _) => Some(identifier),
        SimpleType::Literal(_) | SimpleType::OptionalLiteral(_) => None,
    }
}

fn first_unmapped_java_type(ty: &OwnedType, hierarchy: &TypeHierarchy) -> Option<String> {
    match ty {
        OwnedType::Array(inner) => first_unmapped_java_type(inner, hierarchy),
        OwnedType::Generic { .. } => None,
        OwnedType::Named { name, arguments } => {
            let is_static = hierarchy
                .get_type(name)
                .is_some_and(|definition| definition.module_path.is_none());
            let mut imports = BTreeSet::new();
            if is_static && native_type_to_java(name, &mut imports).is_none() {
                return Some(name.clone());
            }
            arguments
                .iter()
                .find_map(|argument| first_unmapped_java_type(argument, hierarchy))
        }
        OwnedType::Qualified {
            module_path,
            name,
            arguments,
        } => {
            let mut imports = BTreeSet::new();
            if module_path.is_none() && native_type_to_java(name, &mut imports).is_none() {
                return Some(name.clone());
            }
            arguments
                .iter()
                .find_map(|argument| first_unmapped_java_type(argument, hierarchy))
        }
    }
}

fn unsupported_type_diagnostic<'src>(
    target: &str,
    identifier: &'src TokenData<'src>,
    name: &str,
) -> XenoDiagnostic<'src> {
    XenoDiagnostic {
        location: identifier.clone(),
        message: format!(
            "{target} cannot represent type '{name}' because it has no native mapping."
        ),
        severity: XenoDiagSeverity::Err,
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
        Expr::Type(Type::Simple(SimpleType::Identifier(identifier, _))) => {
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
            | SimpleType::OptionalIdentifier(_, _)
            | SimpleType::OptionalArray(_, _)
    )
}

fn required_literal<'src>(ty: &'src SimpleType<'src>) -> Option<&'src Literal<'src>> {
    match ty {
        SimpleType::Literal(literal) => Some(literal),
        _ => None,
    }
}

fn literal_to_java(literal: &Literal, java_type: &str) -> String {
    match literal {
        Literal::String(value, _) => java_string_literal(value),
        Literal::Boolean(value, _) => value.to_string(),
        Literal::Int(integer) if java_type == "Long" || java_type == "long" => {
            format!("{}L", integer.value)
        }
        Literal::Int(integer) if java_type == "BigInteger" => {
            format!("new BigInteger(\"{}\")", integer.value)
        }
        Literal::Int(integer) => integer.value.to_string(),
        Literal::Float(float) if java_type == "Float" => format!("{}F", float.value),
        Literal::Float(float) if java_type == "BigDecimal" => {
            format!("new BigDecimal(\"{}\")", float.value)
        }
        Literal::Float(float) => format!("{}D", float.value),
    }
}

fn java_integer_type(
    representation: IntegerRepresentation,
    imports: &mut BTreeSet<String>,
) -> &'static str {
    let primitive = match (representation.signed, representation.size) {
        (_, IntegerSize::Arbitrary) => None,
        (true, IntegerSize::Bits(0..=8)) | (false, IntegerSize::Bits(0..=7)) => Some("Byte"),
        (true, IntegerSize::Bits(9..=16)) | (false, IntegerSize::Bits(8..=15)) => Some("Short"),
        (true, IntegerSize::Bits(17..=32)) | (false, IntegerSize::Bits(16..=31)) => Some("Integer"),
        (true, IntegerSize::Bits(33..=64)) | (false, IntegerSize::Bits(32..=63)) => Some("Long"),
        _ => None,
    };
    primitive.unwrap_or_else(|| {
        imports.insert("java.math.BigInteger".to_string());
        "BigInteger"
    })
}

fn java_float_type(size: FloatSize, imports: &mut BTreeSet<String>) -> &'static str {
    match size {
        FloatSize::F32 => "Float",
        FloatSize::F64 => "Double",
        FloatSize::Decimal => {
            imports.insert("java.math.BigDecimal".to_string());
            "BigDecimal"
        }
    }
}

fn common_java_integer_type<'a>(
    literals: impl Iterator<Item = &'a IntLiteral<'a>>,
    imports: &mut BTreeSet<String>,
) -> &'static str {
    let widest = literals
        .map(|literal| java_integer_type(literal.representation, imports))
        .max_by_key(|ty| match *ty {
            "Byte" => 0,
            "Short" => 1,
            "Integer" => 2,
            "Long" => 3,
            "BigInteger" => 4,
            _ => unreachable!(),
        })
        .unwrap_or("Integer");
    match widest {
        "Byte" | "Short" | "Integer" => "int",
        "Long" => "long",
        other => other,
    }
}

fn java_string_literal(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            control if control.is_control() => {
                output.push_str(&format!("\\u{:04X}", control as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn format_generic_params(generics: Option<&[(&TokenData, Option<&TokenData>)]>) -> String {
    let Some(generics) = generics.filter(|generics| !generics.is_empty()) else {
        return String::new();
    };

    format!(
        "<{}>",
        generics
            .iter()
            .map(|(name, _constraint)| name.v)
            .collect::<Vec<_>>()
            .join(", ")
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

fn write_java_file(package_dir: &std::path::Path, module_path: &str, name: &str, content: &str) {
    let path = package_dir.join(format!("{name}.java"));
    match fs::write(&path, content) {
        Ok(_) => println!("✓ {module_path} → {}", path.display()),
        Err(error) => eprintln!("✗ {module_path} — failed to write: {error}"),
    }
}

fn generated_java_header(package: &str) -> String {
    let mut output = String::from("// Auto-generated by xenomorph-java — do not edit.\n");
    if !package.is_empty() {
        output.push_str(&format!("package {package};\n"));
    }
    output.push('\n');
    output
}

fn generate_json_tuple_annotation(package: &str) -> String {
    let mut output = generated_java_header(package);
    output.push_str(
        r#"import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a generated class as having a positional JSON-array representation.
 *
 * <p>Register {@link TupleTypeAdapterFactory} on every Gson instance that
 * serializes or deserializes generated tuple classes.</p>
 */
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.TYPE)
public @interface JsonTuple {
}
"#,
    );
    output
}

fn generate_tuple_type_adapter_factory(package: &str) -> String {
    let mut output = generated_java_header(package);
    output.push_str(
        r#"import com.google.gson.Gson;
import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParseException;
import com.google.gson.TypeAdapter;
import com.google.gson.TypeAdapterFactory;
import com.google.gson.reflect.TypeToken;
import com.google.gson.stream.JsonReader;
import com.google.gson.stream.JsonToken;
import com.google.gson.stream.JsonWriter;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Gson adapter factory for classes generated with {@link JsonTuple}.
 *
 * <p>Tuple fields must be named {@code item0}, {@code item1}, and so on. The
 * fields are serialized as a JSON array in numeric field order.</p>
 *
 * <p>Register the factory on every Gson instance that handles generated
 * tuples:</p>
 *
 * <pre>{@code
 * Gson gson = new GsonBuilder()
 *         .registerTypeAdapterFactory(new TupleTypeAdapterFactory())
 *         .create();
 * }</pre>
 */
public final class TupleTypeAdapterFactory implements TypeAdapterFactory {

    private static final Pattern ITEM_PATTERN = Pattern.compile("^item(\\d+)$");

    @Override
    public <T> TypeAdapter<T> create(Gson gson, TypeToken<T> requestedType) {
        Class<? super T> rawType = requestedType.getRawType();

        if (!rawType.isAnnotationPresent(JsonTuple.class)) {
            return null;
        }

        List<String> fieldNames = findTupleFieldNames(rawType);
        TypeAdapter<T> delegate = gson.getDelegateAdapter(this, requestedType);
        TypeAdapter<JsonElement> elementAdapter = gson.getAdapter(JsonElement.class);

        return new TupleAdapter<>(rawType, delegate, elementAdapter, fieldNames).nullSafe();
    }

    private static List<String> findTupleFieldNames(Class<?> tupleClass) {
        List<FieldAndIndex> indexedFields = new ArrayList<>();

        for (Field field : tupleClass.getDeclaredFields()) {
            int modifiers = field.getModifiers();
            if (field.isSynthetic()
                    || Modifier.isStatic(modifiers)
                    || Modifier.isTransient(modifiers)) {
                continue;
            }

            Matcher matcher = ITEM_PATTERN.matcher(field.getName());
            if (!matcher.matches()) {
                continue;
            }

            int index;
            try {
                index = Integer.parseInt(matcher.group(1));
            } catch (NumberFormatException exception) {
                throw new JsonParseException(
                        "Invalid tuple field name: "
                                + tupleClass.getName()
                                + "."
                                + field.getName(),
                        exception
                );
            }

            indexedFields.add(new FieldAndIndex(field.getName(), index));
        }

        indexedFields.sort(Comparator.comparingInt(value -> value.index));

        List<String> result = new ArrayList<>(indexedFields.size());
        for (int expectedIndex = 0;
             expectedIndex < indexedFields.size();
             expectedIndex++) {
            FieldAndIndex value = indexedFields.get(expectedIndex);
            if (value.index != expectedIndex) {
                throw new JsonParseException(
                        "Non-contiguous tuple fields on "
                                + tupleClass.getName()
                                + ": expected item"
                                + expectedIndex
                                + " but found "
                                + value.fieldName
                );
            }
            result.add(value.fieldName);
        }

        return result;
    }

    private static final class FieldAndIndex {
        private final String fieldName;
        private final int index;

        private FieldAndIndex(String fieldName, int index) {
            this.fieldName = fieldName;
            this.index = index;
        }
    }

    private static final class TupleAdapter<T> extends TypeAdapter<T> {
        private final Class<?> tupleClass;
        private final TypeAdapter<T> delegate;
        private final TypeAdapter<JsonElement> elementAdapter;
        private final List<String> fieldNames;

        private TupleAdapter(
                Class<?> tupleClass,
                TypeAdapter<T> delegate,
                TypeAdapter<JsonElement> elementAdapter,
                List<String> fieldNames
        ) {
            this.tupleClass = tupleClass;
            this.delegate = delegate;
            this.elementAdapter = elementAdapter;
            this.fieldNames = fieldNames;
        }

        @Override
        public T read(JsonReader in) throws IOException {
            if (in.peek() != JsonToken.BEGIN_ARRAY) {
                throw new JsonParseException(
                        "Expected a JSON array for tuple "
                                + tupleClass.getName()
                                + " but found "
                                + in.peek()
                                + " at "
                                + in.getPath()
                );
            }

            JsonObject tupleObject = new JsonObject();
            in.beginArray();

            for (int index = 0; index < fieldNames.size(); index++) {
                if (!in.hasNext()) {
                    throw new JsonParseException(
                            "Expected "
                                    + fieldNames.size()
                                    + " values for tuple "
                                    + tupleClass.getName()
                                    + ", but the array ended at index "
                                    + index
                                    + " at "
                                    + in.getPath()
                    );
                }

                tupleObject.add(fieldNames.get(index), elementAdapter.read(in));
            }

            if (in.hasNext()) {
                throw new JsonParseException(
                        "Expected exactly "
                                + fieldNames.size()
                                + " values for tuple "
                                + tupleClass.getName()
                                + ", but the JSON array contains additional values at "
                                + in.getPath()
                );
            }

            in.endArray();
            try {
                return delegate.fromJsonTree(tupleObject);
            } catch (JsonParseException exception) {
                throw exception;
            } catch (RuntimeException exception) {
                throw new JsonParseException(
                        "Could not deserialize tuple " + tupleClass.getName(),
                        exception
                );
            }
        }

        @Override
        public void write(JsonWriter out, T tuple) throws IOException {
            JsonElement serialized;
            try {
                serialized = delegate.toJsonTree(tuple);
            } catch (RuntimeException exception) {
                throw new JsonParseException(
                        "Could not serialize tuple " + tupleClass.getName(),
                        exception
                );
            }

            if (!serialized.isJsonObject()) {
                throw new JsonParseException(
                        "Expected Gson to serialize tuple "
                                + tupleClass.getName()
                                + " as an object before array conversion"
                );
            }

            JsonObject tupleObject = serialized.getAsJsonObject();
            out.beginArray();
            for (String fieldName : fieldNames) {
                JsonElement value = tupleObject.get(fieldName);
                elementAdapter.write(out, value == null ? JsonNull.INSTANCE : value);
            }
            out.endArray();
        }
    }
}
"#,
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::semantic::{GenericParameterInfo, TypeDeclarationInfo, BUILTIN_TYPES};
    use xenomorph_common::{
        lexer::Lexer,
        parser::{IntegerRepresentation, IntegerSize, Parser},
    };

    fn imports() -> BTreeSet<String> {
        BTreeSet::new()
    }

    fn parse(source: &str) -> Vec<Declaration<'_>> {
        let tokens = Box::leak(Box::new(
            Lexer::tokenize(Box::leak(source.to_string().into_boxed_str()))
                .expect("Java generator fixture should lex"),
        ));
        let (ast, diagnostics) = Parser::parse(tokens);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:#?}"
        );
        ast
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
                SimpleType::Identifier(&field_type, None),
                Some(&field_docs),
            )]),
            vec![],
        );

        let output = JavaGenerator::new()
            .generate_type_decl(&Some("A user-facing profile."), "Profile", None, &ty)
            .expect("structs generate Java classes");

        assert!(output.contains(" * A user-facing profile."));
        assert!(output.contains("     * Name shown to users."));
        assert!(output.contains("    private String displayName;"));
    }

    #[test]
    fn test_builtin_mappings() {
        let mut i = imports();
        assert_eq!(
            native_type_to_java("string", &mut i).as_deref(),
            Some("String")
        );
        assert_eq!(
            native_type_to_java("bool", &mut i).as_deref(),
            Some("Boolean")
        );
        assert_eq!(native_type_to_java("i8", &mut i).as_deref(), Some("Byte"));
        assert_eq!(native_type_to_java("u8", &mut i).as_deref(), Some("Short"));
        assert_eq!(native_type_to_java("i16", &mut i).as_deref(), Some("Short"));
        assert_eq!(
            native_type_to_java("u16", &mut i).as_deref(),
            Some("Integer")
        );
        assert_eq!(native_type_to_java("i64", &mut i).as_deref(), Some("Long"));
        assert_eq!(
            native_type_to_java("u64", &mut i).as_deref(),
            Some("BigInteger")
        );
        assert_eq!(native_type_to_java("f32", &mut i).as_deref(), Some("Float"));
        assert_eq!(
            native_type_to_java("f64", &mut i).as_deref(),
            Some("Double")
        );
        assert_eq!(
            native_type_to_java("decimal", &mut i).as_deref(),
            Some("BigDecimal")
        );
        assert_eq!(native_type_to_java("uuid", &mut i).as_deref(), Some("UUID"));
        assert_eq!(
            native_type_to_java("binary", &mut i).as_deref(),
            Some("byte[]")
        );
        assert_eq!(native_type_to_java("MyType", &mut i), None);
        assert!(i.contains("java.math.BigInteger"));
        assert!(i.contains("java.util.UUID"));
    }

    #[test]
    fn integer_literal_types_are_the_smallest_lossless_java_types() {
        let mut i = imports();
        assert_eq!(
            java_integer_type(
                IntegerRepresentation {
                    signed: false,
                    size: IntegerSize::Bits(7),
                },
                &mut i,
            ),
            "Byte"
        );
        assert_eq!(
            java_integer_type(
                IntegerRepresentation {
                    signed: false,
                    size: IntegerSize::Bits(8),
                },
                &mut i,
            ),
            "Short"
        );
        assert_eq!(
            java_integer_type(
                IntegerRepresentation {
                    signed: false,
                    size: IntegerSize::Bits(64),
                },
                &mut i,
            ),
            "BigInteger"
        );
        assert!(i.contains("java.math.BigInteger"));
    }

    #[test]
    fn exact_decimal_literals_use_big_decimal() {
        let mut i = imports();
        assert_eq!(java_float_type(FloatSize::Decimal, &mut i), "BigDecimal");
        assert!(i.contains("java.math.BigDecimal"));
    }

    #[test]
    fn generator_rejects_an_integer_that_violates_its_representation() {
        let token = token("256");
        let ty = SimpleType::Literal(Literal::Int(IntLiteral {
            value: 256.into(),
            representation: IntegerRepresentation {
                signed: false,
                size: IntegerSize::Bits(8),
            },
            token: &token,
            cast: None,
        }));
        let mut generator = JavaGenerator::new();
        let mut errors = Vec::new();

        generator.on_simple_type(&ty, &mut errors);

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("cannot represent"));
    }

    #[test]
    fn test_generic_aliases_are_expanded_in_generic_classes() {
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_declaration(
            "test",
            "UserName",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("T")],
                transparent_alias: true,
            },
        );

        let type_parameter = token("T");
        let constraint = token("HasLength");
        let field_name = token("name");
        let alias_name = token("UserName");
        let argument = token("T");
        let generics = [(&type_parameter, Some(&constraint))];
        let ty = (
            Type::Struct(vec![(
                &field_name,
                SimpleType::Identifier(
                    &alias_name,
                    Some(vec![SimpleType::Identifier(&argument, None)]),
                ),
                None,
            )]),
            vec![],
        );

        let output = generator
            .generate_type_decl(&None, "User", Some(&generics), &ty)
            .expect("structs generate Java classes");

        assert!(output.contains("public class User<T> {"));
        assert!(output.contains("    private T name;"));
        assert!(!output.contains("UserName<T>"));
    }

    #[test]
    fn test_array_aliases_are_expanded_recursively() {
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_declaration(
            "test",
            "Positive",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("T")],
                transparent_alias: true,
            },
        );
        generator.type_hierarchy.insert_declaration(
            "test",
            "NonEmpty",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::Array(Box::new(OwnedType::Named {
                    name: "Positive".to_string(),
                    arguments: vec![OwnedType::named("T")],
                }))],
                transparent_alias: true,
            },
        );

        let field_name = token("values");
        let alias_name = token("NonEmpty");
        let argument = token("u8");
        let ty = (
            Type::Struct(vec![(
                &field_name,
                SimpleType::Identifier(
                    &alias_name,
                    Some(vec![SimpleType::Identifier(&argument, None)]),
                ),
                None,
            )]),
            vec![],
        );

        let output = generator
            .generate_type_decl(&None, "Store", None, &ty)
            .expect("structs generate Java classes");

        assert!(output.contains("import java.util.List;"));
        assert!(output.contains("    private List<Short> values;"));
    }

    #[test]
    fn test_concrete_generic_classes_keep_type_arguments() {
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_declaration(
            "test",
            "Box",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("dict")],
                transparent_alias: false,
            },
        );

        let field_name = token("box");
        let box_name = token("Box");
        let argument = token("string");
        let ty = (
            Type::Struct(vec![(
                &field_name,
                SimpleType::Identifier(
                    &box_name,
                    Some(vec![SimpleType::Identifier(&argument, None)]),
                ),
                None,
            )]),
            vec![],
        );

        let output = generator
            .generate_type_decl(&None, "Holder", None, &ty)
            .expect("structs generate Java classes");

        assert!(output.contains("    private Box<String> box;"));
    }

    #[test]
    fn tuple_declarations_generate_positional_classes() {
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_declaration(
            "test",
            "UserName",
            TypeDeclarationInfo {
                generic_params: vec![generic("T")],
                parents: vec![OwnedType::named("T")],
                transparent_alias: true,
            },
        );
        generator.type_hierarchy.insert_declaration(
            "test",
            "UserAge",
            TypeDeclarationInfo {
                generic_params: vec![],
                parents: vec![OwnedType::named("u8")],
                transparent_alias: true,
            },
        );

        let user_name = token("UserName");
        let string = token("string");
        let user_age = token("UserAge");
        let email = token("email");
        let bool_type = token("bool");
        let ty = (
            Type::Tuple(vec![
                SimpleType::Identifier(
                    &user_name,
                    Some(vec![SimpleType::Identifier(&string, None)]),
                ),
                SimpleType::Identifier(&user_age, None),
                SimpleType::Identifier(&email, None),
                SimpleType::Identifier(&bool_type, None),
            ]),
            vec![],
        );

        let output = generator
            .generate_type_decl(&None, "TestTuple", None, &ty)
            .expect("tuple declarations generate Java classes");

        assert!(output.contains("public class TestTuple {"));
        assert!(output.contains("@JsonTuple"));
        assert!(output.contains("import com.google.gson.annotations.SerializedName;"));
        assert!(output.contains("    @SerializedName(\"item0\")"));
        assert!(output.contains("    private String item0;"));
        assert!(output.contains("    private Short item1;"));
        assert!(output.contains("    private String item2;"));
        assert!(output.contains("    private Boolean item3;"));
    }

    #[test]
    fn tuple_references_have_a_generated_java_file() {
        let ast = parse(
            "type TestTuple = [string, u8, email, bool]; type UserType = { meta: TestTuple };",
        );
        let mut generator = generator_with_builtins();

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generator.files.iter().any(|(name, contents)| {
            name == "TestTuple" && contents.contains("public class TestTuple")
        }));
        assert!(generator.files.iter().any(|(name, contents)| {
            name == "UserType" && contents.contains("private TestTuple meta;")
        }));
        assert_eq!(
            generator
                .tuple_support_files()
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            vec!["JsonTuple", "TupleTypeAdapterFactory"]
        );
    }

    #[test]
    fn tuple_support_is_not_emitted_for_tuple_free_modules() {
        let ast = parse("type UserType = { name: string };");
        let mut generator = generator_with_builtins();

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generator.tuple_support_files().is_empty());
    }

    #[test]
    fn generated_tuple_support_documents_and_implements_gson_registration() {
        let annotation = generate_json_tuple_annotation("com.example.model");
        let factory = generate_tuple_type_adapter_factory("com.example.model");

        assert!(annotation.contains("package com.example.model;"));
        assert!(annotation.contains("@Retention(RetentionPolicy.RUNTIME)"));
        assert!(factory.contains("package com.example.model;"));
        assert!(factory.contains("implements TypeAdapterFactory"));
        assert!(factory.contains(".registerTypeAdapterFactory(new TupleTypeAdapterFactory())"));
        assert!(factory.contains("Pattern.compile(\"^item(\\\\d+)$\")"));
        assert!(factory.contains("gson.getDelegateAdapter(this, requestedType)"));
        assert!(factory.contains("Expected exactly "));
    }

    #[test]
    fn test_required_literals_are_initialized_final_fields() {
        let field_name = token("Ty");
        let literal_token = token("\"#nv\\\"Store\"");
        let ty = (
            Type::Struct(vec![(
                &field_name,
                SimpleType::Literal(Literal::String("#nv\"Store".to_string(), &literal_token)),
                None,
            )]),
            vec![],
        );

        let output = JavaGenerator::new()
            .generate_type_decl(&None, "Store", None, &ty)
            .expect("structs generate Java classes");

        assert!(output.contains("    private final String Ty = \"#nv\\\"Store\";"));
        assert!(!output.contains("@NonNull"));
    }

    #[test]
    fn test_parameterized_dict_maps_directly_to_java_map() {
        let generator = generator_with_builtins();
        let dict = OwnedType::Named {
            name: "dict".to_string(),
            arguments: vec![OwnedType::named("string"), OwnedType::named("u8")],
        };
        let mut i = imports();

        assert_eq!(
            generator.owned_type_to_java(&dict, &mut i),
            "Map<String, Short>"
        );
        assert!(i.contains("java.util.Map"));
    }

    #[test]
    fn generator_reports_static_types_without_a_java_mapping() {
        static SYMBOL: SemanticType = SemanticType {
            name: "symbol",
            documentation: None,
            generic_params: None,
            parents: None,
        };
        let symbol = token("symbol");
        let ty = SimpleType::Identifier(&symbol, None);
        let mut generator = JavaGenerator::new();
        generator.type_hierarchy.insert_semantic_type(&SYMBOL);
        let mut errors = Vec::new();

        generator.on_simple_type(&ty, &mut errors);

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "Java cannot represent type 'symbol' because it has no native mapping."
        );
        assert_eq!(errors[0].severity, XenoDiagSeverity::Err);
    }

    #[test]
    fn generator_reports_aliases_to_unmapped_static_types() {
        static SYMBOL: SemanticType = SemanticType {
            name: "symbol",
            documentation: None,
            generic_params: None,
            parents: None,
        };
        let alias = token("Alias");
        let ty = SimpleType::Identifier(&alias, None);
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_semantic_type(&SYMBOL);
        generator.type_hierarchy.insert_declaration(
            "test",
            "Alias",
            TypeDeclarationInfo {
                generic_params: Vec::new(),
                parents: vec![OwnedType::named("symbol")],
                transparent_alias: true,
            },
        );
        let mut errors = Vec::new();

        generator.on_simple_type(&ty, &mut errors);

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "Java cannot represent type 'symbol' because it has no native mapping."
        );
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

    #[test]
    fn lombok_types_own_the_lombok_trait() {
        assert!(LOMBOK_TYPES
            .iter()
            .all(|semantic_type| semantic_type.implements(&LOMBOK_TRAIT)));
        assert!(xenomorph_common::semantic::BUILTIN_TYPES
            .iter()
            .all(|semantic_type| !semantic_type.implements(&LOMBOK_TRAIT)));
    }

    fn token(value: &'static str) -> TokenData<'static> {
        TokenData {
            v: value,
            l: 0,
            c: 0,
        }
    }

    fn generic(name: &str) -> GenericParameterInfo {
        GenericParameterInfo {
            name: name.to_string(),
            constraint: None,
            constraint_scope: None,
        }
    }

    fn generator_with_builtins() -> JavaGenerator {
        let mut generator = JavaGenerator::new();
        generator.type_hierarchy.set_current_module("test");
        generator.type_hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            generator.type_hierarchy.insert_semantic_type(semantic_type);
        }
        generator
    }
}
