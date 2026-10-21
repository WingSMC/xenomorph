use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use xenomorph_common::config::{Config, ConfigValue};
use xenomorph_common::parser::{
    decimal_roundtrips_as, integer_fits, Annotation, Declaration, Expr, FloatSize, IntLiteral,
    IntegerRepresentation, IntegerSize, KeyValExpr, Literal, SimpleType, Type, XenoType,
};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::semantic::{
    is_type_utility, simple_to_owned_type, type_to_owned_type, unsupported_target_type_diagnostic,
    AnalyzerListener, OwnedField, OwnedLiteral, OwnedSet, OwnedType, ScopeInfo, TypeDefinition,
    TypeHierarchy, XenoAnnotation, XenoAnnotationKind, XenoConstraint, XenoParam, XenoParent,
    XenoTrait, XenoTraitKind, XenoType as SemanticType, ANY_TARGET_PARAM,
};
use xenomorph_common::utils::extract_documentation;
use xenomorph_common::{TokenData, XenoDiagSeverity, XenoDiagnostic};

// ── Plugin registration ─────────────────────────────────────────────

static NAME: &str = "java";
static VERSION: &str = env!("CARGO_PKG_VERSION");
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    namespace: "java",
    version: VERSION,
    initialize: None,
    shutdown: None,
    provide_types: Some(provide_types),
    provide_annotations: Some(provide_annotations),
    provide_custom_declarations: None,
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
    optional: false,
};

static LOMBOK_ANNOTATION: XenoAnnotation = XenoAnnotation {
    name: "Lombok",
    documentation: Some(
        "Attaches one or more supported Lombok decorators to the generated Java DTO, e.g. `@java::Lombok(java::Data, java::Builder)`.",
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

xenomorph_common::export_plugin!(PLUGIN);

// ── Generator listener ──────────────────────────────────────────────

#[derive(Clone, Copy)]
struct DeclarationMetadata<'a> {
    docs: Option<&'a str>,
    deprecated: bool,
    reason: Option<&'a str>,
}

impl<'a> DeclarationMetadata<'a> {
    fn new(docs: &'a Option<String>, ty: &'a XenoType, source: &str) -> Self {
        let annotation =
            ty.1.iter()
                .find(|annotation| annotation.ident.text(source) == "deprecated");
        Self {
            docs: docs.as_deref(),
            deprecated: annotation.is_some(),
            reason: annotation.and_then(|annotation| match annotation.params.first() {
                Some(Expr::Type(Type::Simple(SimpleType::Literal(Literal::String(reason, _))))) => {
                    Some(reason.as_str())
                }
                _ => None,
            }),
        }
    }
}

struct JavaConstant {
    name: String,
    java_type: String,
    initializer: String,
    docs: Option<String>,
    deprecated: bool,
    reason: Option<String>,
}

/// Identity of a source declaration: its module path and name.
type DeclarationKey = (Option<String>, String);

/// A literal set or literal sum generated as a Java enum.
struct LiteralEnum {
    /// `String`, a numeric primitive, `BigInteger`, or `BigDecimal`.
    value_type: &'static str,
    literals: Vec<OwnedLiteral>,
}

impl LiteralEnum {
    fn is_numeric(&self) -> bool {
        self.value_type != "String"
    }
}

struct JavaGenerator {
    source: String,
    workdir: PathBuf,
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
    /// Homogeneous literal collections emitted as `Xenomorph` constants.
    constants: Vec<JavaConstant>,
    /// Marker interfaces each class-backed declaration implements, collected
    /// from the sums of every module in the type hierarchy.
    sum_parents: BTreeMap<DeclarationKey, BTreeSet<String>>,
    /// Whether the current module contains a generated tuple declaration.
    has_tuples: bool,
    /// Annotation arguments are metadata and are not emitted as Java value types.
    annotation_depth: usize,
}

impl JavaGenerator {
    fn new() -> Self {
        Self {
            source: String::new(),
            workdir: PathBuf::new(),
            abs_path: PathBuf::new(),
            module_path: String::new(),
            output_dir: None,
            package: String::new(),
            blanket_value: false,
            blanket_builder: false,
            blanket_data: false,
            type_hierarchy: TypeHierarchy::default(),
            files: Vec::new(),
            constants: Vec::new(),
            sum_parents: BTreeMap::new(),
            has_tuples: false,
            annotation_depth: 0,
        }
    }
}

fn create_generator(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    let mut generator = JavaGenerator::new();
    generator.workdir = config.workdir.clone();
    let plugin_configs = &config.plugins.config;

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
    fn set_source(&mut self, source: &str) {
        self.source.clear();
        self.source.push_str(source);
    }
    fn on_before_module(&mut self, scope: &ScopeInfo) {
        self.abs_path = scope.abs_path.clone();
        self.module_path = scope.module_path.clone();
        self.type_hierarchy = scope.type_hierarchy.clone();
        self.files.clear();
        self.constants.clear();
        self.has_tuples = false;
        self.annotation_depth = 0;
    }

    fn on_before_ast(
        &mut self,
        ast: &[Declaration],
        errors: &mut Vec<xenomorph_common::XenoDiagnostic>,
    ) {
        self.sum_parents = self.collect_sum_parents();
        for decl in ast {
            if let Declaration::Type {
                docs,
                name,
                generics,
                ty: t,
                ..
            } = decl
            {
                let type_name = name.text(&self.source);
                if let Some(literal_enum) = self
                    .type_hierarchy
                    .get_type(type_name)
                    .and_then(|definition| self.literal_enum(definition))
                {
                    let metadata = DeclarationMetadata::new(docs, t, &self.source);
                    let constants = literal_enum_constant_names(&literal_enum.literals);
                    report_enum_constant_collisions(name, &t.0, &constants, errors);
                    let content =
                        self.generate_literal_enum(metadata, type_name, &literal_enum, &constants);
                    self.files.push((type_name.to_string(), content));
                    continue;
                }
                if let Some((java_type, initializer)) = self.constant_collection(&t.0) {
                    let metadata = DeclarationMetadata::new(docs, t, &self.source);
                    self.constants.push(JavaConstant {
                        name: type_name.to_string(),
                        java_type,
                        initializer,
                        docs: docs.clone(),
                        deprecated: metadata.deprecated,
                        reason: metadata.reason.map(str::to_string),
                    });
                    continue;
                }
                if matches!(&t.0, Type::Tuple(_)) {
                    self.has_tuples = true;
                }
                if let Some(content) =
                    self.generate_type_decl(docs, type_name, generics.as_deref(), t)
                {
                    self.files.push((type_name.to_string(), content));
                }
            }
        }
    }

    fn on_before_decl(&mut self, decl: &Declaration, errors: &mut Vec<XenoDiagnostic>) {
        if let Declaration::Type { name, generics, .. } = decl {
            report_java_keyword(name, "type", errors, &self.source);
            for (generic, _) in generics.as_deref().unwrap_or_default() {
                report_java_keyword(generic, "generic parameter", errors, &self.source);
            }
        }
    }

    fn on_before_struct(&mut self, fields: &[KeyValExpr], errors: &mut Vec<XenoDiagnostic>) {
        validate_java_member_names(fields, "field", errors, &self.source);
    }

    fn on_before_enum(&mut self, variants: &[KeyValExpr], errors: &mut Vec<XenoDiagnostic>) {
        validate_java_member_names(variants, "enum variant", errors, &self.source);
    }

    fn on_simple_type(&mut self, ty: &SimpleType, errors: &mut Vec<XenoDiagnostic>) {
        if self.annotation_depth == 0 {
            if let Some(diagnostic) = unsupported_target_type_diagnostic(
                "Java",
                ty,
                &self.type_hierarchy,
                &self.source,
                has_native_java_mapping,
            ) {
                errors.push(diagnostic);
            }
        }

        let literal = match ty.inner() {
            SimpleType::Literal(literal) => literal,
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
                tags: vec![],
                location: literal.get_last_token(),
                message: format!(
                    "Java cannot represent numeric literal '{}' using its requested representation.",
                    literal.token().text(&self.source)
                ),
                severity: XenoDiagSeverity::Err,
            });
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
        let base = match &self.output_dir {
            Some(dir) => self.workdir.join(dir),
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

        if let Some(constants) = self.constants_file() {
            write_java_file(
                &package_dir,
                scope.module_path.as_str(),
                "Xenomorph",
                &constants,
            );
        }
    }
}

// ── Type declaration generation ─────────────────────────────────────

impl JavaGenerator {
    /// The Java type a field gets when it references a source declaration
    /// that has no class of its own, or that Java represents specially.
    fn declared_type_to_java(
        &self,
        definition: &TypeDefinition,
        imports: &mut BTreeSet<String>,
    ) -> Option<String> {
        definition.module_path.as_ref()?;
        if self.literal_enum(definition).is_some()
            || self.interface_sum_members(definition).is_some()
        {
            return Some(definition.name.clone());
        }
        let body = definition.body.as_ref()?;
        match body {
            OwnedType::Set(set) if set.values.is_none() => {
                Some(self.owned_type_to_java(body, imports))
            }
            // A literal set is a value type; one without an enum (booleans)
            // takes the scalar type of its members.
            OwnedType::Set(set) => common_java_owned_literal_type(
                &set.values.as_ref()?.iter().collect::<Vec<_>>(),
                imports,
            )
            .or_else(|| Some("Object".to_string())),
            OwnedType::Tuple(items) => {
                let literals = items
                    .iter()
                    .map(|item| match item {
                        OwnedType::Literal(literal) => Some(literal),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "{}[]",
                    common_java_owned_literal_type(&literals, imports)?
                ))
            }
            // Sums mixing scalars, literals, or members without a class have
            // no common Java type.
            OwnedType::Sum(_) => Some("Object".to_string()),
            _ => None,
        }
    }

    /// Prefilled sets and sums of literals become Java enums when every
    /// literal shares one string or numeric Java type.
    fn literal_enum(&self, definition: &TypeDefinition) -> Option<LiteralEnum> {
        if definition.module_path.is_none() || !definition.generic_params.is_empty() {
            return None;
        }
        let (element, literals) = match definition.body.as_ref()? {
            OwnedType::Set(OwnedSet {
                element_type,
                values: Some(values),
            }) => (element_type.as_deref(), values.clone()),
            OwnedType::Sum(items) if !items.is_empty() => {
                let mut visiting = vec![declaration_key(definition)];
                let mut literals = Vec::new();
                for item in items {
                    for literal in self.sum_member_literals(
                        definition.module_path.as_deref(),
                        item,
                        &mut visiting,
                    )? {
                        if !literals.contains(&literal) {
                            literals.push(literal);
                        }
                    }
                }
                (None, literals)
            }
            _ => return None,
        };

        let mut imports = BTreeSet::new();
        let mut common =
            common_java_owned_literal_type(&literals.iter().collect::<Vec<_>>(), &mut imports);
        if !literals.is_empty() && common.is_none() {
            return None;
        }
        if let Some(element) = element {
            let element = self.owned_type_to_java(
                &self.type_hierarchy.resolve_for_target(element),
                &mut imports,
            );
            common = Some(match common {
                None => element,
                Some(common) => widen_java_type(&common, &element).unwrap_or(common),
            });
        }
        let value_type = match common.as_deref().unwrap_or("String") {
            "String" => "String",
            "Byte" | "Short" | "Integer" => "int",
            "Long" => "long",
            "BigInteger" => "BigInteger",
            "Float" => "float",
            "Double" => "double",
            "BigDecimal" => "BigDecimal",
            _ if literals.is_empty() => "String",
            _ => return None,
        };
        Some(LiteralEnum {
            value_type,
            literals,
        })
    }

    /// Literals a sum member contributes to a literal enum: the literal
    /// itself, or every value of a referenced literal set or literal sum.
    /// `None` when the member is not literal-only.
    fn sum_member_literals(
        &self,
        context: Option<&str>,
        member: &OwnedType,
        visiting: &mut Vec<DeclarationKey>,
    ) -> Option<Vec<OwnedLiteral>> {
        let definition = match member {
            OwnedType::Literal(literal) => return Some(vec![literal.clone()]),
            OwnedType::Named { name, arguments } if arguments.is_empty() => {
                self.type_hierarchy.get_type_from(context, name)?
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } if arguments.is_empty() => self
                .type_hierarchy
                .get_type_in(module_path.as_deref(), name)?,
            _ => return None,
        };
        let key = declaration_key(definition);
        if visiting.contains(&key) || !definition.generic_params.is_empty() {
            return None;
        }
        visiting.push(key);
        let context = definition.module_path.as_deref();
        let literals = match definition.body.as_ref()? {
            OwnedType::Set(OwnedSet {
                values: Some(values),
                ..
            }) => Some(values.clone()),
            OwnedType::Sum(items) => items.iter().try_fold(Vec::new(), |mut acc, item| {
                acc.extend(self.sum_member_literals(context, item, visiting)?);
                Some(acc)
            }),
            other => self.sum_member_literals(context, other, visiting),
        };
        visiting.pop();
        literals
    }

    /// Sums of class-backed declarations become marker interfaces. Returns
    /// the sum's direct members, or `None` when Java cannot model it so.
    fn interface_sum_members(&self, definition: &TypeDefinition) -> Option<Vec<DeclarationKey>> {
        self.interface_sum_members_inner(definition, &mut Vec::new())
    }

    fn interface_sum_members_inner(
        &self,
        definition: &TypeDefinition,
        visiting: &mut Vec<DeclarationKey>,
    ) -> Option<Vec<DeclarationKey>> {
        let OwnedType::Sum(items) = definition.body.as_ref()? else {
            return None;
        };
        if items.is_empty()
            || definition.module_path.is_none()
            || !definition.generic_params.is_empty()
        {
            return None;
        }
        let key = declaration_key(definition);
        if visiting.contains(&key) {
            return None;
        }
        visiting.push(key);
        let members = items
            .iter()
            .map(|item| self.class_backed_member(definition.module_path.as_deref(), item, visiting))
            .collect::<Option<Vec<_>>>();
        visiting.pop();
        let mut unique = Vec::new();
        for member in members? {
            if !unique.contains(&member) {
                unique.push(member);
            }
        }
        Some(unique)
    }

    /// The declaration a sum member names when Java generates a class or a
    /// marker interface for it. Transparent aliases are followed.
    fn class_backed_member(
        &self,
        context: Option<&str>,
        member: &OwnedType,
        visiting: &mut Vec<DeclarationKey>,
    ) -> Option<DeclarationKey> {
        let definition = match member {
            OwnedType::Named { name, arguments } if arguments.is_empty() => {
                self.type_hierarchy.get_type_from(context, name)?
            }
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } if arguments.is_empty() => self
                .type_hierarchy
                .get_type_in(module_path.as_deref(), name)?,
            _ => return None,
        };
        if definition.module_path.is_none() || !definition.generic_params.is_empty() {
            return None;
        }
        let key = declaration_key(definition);
        if self.generates_class(definition)
            || self
                .interface_sum_members_inner(definition, visiting)
                .is_some()
        {
            return Some(key);
        }
        match definition.body.as_ref()? {
            target @ (OwnedType::Named { .. } | OwnedType::Qualified { .. })
                if definition.transparent_alias && !visiting.contains(&key) =>
            {
                visiting.push(key);
                let member =
                    self.class_backed_member(definition.module_path.as_deref(), target, visiting);
                visiting.pop();
                member
            }
            _ => None,
        }
    }

    /// Whether the declaration generates a Java class from struct fields:
    /// a struct, a struct-shaped intersection, or a struct utility.
    fn generates_class(&self, definition: &TypeDefinition) -> bool {
        let struct_shaped = || {
            matches!(
                self.type_hierarchy.struct_fields(&OwnedType::Qualified {
                    module_path: definition.module_path.clone(),
                    name: definition.name.clone(),
                    arguments: Vec::new(),
                }),
                Ok(Some(_))
            )
        };
        match definition.body.as_ref() {
            Some(OwnedType::Struct(_)) => true,
            Some(OwnedType::Intersection(_)) => struct_shaped(),
            Some(OwnedType::Named { name, .. })
                if definition.transparent_alias && is_type_utility(name) =>
            {
                struct_shaped()
            }
            _ => false,
        }
    }

    /// Maps each class-backed declaration to the marker interfaces of the
    /// sums that list it, across every module in the type hierarchy.
    fn collect_sum_parents(&self) -> BTreeMap<DeclarationKey, BTreeSet<String>> {
        let mut parents: BTreeMap<DeclarationKey, BTreeSet<String>> = BTreeMap::new();
        for definition in self.type_hierarchy.declarations() {
            for member in self.interface_sum_members(definition).unwrap_or_default() {
                parents
                    .entry(member)
                    .or_default()
                    .insert(definition.name.clone());
            }
        }
        parents
    }

    /// Marker interfaces implemented (classes) or extended (interfaces) by
    /// the declaration named `name` in the current module.
    fn sum_interfaces_of(&self, name: &str) -> Vec<String> {
        self.type_hierarchy
            .get_type(name)
            .and_then(|definition| self.sum_parents.get(&declaration_key(definition)))
            .map(|parents| parents.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Homogeneous literal tuples, and literal sets without a Java enum
    /// (booleans), become `Xenomorph` array constants. Heterogeneous literal
    /// tuples keep their positional tuple class.
    fn constant_collection(&self, ty: &Type) -> Option<(String, String)> {
        let literals: Vec<&Literal> = match ty {
            Type::Set(set) => set.values.as_ref()?.iter().collect(),
            Type::Tuple(items) => items
                .iter()
                .map(|item| match item {
                    SimpleType::Literal(literal) => Some(literal),
                    _ => None,
                })
                .collect::<Option<_>>()?,
            _ => return None,
        };

        let mut imports = BTreeSet::new();
        let element = common_java_literal_type(&literals, &mut imports)?;
        let values = literals
            .iter()
            .map(|literal| literal_to_java(literal, &element))
            .collect::<Vec<_>>();
        Some((format!("{element}[]"), format!("{{{}}}", values.join(", "))))
    }

    fn constants_file(&self) -> Option<String> {
        if self.constants.is_empty() {
            return None;
        }

        let mut imports = BTreeSet::new();
        for constant in &self.constants {
            let java_type = &constant.java_type;
            if java_type.starts_with("BigInteger") {
                imports.insert("java.math.BigInteger".to_string());
            }
            if java_type.starts_with("BigDecimal") {
                imports.insert("java.math.BigDecimal".to_string());
            }
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
        out.push_str("public class Xenomorph {\n");
        for constant in &self.constants {
            push_declaration_metadata(
                &mut out,
                DeclarationMetadata {
                    docs: constant.docs.as_deref(),
                    deprecated: constant.deprecated,
                    reason: constant.reason.as_deref(),
                },
                "    ",
            );
            out.push_str(&format!(
                "    public static final {} {} = {};\n",
                constant.java_type, constant.name, constant.initializer,
            ));
        }
        out.push_str("}\n");
        Some(out)
    }

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
        docs: &Option<String>,
        name: &str,
        generics: Option<&[(TokenData, Option<TokenData>)]>,
        t: &XenoType,
    ) -> Option<String> {
        let lombok_decorators = collect_lombok_decorators(&t.1, &self.source);
        let metadata = DeclarationMetadata::new(docs, t, &self.source);

        if let Some(fields) = self.evaluated_utility_fields(&t.0) {
            return Some(self.generate_owned_class(
                metadata,
                name,
                &format_generic_params(generics, &self.source),
                &fields,
                &lombok_decorators,
            ));
        }

        if let Some(fields) = self.intersection_fields(&t.0) {
            return Some(self.generate_owned_class(
                metadata,
                name,
                &format_generic_params(generics, &self.source),
                &fields,
                &lombok_decorators,
            ));
        }

        match &t.0 {
            Type::Struct(fields) => Some(self.generate_class(
                metadata,
                name,
                &format_generic_params(generics, &self.source),
                fields,
                &lombok_decorators,
            )),
            Type::Tuple(items) => Some(self.generate_tuple_class(
                metadata,
                name,
                &format_generic_params(generics, &self.source),
                items,
                &lombok_decorators,
            )),
            Type::Enum(variants) => Some(self.generate_enum(metadata, name, variants)),
            Type::Sum(_) => {
                let definition = self.type_hierarchy.get_type(name)?;
                let members = self.interface_sum_members(definition)?;
                Some(self.generate_interface(metadata, name, &members))
            }
            _ => None,
        }
    }

    // ── Class (struct) generation ───────────────────────────────────

    /// Type utilities are operations rather than referenceable types, so a
    /// struct-producing utility is generated as a class of its own.
    fn evaluated_utility_fields(&self, ty: &Type) -> Option<Vec<OwnedField>> {
        let Type::Simple(SimpleType::Identifier(identifier, Some(arguments))) = ty else {
            return None;
        };
        if !is_type_utility(identifier.text(&self.source)) {
            return None;
        }
        let candidate = OwnedType::Named {
            name: identifier.text(&self.source).to_string(),
            arguments: arguments
                .iter()
                .map(|ty| simple_to_owned_type(ty, &self.source))
                .collect(),
        };
        match self.type_hierarchy.evaluate_type(&candidate).ok()? {
            OwnedType::Struct(fields) => Some(fields),
            _ => None,
        }
    }

    /// Java has no intersection types, so an intersection of struct-shaped
    /// members is flattened into one class carrying every member's fields.
    fn intersection_fields(&self, ty: &Type) -> Option<Vec<OwnedField>> {
        if !matches!(ty, Type::Intersection(_)) {
            return None;
        }
        self.type_hierarchy
            .struct_fields(&type_to_owned_type(ty, &self.source))
            .ok()?
    }

    fn generate_owned_class(
        &self,
        metadata: DeclarationMetadata<'_>,
        name: &str,
        generic_params: &str,
        fields: &[OwnedField],
        type_lombok_decorators: &[String],
    ) -> String {
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let class_annotations = self.class_annotations(type_lombok_decorators);

        let mut field_block = String::new();
        for field in fields {
            let java_type = self.owned_type_to_java(&field.ty, &mut imports);
            let java_name = java_member_name(&field.name);
            let literal = match &field.ty {
                OwnedType::Literal(literal) => Some(literal),
                _ => None,
            };

            if let Some(documentation) = field.documentation.as_deref() {
                push_indented_javadoc(&mut field_block, documentation, "    ");
            }
            push_serialized_name(&mut field_block, &mut imports, &field.name, &java_name);
            if !matches!(field.ty, OwnedType::Optional(_)) && literal.is_none() {
                field_block.push_str("    @NonNull\n");
                register_lombok_import(&mut imports, "NonNull");
            }
            if let Some(literal) = literal {
                field_block.push_str(&format!(
                    "    private final {java_type} {} = {};\n",
                    java_name,
                    owned_literal_to_java(literal, &java_type)
                ));
            } else {
                field_block.push_str(&format!("    private {java_type} {java_name};\n"));
            }
        }

        self.render_class(
            metadata,
            name,
            generic_params,
            imports,
            &class_annotations,
            None,
            &field_block,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_class(
        &self,
        metadata: DeclarationMetadata<'_>,
        name: &str,
        generic_params: &str,
        mut imports: BTreeSet<String>,
        class_annotations: &[String],
        marker: Option<&str>,
        field_block: &str,
    ) -> String {
        for annotation in class_annotations {
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

        push_declaration_metadata(&mut out, metadata, "");
        if let Some(marker) = marker {
            out.push_str(&format!("@{marker}\n"));
        }
        for annotation in class_annotations {
            out.push_str(&format!("@{annotation}\n"));
        }
        let interfaces = self.sum_interfaces_of(name);
        let implements = if interfaces.is_empty() {
            String::new()
        } else {
            format!(" implements {}", interfaces.join(", "))
        };
        out.push_str(&format!(
            "public class {name}{generic_params}{implements} {{\n"
        ));
        out.push_str(field_block);
        out.push_str("}\n");
        out
    }

    fn generate_class(
        &self,
        metadata: DeclarationMetadata<'_>,
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
            let java_name = java_member_name(key.text(&self.source));
            let nullable = self.is_nullable(value);
            let literal = required_literal(value);

            if let Some(docs) = docs {
                push_indented_javadoc(
                    &mut field_block,
                    &extract_documentation(docs, &self.source),
                    "    ",
                );
            }
            push_serialized_name(
                &mut field_block,
                &mut imports,
                key.text(&self.source),
                &java_name,
            );
            if !nullable && literal.is_none() {
                field_block.push_str("    @NonNull\n");
                register_lombok_import(&mut imports, "NonNull");
            }
            if let Some(literal) = literal {
                field_block.push_str(&format!(
                    "    private final {java_type} {} = {};\n",
                    java_name,
                    literal_to_java(literal, &java_type)
                ));
            } else {
                field_block.push_str(&format!("    private {java_type} {java_name};\n"));
            }
        }

        self.render_class(
            metadata,
            name,
            generic_params,
            imports,
            &class_annotations,
            None,
            &field_block,
        )
    }

    fn generate_tuple_class(
        &self,
        metadata: DeclarationMetadata<'_>,
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
            if !self.is_nullable(item) && literal.is_none() {
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

        self.render_class(
            metadata,
            name,
            generic_params,
            imports,
            &class_annotations,
            Some("JsonTuple"),
            &field_block,
        )
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

    fn generate_enum(
        &self,
        metadata: DeclarationMetadata<'_>,
        name: &str,
        variants: &[KeyValExpr],
    ) -> String {
        let all_int = !variants.is_empty()
            && variants
                .iter()
                .all(|(_, value, _)| matches!(value, SimpleType::Literal(Literal::Int(_))));

        let mut imports = BTreeSet::new();
        if variants
            .iter()
            .any(|(key, _, _)| java_member_name(key.text(&self.source)) != key.text(&self.source))
        {
            imports.insert("com.google.gson.annotations.SerializedName".to_string());
        }
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

        push_declaration_metadata(&mut out, metadata, "");
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
                    push_indented_javadoc(
                        &mut out,
                        &extract_documentation(docs, &self.source),
                        "    ",
                    );
                }
                let java_name = java_member_name(key.text(&self.source));
                push_serialized_name_line(&mut out, key.text(&self.source), &java_name, "    ");
                out.push_str(&format!("    {java_name}({value}){sep}\n"));
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
                    push_indented_javadoc(
                        &mut out,
                        &extract_documentation(docs, &self.source),
                        "    ",
                    );
                }
                let java_name = java_member_name(key.text(&self.source));
                push_serialized_name_line(&mut out, key.text(&self.source), &java_name, "    ");
                out.push_str(&format!("    {java_name}{sep}\n"));
            }
        }

        out.push_str("}\n");
        out
    }

    /// A sum of class-backed declarations becomes a marker interface that
    /// each member class implements and each member sum extends.
    fn generate_interface(
        &self,
        metadata: DeclarationMetadata<'_>,
        name: &str,
        members: &[DeclarationKey],
    ) -> String {
        let member_links = members
            .iter()
            .map(|(_, member)| format!("{{@link {member}}}"))
            .collect::<Vec<_>>()
            .join(", ");
        let note = format!(
            "Sum of {member_links}; every member implements this marker interface.\n\n\
             <p>Gson cannot deserialize an interface by itself. Register a \
             discriminator-aware {{@code TypeAdapter}} for {{@code {name}}} (TBD) \
             on every Gson instance that reads it.</p>"
        );
        let docs = match metadata.docs {
            Some(docs) => format!("{docs}\n\n{note}"),
            None => note,
        };

        let mut out = generated_java_header(&self.package);
        push_declaration_metadata(
            &mut out,
            DeclarationMetadata {
                docs: Some(&docs),
                ..metadata
            },
            "",
        );
        let parents = self.sum_interfaces_of(name);
        let extends = if parents.is_empty() {
            String::new()
        } else {
            format!(" extends {}", parents.join(", "))
        };
        out.push_str(&format!("public interface {name}{extends} {{\n}}\n"));
        out
    }

    /// A literal set or literal sum becomes an enum whose constants carry the
    /// exact literals, with lookup helpers for validation and selection.
    fn generate_literal_enum(
        &self,
        metadata: DeclarationMetadata<'_>,
        name: &str,
        literal_enum: &LiteralEnum,
        constants: &[(String, bool)],
    ) -> String {
        let value_type = literal_enum.value_type;
        let numeric = literal_enum.is_numeric();
        let mut imports = BTreeSet::new();
        if numeric {
            for import in [
                "com.google.gson.JsonParseException",
                "com.google.gson.TypeAdapter",
                "com.google.gson.annotations.JsonAdapter",
                "com.google.gson.stream.JsonReader",
                "com.google.gson.stream.JsonToken",
                "com.google.gson.stream.JsonWriter",
                "java.io.IOException",
                "java.math.BigDecimal",
            ] {
                imports.insert(import.to_string());
            }
            if value_type == "BigInteger" {
                imports.insert("java.math.BigInteger".to_string());
            }
        } else {
            imports.insert("com.google.gson.annotations.SerializedName".to_string());
        }

        let mut out = generated_java_header(&self.package);
        for import in &imports {
            out.push_str(&format!("import {import};\n"));
        }
        out.push('\n');
        push_declaration_metadata(&mut out, metadata, "");
        if numeric {
            out.push_str(&format!("@JsonAdapter({name}.GsonAdapter.class)\n"));
        }
        out.push_str(&format!("public enum {name} {{\n"));
        if literal_enum.literals.is_empty() {
            out.push_str("    ;\n");
        }
        for (index, (literal, (constant, _))) in
            literal_enum.literals.iter().zip(constants).enumerate()
        {
            let separator = if index + 1 == literal_enum.literals.len() {
                ";"
            } else {
                ","
            };
            if let OwnedLiteral::String(value) = literal {
                out.push_str(&format!(
                    "    @SerializedName({})\n",
                    java_string_literal(value)
                ));
            }
            out.push_str(&format!(
                "    {constant}({}){separator}\n",
                owned_literal_to_java(literal, value_type)
            ));
        }

        let matches = match value_type {
            "String" => "constant.value.equals(value)",
            "BigInteger" | "BigDecimal" => "value != null && constant.value.compareTo(value) == 0",
            _ => "constant.value == value",
        };
        let to_string = if numeric {
            "String.valueOf(value)"
        } else {
            "value"
        };
        out.push_str(&format!(
            r#"
    private final {value_type} value;

    {name}({value_type} value) {{
        this.value = value;
    }}

    /**
     * Returns the literal this constant stands for.
     */
    public {value_type} getValue() {{
        return value;
    }}

    /**
     * Returns the constant whose literal is {{@code value}}.
     *
     * @throws IllegalArgumentException if {{@code value}} is not one of the literals.
     */
    public static {name} fromValue({value_type} value) {{
        {name} constant = lookup(value);
        if (constant == null) {{
            throw new IllegalArgumentException("Unknown {name} value: " + value);
        }}
        return constant;
    }}

    /**
     * Returns whether {{@code value}} is one of the literals.
     */
    public static boolean contains({value_type} value) {{
        return lookup(value) != null;
    }}

    private static {name} lookup({value_type} value) {{
        for ({name} constant : values()) {{
            if ({matches}) {{
                return constant;
            }}
        }}
        return null;
    }}

    @Override
    public String toString() {{
        return {to_string};
    }}
"#
        ));
        if numeric {
            out.push_str(&format!(
                r#"
    /**
     * Reads and writes the numeric literal instead of the constant name.
     */
    public static final class GsonAdapter extends TypeAdapter<{name}> {{
        @Override
        public void write(JsonWriter out, {name} constant) throws IOException {{
            if (constant == null) {{
                out.nullValue();
            }} else {{
                out.value((Number) constant.value);
            }}
        }}

        @Override
        public {name} read(JsonReader in) throws IOException {{
            if (in.peek() == JsonToken.NULL) {{
                in.nextNull();
                return null;
            }}
            String text = in.nextString();
            BigDecimal number;
            try {{
                number = new BigDecimal(text);
            }} catch (NumberFormatException exception) {{
                throw new JsonParseException("Unknown {name} value: " + text, exception);
            }}
            for ({name} constant : values()) {{
                if (new BigDecimal(String.valueOf(constant.value)).compareTo(number) == 0) {{
                    return constant;
                }}
            }}
            throw new JsonParseException("Unknown {name} value: " + text);
        }}
    }}
"#
            ));
        }
        out.push_str("}\n");
        out
    }

    // ── Type mapping ────────────────────────────────────────────────

    fn type_to_java(&self, ty: &SimpleType, imports: &mut BTreeSet<String>) -> String {
        self.owned_type_to_java(&self.resolved_type(ty), imports)
    }

    /// The concrete type a field carries once aliases and utilities are
    /// resolved, e.g. `Required<D>` becomes `D` without its optionality.
    fn resolved_type(&self, ty: &SimpleType) -> OwnedType {
        self.type_hierarchy
            .resolve_for_target(&simple_to_owned_type(ty, &self.source))
    }

    /// Optionality can come from the `?` prefix or from a resolved alias or
    /// utility such as `Optional<T>`.
    fn is_nullable(&self, ty: &SimpleType) -> bool {
        ty.is_optional() || matches!(self.resolved_type(ty), OwnedType::Optional(_))
    }

    fn owned_type_to_java(&self, ty: &OwnedType, imports: &mut BTreeSet<String>) -> String {
        match ty {
            OwnedType::Array(inner) => {
                imports.insert("java.util.List".to_string());
                format!("List<{}>", self.owned_type_to_java(inner, imports))
            }
            OwnedType::Optional(inner) => self.owned_type_to_java(inner, imports),
            OwnedType::Literal(OwnedLiteral::String(_)) => "String".to_string(),
            OwnedType::Literal(OwnedLiteral::Boolean(_)) => "Boolean".to_string(),
            OwnedType::Literal(OwnedLiteral::Integer { representation, .. }) => {
                java_integer_type(*representation, imports).to_string()
            }
            OwnedType::Literal(OwnedLiteral::Float { representation, .. }) => {
                java_float_type(representation.size, imports).to_string()
            }
            OwnedType::Tuple(_) => {
                imports.insert("java.util.List".to_string());
                "List<Object>".to_string()
            }
            OwnedType::Set(set) => {
                imports.insert("java.util.Set".to_string());
                let element = set
                    .element_type
                    .as_deref()
                    .map(|element| self.owned_type_to_java(element, imports))
                    .or_else(|| {
                        // An untyped prefill takes its element type from the first value.
                        set.values
                            .as_ref()
                            .and_then(|values| values.first())
                            .map(|value| {
                                self.owned_type_to_java(&OwnedType::Literal(value.clone()), imports)
                            })
                    })
                    .unwrap_or_else(|| "Object".to_string());
                format!("Set<{element}>")
            }
            OwnedType::Struct(_) | OwnedType::Enum(_) => "Object".to_string(),
            OwnedType::Sum(items) | OwnedType::Intersection(items) => items
                .first()
                .map(|item| self.owned_type_to_java(item, imports))
                .unwrap_or_else(|| "Object".to_string()),
            OwnedType::Generic { name, .. } => name.clone(),
            OwnedType::Named { name, arguments } => self.named_owned_type_to_java(
                name,
                arguments,
                self.type_hierarchy.get_type(name),
                imports,
            ),
            OwnedType::Qualified {
                module_path,
                name,
                arguments,
            } => self.named_owned_type_to_java(
                name,
                arguments,
                self.type_hierarchy
                    .get_type_in(module_path.as_deref(), name),
                imports,
            ),
        }
    }

    fn named_owned_type_to_java(
        &self,
        name: &str,
        arguments: &[OwnedType],
        definition: Option<&TypeDefinition>,
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
        if name == "Dict" {
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
        if let Some(declared) =
            definition.and_then(|definition| self.declared_type_to_java(definition, imports))
        {
            return declared;
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
        "Dict" => {
            imports.insert("java.util.Map".to_string());
            "Map<String, Object>".to_string()
        }
        "NEVER" => "Void".to_string(),
        "any" | "null" => "Object".to_string(),
        _ => return None,
    })
}

fn has_native_java_mapping(name: &str) -> bool {
    native_type_to_java(name, &mut BTreeSet::new()).is_some()
}

/// Collects the decorator names from all `@Lombok(...)` annotations attached to
/// a type or field, e.g. `@Lombok(ToString, EqualsAndHashCode)` → `["ToString",
/// "EqualsAndHashCode"]`.
fn collect_lombok_decorators(annotations: &[Annotation], source: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    for annotation in annotations {
        if annotation.ident.text(source) != "java::Lombok" {
            continue;
        }
        for arg in &annotation.params {
            if let Some(decorator) = decorator_name(arg, source) {
                push_unique(&mut decorators, &decorator);
            }
        }
    }
    decorators
}

/// Renders a single `@Lombok(...)` argument as a Java decorator name.
fn decorator_name(arg: &Expr, source: &str) -> Option<String> {
    match arg {
        Expr::Type(Type::Simple(SimpleType::Identifier(identifier, _))) => identifier
            .text(source)
            .strip_prefix("java::")
            .map(str::to_string),
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

fn required_literal(ty: &SimpleType) -> Option<&Literal> {
    match ty {
        SimpleType::Literal(literal) => Some(literal),
        _ => None,
    }
}

fn owned_literal_to_java(literal: &OwnedLiteral, java_type: &str) -> String {
    match literal {
        OwnedLiteral::String(value) => java_string_literal(value),
        OwnedLiteral::Boolean(value) => value.to_string(),
        OwnedLiteral::Integer { value, .. } if java_type == "Long" || java_type == "long" => {
            format!("{value}L")
        }
        OwnedLiteral::Integer { value, .. } if java_type == "BigInteger" => {
            format!("new BigInteger(\"{value}\")")
        }
        OwnedLiteral::Integer { value, .. } if java_type == "BigDecimal" => {
            format!("new BigDecimal(\"{value}\")")
        }
        OwnedLiteral::Integer { value, .. } => value.to_string(),
        OwnedLiteral::Float { value, .. } if java_type == "Float" || java_type == "float" => {
            format!("{value}F")
        }
        OwnedLiteral::Float { value, .. } if java_type == "BigDecimal" => {
            format!("new BigDecimal(\"{value}\")")
        }
        OwnedLiteral::Float { value, .. } => format!("{value}D"),
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

/// The array element type shared by every literal, or `None` when the
/// literals cannot live in one non-`Object` array.
fn common_java_literal_type(
    literals: &[&Literal],
    imports: &mut BTreeSet<String>,
) -> Option<String> {
    let mut common: Option<String> = None;
    for literal in literals {
        let java_type = match literal {
            Literal::String(_, _) => "String".to_string(),
            Literal::Boolean(_, _) => "Boolean".to_string(),
            Literal::Int(integer) => java_integer_type(integer.representation, imports).to_string(),
            Literal::Float(float) => {
                java_float_type(float.representation.size, imports).to_string()
            }
        };
        common = Some(match common {
            None => java_type,
            Some(current) => widen_java_type(&current, &java_type)?,
        });
    }
    common
}

fn common_java_owned_literal_type(
    literals: &[&OwnedLiteral],
    imports: &mut BTreeSet<String>,
) -> Option<String> {
    let mut common: Option<String> = None;
    for literal in literals {
        let java_type = match literal {
            OwnedLiteral::String(_) => "String".to_string(),
            OwnedLiteral::Boolean(_) => "Boolean".to_string(),
            OwnedLiteral::Integer { representation, .. } => {
                java_integer_type(*representation, imports).to_string()
            }
            OwnedLiteral::Float { representation, .. } => {
                java_float_type(representation.size, imports).to_string()
            }
        };
        common = Some(match common {
            None => java_type,
            Some(current) => widen_java_type(&current, &java_type)?,
        });
    }
    common
}

/// Widens two literal types within the same family, e.g. `Byte` and `Short`
/// become `Short`. Crossing families has no common array type.
fn widen_java_type(left: &str, right: &str) -> Option<String> {
    if left == right {
        return Some(left.to_string());
    }
    const INTEGERS: &[&str] = &["Byte", "Short", "Integer", "Long", "BigInteger"];
    const FLOATS: &[&str] = &["Float", "Double", "BigDecimal"];
    for family in [INTEGERS, FLOATS] {
        if let (Some(left), Some(right)) = (
            family.iter().position(|candidate| *candidate == left),
            family.iter().position(|candidate| *candidate == right),
        ) {
            return Some(family[left.max(right)].to_string());
        }
    }
    None
}

fn common_java_integer_type<'a>(
    literals: impl Iterator<Item = &'a IntLiteral>,
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

fn java_member_name(wire_name: &str) -> String {
    let mut output = String::with_capacity(wire_name.len().max(1) + 1);
    for (index, character) in wire_name.chars().enumerate() {
        if index == 0 && !is_java_identifier_start(character) {
            output.push('_');
        }
        output.push(if is_java_identifier_part(character) {
            character
        } else {
            '_'
        });
    }
    if output.is_empty() {
        output.push('_');
    }
    if is_java_keyword(&output) {
        output.push('_');
    }
    output
}

fn declaration_key(definition: &TypeDefinition) -> DeclarationKey {
    (definition.module_path.clone(), definition.name.clone())
}

/// An UPPER_SNAKE_CASE enum constant name for a literal, e.g.
/// `"Decision Table Testing"` → `DECISION_TABLE_TESTING`, `"camelCase"` →
/// `CAMEL_CASE`, `-1.5` → `VALUE_MINUS_1_5`.
fn java_enum_constant_name(literal: &OwnedLiteral) -> String {
    let text = match literal {
        OwnedLiteral::String(value) => value.clone(),
        OwnedLiteral::Boolean(value) => value.to_string(),
        OwnedLiteral::Integer { value, .. } => format!("value {value}").replace('-', " minus "),
        OwnedLiteral::Float { value, .. } => format!("value {value}").replace('-', " minus "),
    };

    let characters = text.chars().collect::<Vec<_>>();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for (index, &character) in characters.iter().enumerate() {
        if !character.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let previous = index.checked_sub(1).map(|index| characters[index]);
        let next = characters.get(index + 1);
        let starts_word = character.is_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_lowercase()
                    || (previous.is_uppercase() && next.is_some_and(|next| next.is_lowercase()))
            });
        if starts_word && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(character);
    }
    if !word.is_empty() {
        words.push(word);
    }
    let name = words.join("_").to_uppercase();
    java_member_name(if name.is_empty() { "EMPTY" } else { &name })
}

/// Enum constant names for literals in order. Names that collide after
/// normalization get a numeric suffix and are flagged `true`.
fn literal_enum_constant_names(literals: &[OwnedLiteral]) -> Vec<(String, bool)> {
    let mut used = BTreeSet::new();
    literals
        .iter()
        .map(|literal| {
            let base = java_enum_constant_name(literal);
            if used.insert(base.clone()) {
                return (base, false);
            }
            let unique = (2..)
                .map(|suffix| format!("{base}_{suffix}"))
                .find(|candidate| !used.contains(candidate))
                .expect("an unused suffix exists");
            used.insert(unique.clone());
            (unique, true)
        })
        .collect()
}

fn report_enum_constant_collisions(
    name: &TokenData,
    ty: &Type,
    constants: &[(String, bool)],
    errors: &mut Vec<XenoDiagnostic>,
) {
    let literals: Vec<&Literal> = match ty {
        Type::Set(set) => set.values.iter().flatten().collect(),
        Type::Sum(items) => items
            .iter()
            .filter_map(|item| match item {
                SimpleType::Literal(literal) => Some(literal),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    for (index, (constant, collided)) in constants.iter().enumerate() {
        if *collided {
            errors.push(XenoDiagnostic {
                tags: vec![],
                location: literals
                    .get(index)
                    .map_or(*name, |literal| literal.get_last_token()),
                message: format!(
                    "Java enum constant names for this literal collide after normalization; it is generated as '{constant}'."
                ),
                severity: XenoDiagSeverity::Warn,
            });
        }
    }
}

fn is_java_identifier_start(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphabetic()
}

fn is_java_identifier_part(character: char) -> bool {
    is_java_identifier_start(character) || character.is_numeric()
}

fn is_java_keyword(candidate: &str) -> bool {
    matches!(
        candidate,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
            | "_"
            | "true"
            | "false"
            | "null"
            | "var"
            | "yield"
            | "record"
            | "sealed"
            | "permits"
            | "non-sealed"
    )
}

fn report_java_keyword(
    identifier: &TokenData,
    kind: &str,
    errors: &mut Vec<XenoDiagnostic>,
    source: &str,
) {
    if is_java_keyword(identifier.text(source)) {
        errors.push(XenoDiagnostic {
            tags: vec![],
            location: *identifier,
            message: format!(
                "Java {kind} identifier '{}' conflicts with a reserved keyword.",
                identifier.text(source)
            ),
            severity: XenoDiagSeverity::Err,
        });
    }
}

fn push_serialized_name(
    output: &mut String,
    imports: &mut BTreeSet<String>,
    wire_name: &str,
    java_name: &str,
) {
    if wire_name != java_name {
        imports.insert("com.google.gson.annotations.SerializedName".to_string());
        push_serialized_name_line(output, wire_name, java_name, "    ");
    }
}

fn push_serialized_name_line(output: &mut String, wire_name: &str, java_name: &str, indent: &str) {
    if wire_name != java_name {
        output.push_str(&format!(
            "{indent}@SerializedName({})\n",
            java_string_literal(wire_name)
        ));
    }
}

fn validate_java_member_names(
    members: &[KeyValExpr],
    member_kind: &str,
    errors: &mut Vec<XenoDiagnostic>,
    source: &str,
) {
    let mut seen = std::collections::BTreeMap::<String, &str>::new();
    for (key, _, _) in members {
        // Keywords are escaped by `java_member_name`, so `protected` and
        // `protected_` collide here.
        let java_name = java_member_name(key.text(source));
        if let Some(previous) = seen.insert(java_name.clone(), key.text(source)) {
            if previous != key.text(source) {
                errors.push(XenoDiagnostic {
                    tags: vec![],
                    location: *key,
                    message: format!(
                        "Java {member_kind} names '{}' and '{}' both normalize to '{java_name}'.",
                        previous,
                        key.text(source)
                    ),
                    severity: XenoDiagSeverity::Err,
                });
            }
        }
    }
}

fn format_generic_params(
    generics: Option<&[(TokenData, Option<TokenData>)]>,
    source: &str,
) -> String {
    let Some(generics) = generics.filter(|generics| !generics.is_empty()) else {
        return String::new();
    };

    format!(
        "<{}>",
        generics
            .iter()
            .map(|(name, _constraint)| name.text(source))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn push_declaration_metadata(out: &mut String, metadata: DeclarationMetadata<'_>, indent: &str) {
    if metadata.docs.is_some() || metadata.deprecated {
        out.push_str(&format!("{indent}/**\n"));
        if let Some(doc) = metadata.docs {
            for line in doc.lines() {
                out.push_str(&format!("{indent} * {line}\n"));
            }
        }
        if metadata.deprecated {
            let reason = metadata
                .reason
                .unwrap_or("This declaration is deprecated.")
                .replace("*/", "* /");
            let tag = format!("@deprecated {reason}");
            for line in tag.lines() {
                out.push_str(&format!("{indent} * {line}\n"));
            }
        }
        out.push_str(&format!("{indent} */\n"));
    }
    if metadata.deprecated {
        out.push_str(&format!("{indent}@Deprecated\n"));
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
    const FIXTURE_SOURCE: &str = "displayName string /** Name shown to users. */ 256 T HasLength name UserName values NonEmpty u8 box Box UserAge email bool Ty \"#nv\\\"Store\" symbol Alias";
    use xenomorph_common::semantic::{GenericParameterInfo, TypeDeclarationInfo, BUILTIN_TYPES};
    use xenomorph_common::{
        lexer::Lexer,
        parser::{IntegerRepresentation, IntegerSize, Parser},
    };

    fn fixture_generator() -> JavaGenerator {
        let mut g = JavaGenerator::new();
        g.set_source(FIXTURE_SOURCE);
        g
    }
    fn imports() -> BTreeSet<String> {
        BTreeSet::new()
    }

    fn parse(source: &str) -> Vec<Declaration> {
        let tokens = Box::leak(Box::new(
            Lexer::tokenize(Box::leak(source.to_string().into_boxed_str()))
                .expect("Java generator fixture should lex"),
        ));
        let (ast, diagnostics) = Parser::parse(tokens, source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:#?}"
        );
        ast
    }

    #[test]
    fn deprecated_marks_generated_classes_enums_tuples_and_utility_classes() {
        let source = concat!(
            "/** A legacy record. */ type OldRecord = { value: string } @deprecated(\"Use Current instead.\"); ",
            "type OldEnum = enum { Yes: 1, No: 0 } @deprecated; ",
            "type OldTuple = [string, ?bool] @deprecated; ",
            "type OldUtility = Partial<OldRecord> @deprecated; ",
            "type Current = { value: OldRecord };",
        );
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);
        generator.on_before_ast(&ast, &mut Vec::new());

        for name in ["OldRecord", "OldEnum", "OldTuple", "OldUtility"] {
            let (_, output) = generator
                .files
                .iter()
                .find(|(file, _)| file == name)
                .unwrap();
            assert!(output.contains(" * @deprecated"), "{name}: {output}");
            assert!(output.contains("\n@Deprecated\n"), "{name}: {output}");
            assert!(!output.contains("import lombok.Deprecated;"));
        }
        let (_, old) = generator
            .files
            .iter()
            .find(|(name, _)| name == "OldRecord")
            .unwrap();
        assert!(old.contains(" * A legacy record."));
        assert!(old.contains(" * @deprecated Use Current instead."));
        let (_, current) = generator
            .files
            .iter()
            .find(|(name, _)| name == "Current")
            .unwrap();
        assert!(!current.contains("@Deprecated"));
        assert!(!current.contains("@deprecated"));
    }

    #[test]
    fn deprecated_marks_literal_collection_constants_and_keeps_documentation() {
        let source = concat!(
            "/** Use NewColors instead. */ type OldColors = set [\"red\", \"green\"] @deprecated; ",
            "type OldSizes = [1, 300] @deprecated(\"Use NewSizes instead.\"); ",
            "type NewSizes = [2];",
        );
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);
        generator.on_before_ast(&ast, &mut Vec::new());
        let output = generator.constants_file().unwrap();

        assert!(output.contains(" * @deprecated Use NewSizes instead."));
        assert!(output.contains("    @Deprecated\n    public static final Short[] OldSizes"));
        assert_eq!(output.matches("@deprecated").count(), 1);
        assert_eq!(output.matches("@Deprecated").count(), 1);
        assert!(!output.contains("OldColors"));

        let (_, colors) = generator
            .files
            .iter()
            .find(|(name, _)| name == "OldColors")
            .expect("literal sets generate an enum");
        assert!(colors.contains(
            "/**\n * Use NewColors instead.\n * @deprecated This declaration is deprecated.\n */\n@Deprecated\npublic enum OldColors {"
        ));
    }

    #[test]
    fn deprecated_reasons_preserve_lines_without_closing_java_comments() {
        let source = "type Old = enum { Yes: 1 } @deprecated(\"Use NewType.\nKeep */ inside documentation.\");";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);
        generator.on_before_ast(&ast, &mut Vec::new());
        let output = &generator.files[0].1;

        assert!(output.contains(" * @deprecated Use NewType.\n * Keep * / inside documentation.\n */\n@Deprecated\npublic enum Old"));
    }

    #[test]
    fn struct_intersections_generate_a_flattened_class() {
        let source = "type A = { a: string, shared: ?string }; type B = { b: i32, shared: string }; type C = { c: ?bool }; type AB = &A&B; type ABC = &AB&C;";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let class = |name: &str| {
            generator
                .files
                .iter()
                .find(|(file, _)| file == name)
                .map(|(_, class)| class.clone())
                .unwrap_or_else(|| panic!("{name} generates a class"))
        };
        let ab = class("AB");
        assert!(ab.contains("public class AB {"));
        assert!(ab.contains("    @NonNull\n    private String a;"));
        assert!(ab.contains("    @NonNull\n    private Integer b;"));
        assert!(ab.contains("    @NonNull\n    private String shared;"));
        let abc = class("ABC");
        assert!(abc.contains("private String a;"));
        assert!(abc.contains("private Integer b;"));
        assert!(abc.contains("    private Boolean c;"));
    }

    #[test]
    fn struct_utilities_generate_a_resolved_class() {
        let source = "type A = { field1: string, field2: i32, field3: ?bool }; type Keys = |\"field1\"|\"field3\"; type B = Pick<A, Keys>;";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let (_, class) = generator
            .files
            .iter()
            .find(|(name, _)| name == "B")
            .expect("struct utilities generate a class");
        assert!(class.contains("public class B {"));
        assert!(class.contains("    @NonNull\n    private String field1;"));
        assert!(class.contains("    private Boolean field3;"));
        assert!(!class.contains("field2"));
        assert!(!class.contains("Pick"));
    }

    #[test]
    fn homogeneous_literal_tuples_become_xenomorph_array_constants() {
        let source = "type Sizes = [1, 300]; type Holder = { sizes: Sizes };";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let constants = generator
            .constants_file()
            .expect("literal tuples produce constants");
        assert!(constants.contains("public class Xenomorph {"));
        assert!(constants.contains("public static final Short[] Sizes = {1, 300};"));
        assert!(!generator.files.iter().any(|(name, _)| name == "Sizes"));
        assert!(generator.files.iter().any(
            |(name, contents)| name == "Holder" && contents.contains("private Short[] sizes;")
        ));
    }

    fn generated<'a>(generator: &'a JavaGenerator, name: &str) -> &'a str {
        generator
            .files
            .iter()
            .find(|(file, _)| file == name)
            .map(|(_, contents)| contents.as_str())
            .unwrap_or_else(|| panic!("{name} generates a file"))
    }

    #[test]
    fn string_literal_sets_and_sums_become_enums_used_as_field_types() {
        let source = concat!(
            "type Kind = set [\"Decision Table Testing\", \"camelCase\", \"a-b\"]; ",
            "type Mode = |\"on\"|\"off\"; ",
            "type S = { kind: Kind, kinds: Kind[], maybe: ?Kind, mode: Mode };",
        );
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generator.constants_file().is_none());
        let kind = generated(&generator, "Kind");
        assert!(kind.contains("import com.google.gson.annotations.SerializedName;"));
        assert!(kind.contains("public enum Kind {"));
        assert!(kind.contains(
            "    @SerializedName(\"Decision Table Testing\")\n    DECISION_TABLE_TESTING(\"Decision Table Testing\"),\n"
        ));
        assert!(
            kind.contains("    @SerializedName(\"camelCase\")\n    CAMEL_CASE(\"camelCase\"),\n")
        );
        assert!(kind.contains("    @SerializedName(\"a-b\")\n    A_B(\"a-b\");\n"));
        assert!(kind.contains("    private final String value;"));
        assert!(kind.contains("    public String getValue() {"));
        assert!(kind.contains("    public static Kind fromValue(String value) {"));
        assert!(
            kind.contains("throw new IllegalArgumentException(\"Unknown Kind value: \" + value);")
        );
        assert!(kind.contains("    public static boolean contains(String value) {"));
        assert!(kind.contains("if (constant.value.equals(value)) {"));
        assert!(kind.contains("    public String toString() {\n        return value;"));
        assert!(generated(&generator, "Mode").contains("    ON(\"on\"),\n"));

        let holder = generated(&generator, "S");
        assert!(holder.contains("    @NonNull\n    private Kind kind;"));
        assert!(holder.contains("    @NonNull\n    private List<Kind> kinds;"));
        assert!(holder.contains("    private Kind maybe;"));
        assert!(!holder.contains("@NonNull\n    private Kind maybe;"));
        assert!(holder.contains("    private Mode mode;"));
    }

    #[test]
    fn sums_of_literal_sets_merge_into_one_enum() {
        let source = concat!(
            "type Technique = set [\"BVA\", \"EP\"]; ",
            "type Placeholder = set [\"placeholder\"]; ",
            "type Stored = |Technique|Placeholder|\"EP\"; ",
            "type Holder = { technique: Stored };",
        );
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let stored = generated(&generator, "Stored");
        assert!(stored.contains("public enum Stored {"));
        assert!(stored.contains("    BVA(\"BVA\"),\n"));
        assert!(stored.contains("    EP(\"EP\"),\n"));
        assert!(stored.contains("    PLACEHOLDER(\"placeholder\");\n"));
        assert_eq!(stored.matches("EP(\"EP\")").count(), 1);
        assert!(generated(&generator, "Holder").contains("    private Stored technique;"));
    }

    #[test]
    fn numeric_literal_sets_become_enums_with_numeric_gson_values() {
        let source = "type Small = set<u8> [1, 2]; type Ratio = set [0.5, 1.5]; type Holder = { small: Small };";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let small = generated(&generator, "Small");
        assert!(small.contains("@JsonAdapter(Small.GsonAdapter.class)\npublic enum Small {"));
        assert!(small.contains("    VALUE_1(1),\n    VALUE_2(2);\n"));
        assert!(small.contains("    private final int value;"));
        assert!(small.contains("    public static Small fromValue(int value) {"));
        assert!(small.contains("if (constant.value == value) {"));
        assert!(small.contains("out.value((Number) constant.value);"));
        assert!(!small.contains("SerializedName"));
        let ratio = generated(&generator, "Ratio");
        assert!(ratio.contains("    VALUE_0_5(0.5F),\n    VALUE_1_5(1.5F);\n"));
        assert!(ratio.contains("    private final float value;"));
        assert!(generated(&generator, "Holder").contains("private Small small;"));
    }

    #[test]
    fn colliding_enum_constant_names_get_a_suffix_and_a_warning() {
        let source = "type Kind = set [\"a b\", \"a_b\"];";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);
        let mut errors = Vec::new();

        generator.on_before_ast(&ast, &mut errors);

        let kind = generated(&generator, "Kind");
        assert!(kind.contains("    A_B(\"a b\"),\n"));
        assert!(kind.contains("    A_B_2(\"a_b\");\n"));
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0].severity, XenoDiagSeverity::Warn));
        assert_eq!(errors[0].location.text(source), "\"a_b\"");
    }

    #[test]
    fn struct_sums_become_marker_interfaces_implemented_by_members() {
        let source = concat!(
            "/** A message. */ type Msg = |SaveMsg|ProblemMsg @deprecated; ",
            "type Any = |Msg|Other; ",
            "type SaveMsg = { path: string }; ",
            "type A = { a: string }; type B = { b: i32 }; type ProblemMsg = &A&B; ",
            "type Other = Pick<A, OtherKeys>; type OtherKeys = |\"a\"; ",
            "type Holder = { msg: Msg, msgs: Msg[], maybe: ?Msg, any: Any };",
        );
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let msg = generated(&generator, "Msg");
        assert!(msg.contains(" * A message.\n"));
        assert!(msg.contains("{@link SaveMsg}, {@link ProblemMsg}"));
        assert!(msg.contains("discriminator-aware {@code TypeAdapter}"));
        assert!(msg.contains("@Deprecated\npublic interface Msg extends Any {\n}\n"));
        assert!(generated(&generator, "Any").contains("public interface Any {\n}\n"));
        assert!(generated(&generator, "SaveMsg").contains("public class SaveMsg implements Msg {"));
        assert!(generated(&generator, "ProblemMsg")
            .contains("public class ProblemMsg implements Msg {"));
        assert!(generated(&generator, "Other").contains("public class Other implements Any {"));
        assert!(generated(&generator, "A").contains("public class A {"));

        let holder = generated(&generator, "Holder");
        assert!(holder.contains("    @NonNull\n    private Msg msg;"));
        assert!(holder.contains("    @NonNull\n    private List<Msg> msgs;"));
        assert!(holder.contains("    private Msg maybe;"));
        assert!(holder.contains("    @NonNull\n    private Any any;"));
    }

    #[test]
    fn sums_in_other_modules_add_marker_interfaces() {
        let messages = "type SaveMsg = { path: string };";
        let sums = "type Msg = |SaveMsg|Local; type Local = { x: string };";
        let mut generator = generator_with_builtins();
        for (module, source) in [("messages", messages), ("sums", sums)] {
            for declaration in parse(source) {
                if let Declaration::Type {
                    name, generics, ty, ..
                } = declaration
                {
                    generator.type_hierarchy.insert_declaration(
                        module,
                        name.text(source),
                        TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
                    );
                }
            }
        }
        generator
            .type_hierarchy
            .register_module("sums", vec!["messages".to_string()]);
        generator.type_hierarchy.set_current_module("messages");
        generator.set_source(messages);
        let ast = parse(messages);

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generated(&generator, "SaveMsg").contains("public class SaveMsg implements Msg {"));
    }

    #[test]
    fn sums_without_class_members_keep_no_interface() {
        let source = "type Mixed = |string|i32; type Holder = { mixed: Mixed };";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(!generator.files.iter().any(|(name, _)| name == "Mixed"));
        assert!(generated(&generator, "Holder").contains("private Object mixed;"));
    }

    #[test]
    fn heterogeneous_literal_tuples_keep_their_tuple_class() {
        let source = "type Mixed = [\"a\", 1];";
        let ast = parse(source);
        let mut generator = generator_with_builtins();
        generator.set_source(source);

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generator.constants_file().is_none());
        assert!(generator.files.iter().any(|(name, contents)| {
            name == "Mixed"
                && contents.contains("@JsonTuple")
                && contents.contains("private final String item0 = \"a\";")
        }));
    }

    #[test]
    fn sets_without_a_prefill_map_to_java_sets() {
        let source = "type Tags = set<string>; type Holder = { tags: Tags };";
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        assert!(generator.constants_file().is_none());
        assert!(generator.files.iter().any(|(name, contents)| {
            name == "Holder"
                && contents.contains("import java.util.Set;")
                && contents.contains("private Set<String> tags;")
        }));
    }

    #[test]
    fn test_type_and_field_docs_are_preserved() {
        let key = xenomorph_common::TokenData::new(0, 11);
        let field_type = xenomorph_common::TokenData::new(12, 18);
        let field_docs = xenomorph_common::TokenData::new(19, 46);
        let ty = (
            Type::Struct(vec![(
                key,
                SimpleType::Identifier(field_type, None),
                Some(field_docs),
            )]),
            vec![],
        );

        let output = fixture_generator()
            .generate_type_decl(
                &Some("A user-facing profile.".to_string()),
                "Profile",
                None,
                &ty,
            )
            .expect("structs generate Java classes");

        assert!(output.contains(" * A user-facing profile."));
        assert!(output.contains("     * Name shown to users."));
        assert!(output.contains("    private String displayName;"));
    }

    #[test]
    fn quoted_wire_names_use_safe_java_fields_and_gson_names() {
        let source = r#"type Payload = { "ecu.test": string };"#;
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let (_, class) = generator
            .files
            .iter()
            .find(|(name, _)| name == "Payload")
            .expect("struct generates a Java class");
        assert!(class.contains("import com.google.gson.annotations.SerializedName;"));
        assert!(class.contains("@SerializedName(\"ecu.test\")"));
        assert!(class.contains("private String ecu_test;"));
    }

    #[test]
    fn quoted_enum_names_use_safe_java_constants_and_gson_names() {
        let source = r#"type Mode = enum { "ecu.test": 1, normal: 2 };"#;
        let ast = parse(source);
        let mut generator = generator_with_declarations(&ast, source);

        generator.on_before_ast(&ast, &mut Vec::new());

        let (_, generated_enum) = generator
            .files
            .iter()
            .find(|(name, _)| name == "Mode")
            .expect("enum generates a Java enum");
        assert!(generated_enum.contains("@SerializedName(\"ecu.test\")"));
        assert!(generated_enum.contains("ecu_test(1),"));
    }

    #[test]
    fn java_name_normalization_collisions_are_rejected() {
        let source = r#"type Payload = { "ecu.test": string, ecu_test: string };"#;
        let ast = parse(source);
        let Declaration::Type {
            ty: (Type::Struct(fields), _),
            ..
        } = &ast[0]
        else {
            panic!("expected struct declaration");
        };
        let mut errors = Vec::new();

        {
            let mut generator = fixture_generator();
            generator.set_source(source);
            generator.on_before_struct(fields, &mut errors);
        }

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("both normalize to 'ecu_test'"));
    }

    #[test]
    fn java_keyword_member_names_are_escaped_with_gson_names() {
        let source = r#"type Payload = { "protected": string, class: i32 }; type Mode = enum { "public": 1 };"#;
        let ast = parse(source);
        let Declaration::Type {
            ty: (Type::Struct(fields), _),
            ..
        } = &ast[0]
        else {
            panic!("expected struct declaration");
        };
        let mut generator = generator_with_declarations(&ast, source);
        let mut errors = Vec::new();

        generator.on_before_struct(fields, &mut errors);
        generator.on_before_ast(&ast, &mut errors);

        assert!(errors.is_empty(), "{errors:#?}");
        let class = generated(&generator, "Payload");
        assert!(class.contains(
            "    @SerializedName(\"protected\")\n    @NonNull\n    private String protected_;"
        ));
        assert!(class
            .contains("    @SerializedName(\"class\")\n    @NonNull\n    private Integer class_;"));
        assert!(generated(&generator, "Mode")
            .contains("    @SerializedName(\"public\")\n    public_(1);"));
    }

    #[test]
    fn escaped_keyword_member_names_collide_with_their_escaped_spelling() {
        let source = r#"type Payload = { "protected": string, protected_: string };"#;
        let ast = parse(source);
        let Declaration::Type {
            ty: (Type::Struct(fields), _),
            ..
        } = &ast[0]
        else {
            panic!("expected struct declaration");
        };
        let mut errors = Vec::new();

        {
            let mut generator = fixture_generator();
            generator.set_source(source);
            generator.on_before_struct(fields, &mut errors);
        }

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("both normalize to 'protected_'"));
    }

    #[test]
    fn java_keywords_are_rejected_for_native_declaration_names() {
        let source = "type class = string;";
        let ast = parse(source);
        let mut errors = Vec::new();

        {
            let mut generator = fixture_generator();
            generator.set_source(source);
            generator.on_before_decl(&ast[0], &mut errors);
        }

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "Java type identifier 'class' conflicts with a reserved keyword."
        );
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
        assert_eq!(
            native_type_to_java("NEVER", &mut i).as_deref(),
            Some("Void")
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
            token,
            cast: None,
        }));
        let mut generator = JavaGenerator::new();
        generator.set_source(FIXTURE_SOURCE);
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
                body: OwnedType::named("T"),
                transparent_alias: true,
            },
        );

        let type_parameter = token("T");
        let constraint = token("HasLength");
        let field_name = token("name");
        let alias_name = token("UserName");
        let argument = token("T");
        let generics = [(type_parameter, Some(constraint))];
        let ty = (
            Type::Struct(vec![(
                field_name,
                SimpleType::Identifier(
                    alias_name,
                    Some(vec![SimpleType::Identifier(argument, None)]),
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
                body: OwnedType::named("T"),
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
                body: OwnedType::Array(Box::new(OwnedType::Named {
                    name: "Positive".to_string(),
                    arguments: vec![OwnedType::named("T")],
                })),
                transparent_alias: true,
            },
        );

        let field_name = token("values");
        let alias_name = token("NonEmpty");
        let argument = token("u8");
        let ty = (
            Type::Struct(vec![(
                field_name,
                SimpleType::Identifier(
                    alias_name,
                    Some(vec![SimpleType::Identifier(argument, None)]),
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
                parents: vec![OwnedType::named("Dict")],
                body: OwnedType::named("Dict"),
                transparent_alias: false,
            },
        );

        let field_name = token("box");
        let box_name = token("Box");
        let argument = token("string");
        let ty = (
            Type::Struct(vec![(
                field_name,
                SimpleType::Identifier(
                    box_name,
                    Some(vec![SimpleType::Identifier(argument, None)]),
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
                body: OwnedType::named("T"),
                transparent_alias: true,
            },
        );
        generator.type_hierarchy.insert_declaration(
            "test",
            "UserAge",
            TypeDeclarationInfo {
                generic_params: vec![],
                parents: vec![OwnedType::named("u8")],
                body: OwnedType::named("u8"),
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
                SimpleType::Identifier(user_name, Some(vec![SimpleType::Identifier(string, None)])),
                SimpleType::Identifier(user_age, None),
                SimpleType::Identifier(email, None),
                SimpleType::Identifier(bool_type, None),
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
        let source =
            "type TestTuple = [string, u8, email, bool]; type UserType = { meta: TestTuple };";
        let ast = parse(source);
        let mut generator = generator_with_builtins();
        generator.set_source(source);

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
        let source = "type UserType = { name: string };";
        let ast = parse(source);
        let mut generator = generator_with_builtins();
        generator.set_source(source);

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
                field_name,
                SimpleType::Literal(Literal::String("#nv\"Store".to_string(), literal_token)),
                None,
            )]),
            vec![],
        );

        let output = fixture_generator()
            .generate_type_decl(&None, "Store", None, &ty)
            .expect("structs generate Java classes");

        assert!(output.contains("    private final String Ty = \"#nv\\\"Store\";"));
        assert!(!output.contains("@NonNull"));
    }

    #[test]
    fn test_parameterized_dict_maps_directly_to_java_map() {
        let generator = generator_with_builtins();
        let dict = OwnedType::Named {
            name: "Dict".to_string(),
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
        let ty = SimpleType::Identifier(symbol, None);
        let mut generator = JavaGenerator::new();
        generator.set_source(FIXTURE_SOURCE);
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
        let ty = SimpleType::Identifier(alias, None);
        let mut generator = generator_with_builtins();
        generator.type_hierarchy.insert_semantic_type(&SYMBOL);
        generator.type_hierarchy.insert_declaration(
            "test",
            "Alias",
            TypeDeclarationInfo {
                generic_params: Vec::new(),
                parents: vec![OwnedType::named("symbol")],
                body: OwnedType::named("symbol"),
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
        generator.set_source(FIXTURE_SOURCE);
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
    fn qualified_lombok_annotations_emit_java_decorator_names() {
        let source = "type Example = { value: string } @java::Lombok(java::Value, java::Builder);";
        let ast = parse(source);
        let Declaration::Type { ty, .. } = &ast[0] else {
            panic!("type declaration")
        };
        assert_eq!(
            collect_lombok_decorators(&ty.1, source),
            vec!["Value", "Builder"]
        );
        let mut generator = generator_with_declarations(&ast, source);
        generator.on_before_ast(&ast, &mut Vec::new());
        let (_, class) = generator
            .files
            .iter()
            .find(|(name, _)| name == "Example")
            .unwrap();
        assert!(
            class.contains("@Value") && class.contains("@Builder"),
            "{class}"
        );
        assert!(!class.contains("java::"));
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
            .filter(|semantic_type| semantic_type.name != "NEVER")
            .all(|semantic_type| !semantic_type.implements(&LOMBOK_TRAIT)));
        assert!(xenomorph_common::semantic::NEVER.implements(&LOMBOK_TRAIT));
    }

    fn token(value: &str) -> TokenData {
        let start = FIXTURE_SOURCE.find(value).unwrap();
        TokenData::new(start, start + value.len())
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
        generator.set_source(FIXTURE_SOURCE);
        generator.type_hierarchy.set_current_module("test");
        generator.type_hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            generator.type_hierarchy.insert_semantic_type(semantic_type);
        }
        generator
    }

    fn generator_with_declarations(ast: &[Declaration], source: &str) -> JavaGenerator {
        let mut generator = generator_with_builtins();
        generator.set_source(source);
        for declaration in ast {
            if let Declaration::Type {
                name, generics, ty, ..
            } = declaration
            {
                generator.type_hierarchy.insert_declaration(
                    "test",
                    name.text(source),
                    TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
                );
            }
        }
        generator
    }
}
