// use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::{
    config::PluginConfigs,
    module::ModuleData,
    parser::{Annotation, Declaration, Expr, KeyValExpr, SetType, SimpleType, Type, XenoType},
    plugins::LoadedPlugin,
    semantic::{
        annotation_validator::AnnotationValidator, if_validator::IfChainValidator,
        intersection_validator::IntersectionValidator, name_validator::NameValidator,
        GenericParameterInfo, MetadataGraph, OwnedAnnotation, OwnedType, TypeHierarchy,
        BUILTIN_ANNOTATIONS, BUILTIN_TRAITS, BUILTIN_TYPES,
    },
    TokenData, XenoDiagnostic,
};

/// Scope information built by the analyzer and passed to listeners.
/// Contains all known types/annotations with their provenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScopeInfo {
    /// Module path of the current module.
    pub module_path: String,
    /// Absolute filesystem path of the current module.
    #[serde(with = "crate::plugins::protocol::path")]
    pub abs_path: PathBuf,
    /// Types declared in this module.
    pub own_types: Vec<String>,
    /// Types imported from other modules, keyed by module path.
    pub imported_types: HashMap<String, Vec<String>>,
    /// Built-in type names (no module provenance).
    pub builtin_types: HashSet<String>,
    /// All known annotation names (builtins + plugins, flat set).
    pub known_annotations: HashSet<String>,
    /// Canonical runtime hierarchy for builtin, plugin, local, and imported types.
    pub type_hierarchy: TypeHierarchy,
    /// Typed definitions for built-in and plugin annotations.
    #[serde(with = "super::metadata::unique_named_map")]
    pub annotations: HashMap<String, OwnedAnnotation>,
}

impl ScopeInfo {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_metadata()
    }

    /// Checks decoded metadata and scope indexes before a plugin uses them.
    /// Unresolved source declaration bodies remain host semantic diagnostics.
    pub fn validate_metadata(&self) -> Result<(), String> {
        self.type_hierarchy.validate_metadata()?;
        match self.type_hierarchy.current_module() {
            Some(module) if module != self.module_path => {
                return Err("Scope module does not match hierarchy lookup context".into());
            }
            None if !self.own_types.is_empty() || !self.imported_types.is_empty() => {
                return Err("Source scope has no hierarchy lookup context".into());
            }
            _ => {}
        }
        for (key, annotation) in &self.annotations {
            super::metadata::validate_identity(key, &annotation.name)?;
            super::metadata::validate_parameter_names(
                annotation
                    .params
                    .iter()
                    .map(|parameter| parameter.name.as_str()),
                &annotation.name,
            )?;
            for parameter in &annotation.params {
                super::metadata::validate_constraint(
                    &parameter.constraint,
                    |name| self.type_hierarchy.get_type_in(None, name).is_some(),
                    |name| self.type_hierarchy.has_trait(name),
                )?;
            }
            if annotation.variadic && annotation.explicit_parameters().is_empty() {
                return Err(format!(
                    "Variadic annotation '{}' has no explicit parameter to repeat",
                    annotation.name
                ));
            }
        }
        if self.known_annotations.len() != self.annotations.len()
            || self
                .known_annotations
                .iter()
                .any(|name| !self.annotations.contains_key(name))
        {
            return Err("Annotation scope index does not match metadata definitions".into());
        }
        for name in &self.builtin_types {
            if self.type_hierarchy.get_type_in(None, name).is_none() {
                return Err(format!("Scope references missing global type '{name}'"));
            }
        }
        let check_types = |module: &str, names: &[String]| -> Result<(), String> {
            let mut unique = HashSet::new();
            for name in names {
                if !unique.insert(name)
                    || self
                        .type_hierarchy
                        .get_type_in(Some(module), name)
                        .is_none()
                {
                    return Err(format!(
                        "Scope references repeated or missing type '{module}::{name}'"
                    ));
                }
            }
            Ok(())
        };
        check_types(&self.module_path, &self.own_types)?;
        for (module, names) in &self.imported_types {
            check_types(module, names)?;
        }
        Ok(())
    }

    /// Returns true if `name` is a known type (own, imported, or builtin).
    pub fn has_type(&self, name: &str) -> bool {
        self.type_hierarchy.has_type(name)
            && (self.builtin_types.contains(name)
                || self.own_types.iter().any(|n| n == name)
                || self
                    .imported_types
                    .values()
                    .any(|names| names.iter().any(|n| n == name)))
    }

    /// Returns true for either a visible type or a globally registered trait.
    pub fn has_constraint(&self, name: &str) -> bool {
        self.has_type(name) || self.type_hierarchy.has_trait(name)
    }

    pub fn is_static_type(&self, name: &str) -> bool {
        self.type_hierarchy
            .get_type(name)
            .is_some_and(|definition| definition.module_path.is_none())
    }

    /// Returns true if `name` is a known annotation.
    pub fn has_annotation(&self, name: &str) -> bool {
        self.known_annotations.contains(name)
    }

    pub fn find_annotation(&self, name: &str) -> Option<&OwnedAnnotation> {
        self.annotations.get(name)
    }

    pub fn insert_metadata(&mut self, metadata: &MetadataGraph) {
        self.type_hierarchy.insert_metadata(metadata);
        self.builtin_types.extend(metadata.types.keys().cloned());
        self.known_annotations
            .extend(metadata.annotations.keys().cloned());
        self.annotations.extend(metadata.annotations.clone());
    }

    pub fn implements_trait(&self, candidate: &OwnedType, required: &str) -> bool {
        self.type_hierarchy.implements_trait(candidate, required)
    }

    pub fn descends_from_semantic_type(&self, candidate: &OwnedType, target: &str) -> bool {
        self.type_hierarchy
            .descends_from_semantic_type(candidate, target)
    }

    pub fn generic_parameters(&self, name: &str) -> Option<Vec<GenericParameterInfo>> {
        self.type_hierarchy.generic_parameters(name)
    }

    pub fn type_implements_trait(
        &self,
        candidate: &OwnedType,
        required: &crate::semantic::XenoTrait,
    ) -> bool {
        self.type_hierarchy
            .type_implements_trait(candidate, required)
    }

    pub fn is_type_compatible(&self, candidate: &OwnedType, target: &str) -> bool {
        self.type_hierarchy.is_type_compatible(candidate, target)
    }

    pub fn descends_from_static_type(
        &self,
        candidate: &OwnedType,
        target: &crate::semantic::XenoType,
    ) -> bool {
        self.type_hierarchy
            .descends_from_static_type(candidate, target)
    }

    pub fn satisfies_constraint(
        &self,
        candidate: &OwnedType,
        constraint: &str,
        constraint_scope: Option<&str>,
    ) -> bool {
        self.type_hierarchy
            .satisfies_constraint(candidate, constraint, constraint_scope)
    }

    /// Returns the module path that provides a given type name, if it's imported.
    pub fn provider_of(&self, name: &str) -> Option<&str> {
        for (module_path, names) in &self.imported_types {
            if names.iter().any(|n| n == name) {
                return Some(module_path);
            }
        }
        None
    }
}

/// Trait for AST walk event listeners. All methods have default no-op
/// implementations so listeners only need to override the events they
/// care about.
#[allow(unused_variables)]
pub trait AnalyzerListener<'src> {
    /// Called before the AST walk begins, with full scope information.
    fn on_before_module(&mut self, scope: &ScopeInfo) {}
    /// Called after the full AST walk completes, with scope information.
    fn on_after_module(&mut self, scope: &ScopeInfo) {}

    fn on_before_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoDiagnostic<'src>>) {
    }
    fn on_after_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_before_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {
    }
    fn on_after_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    // fn on_before_custom(
    //     &mut self,
    //     plugin_id: &str,
    //     decl_id: &str,
    //     name: &Option<&TokenData<'src>>,
    //     docs: &Option<&'src str>,
    //     value: &Box<dyn Any>,
    //     errors: &mut Vec<XenoError<'src>>,
    // ) {
    // }
    // fn on_after_custom(&mut self, errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_type(&mut self, exprs: &XenoType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}
    fn on_after_type(&mut self, exprs: &XenoType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_before_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}
    fn on_after_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_before_struct(
        &mut self,
        fields: &[KeyValExpr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
    fn on_after_struct(
        &mut self,
        fields: &[KeyValExpr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }

    fn on_before_field(
        &mut self,
        key: &TokenData<'src>,
        value: &SimpleType<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
    fn on_after_field(
        &mut self,
        key: &TokenData<'src>,
        value: &SimpleType<'src>,
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }

    fn on_before_enum(
        &mut self,
        variants: &[KeyValExpr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
    fn on_after_enum(
        &mut self,
        variants: &[KeyValExpr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }

    fn on_array(&mut self, inner: &TokenData<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}
    fn on_after_array(&mut self, inner: &TokenData<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_before_list(
        &mut self,
        inner: &[SimpleType<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
    fn on_after_list(
        &mut self,
        inner: &[SimpleType<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }

    fn on_before_set(&mut self, set: &SetType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}
    fn on_after_set(&mut self, set: &SetType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_simple_type(&mut self, ty: &SimpleType<'src>, errors: &mut Vec<XenoDiagnostic<'src>>) {}

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
    fn on_after_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &[Expr<'src>],
        errors: &mut Vec<XenoDiagnostic<'src>>,
    ) {
    }
}

/// Analyzer that owns metadata used by every module analysis.
/// Created once during registry construction, reused for every module analysis.
pub struct Analyzer {
    /// Whether to use generation mode (true) or analyzer/LSP mode (false).
    pub generation_mode: bool,
    metadata: MetadataGraph,
}

impl Analyzer {
    pub fn new(generation_mode: bool, plugins: &[LoadedPlugin]) -> Self {
        let mut analyzer = Self::with_metadata(generation_mode, MetadataGraph::default());
        for plugin in plugins {
            analyzer.add_metadata(&plugin.description.metadata);
        }
        analyzer
    }

    /// Constructs an analyzer without native plugin listener factories.
    pub fn with_metadata(generation_mode: bool, metadata: MetadataGraph) -> Self {
        Self {
            generation_mode,
            metadata,
        }
    }

    pub fn add_metadata(&mut self, metadata: &MetadataGraph) {
        self.metadata.types.extend(metadata.types.clone());
        self.metadata.traits.extend(metadata.traits.clone());
        self.metadata
            .annotations
            .extend(metadata.annotations.clone());
    }

    /// Analyze a module's AST with full scope from the cache.
    /// Builds known_types and known_annotations from builtins, plugins, own
    /// declarations, and imported module declarations.
    pub fn run<'src>(
        &self,
        ast: &[Declaration<'src>],
        module_data: &ModuleData,
        imports: &[String],
        cache: &HashMap<String, ModuleData>,
        plugins: &[LoadedPlugin],
        _plugin_configs: &PluginConfigs,
    ) -> Vec<XenoDiagnostic<'src>> {
        // ── Pass 1: collect every type and trait before validation ──
        let mut builtin_types: HashSet<String> = HashSet::new();
        let mut known_annotations: HashSet<String> = HashSet::new();
        let mut type_hierarchy = TypeHierarchy::default();
        let mut annotations = HashMap::new();

        let builtin_metadata =
            MetadataGraph::from_all_descriptors(BUILTIN_TYPES, BUILTIN_TRAITS, BUILTIN_ANNOTATIONS);
        for metadata in std::iter::once(&builtin_metadata)
            .chain(std::iter::once(&self.metadata))
            .chain(plugins.iter().map(|plugin| &plugin.description.metadata))
        {
            type_hierarchy.insert_metadata(metadata);
            builtin_types.extend(metadata.types.keys().cloned());
            known_annotations.extend(metadata.annotations.keys().cloned());
            annotations.extend(metadata.annotations.clone());
        }

        // Module declarations were indexed when each ModuleData was created.
        // Qualified keys let the hierarchy retain every cached declaration,
        // including duplicate simple names. Per-module import scopes resolve
        // each declaration's own parent graph without exposing transitive or
        // unrelated names through ScopeInfo::has_type.
        let module_path_str = module_data.borrow_module_path().to_string();
        type_hierarchy.set_current_module(module_path_str.clone());
        for (module_path, module) in cache {
            type_hierarchy.register_module(module_path.clone(), module.borrow_imports().to_vec());
            for declaration in module.borrow_declarations().values() {
                type_hierarchy.insert_declaration(
                    module_path.clone(),
                    declaration.name.clone(),
                    declaration.semantic.clone(),
                );
            }
        }

        let own_types: Vec<String> = module_data
            .borrow_declarations()
            .keys()
            .map(|k| k.to_string())
            .collect();

        // Imported declarations grouped by module (skip self-imports)
        let mut imported_types: HashMap<String, Vec<String>> = HashMap::new();
        for import in imports {
            if import == &module_path_str {
                continue; // skip self-import
            }
            if let Some(m) = cache.get(import) {
                let names: Vec<String> = m
                    .borrow_declarations()
                    .keys()
                    .map(|k| k.to_string())
                    .collect();
                imported_types.insert(import.clone(), names);
            }
        }

        let scope = ScopeInfo {
            module_path: module_path_str,
            abs_path: module_data.borrow_abs_path().to_path_buf(),
            own_types,
            imported_types,
            builtin_types,
            known_annotations,
            type_hierarchy,
            annotations,
        };

        // ── Pass 2: validate and generate with the completed hierarchy ──
        let mut listeners: Vec<Box<dyn AnalyzerListener<'src>>> = vec![
            Box::new(NameValidator::new(&scope)),
            Box::new(IntersectionValidator::new(&scope)),
            Box::new(AnnotationValidator::new(&scope)),
            Box::new(IfChainValidator::new()),
        ];

        // Notify listeners of module context + scope
        for l in listeners.iter_mut() {
            l.on_before_module(&scope);
        }

        // Walk the AST
        let mut errors = Vec::new();

        // Check for self-imports
        for decl in ast {
            if let Declaration::Import { path, location } = decl {
                let import_path = path.join("/");
                if import_path == scope.module_path {
                    errors.push(XenoDiagnostic {
                        severity: crate::XenoDiagSeverity::Err,
                        location: (*location).clone(),
                        message: format!("Module '{}' cannot import itself", import_path),
                    });
                }
            }
        }

        walk_ast(&mut listeners, ast, &mut errors);

        if !self.generation_mode || can_generate(&errors) {
            for plugin in plugins {
                if self.generation_mode && !can_generate(&errors) {
                    break;
                }
                match plugin.run(self.generation_mode, ast, &scope) {
                    Ok(diagnostics) => errors.extend(diagnostics),
                    Err(error) => errors.push(XenoDiagnostic {
                        location: first_source_location(ast),
                        message: format!("Plugin '{}' failed: {error}", plugin.description.name),
                        severity: crate::XenoDiagSeverity::Err,
                    }),
                }
            }
        }

        // Generators write their files when the module is finalized. Keep the
        // log level presentational: warnings and infos do not block this step,
        // but errors from any phase do.
        if !self.generation_mode || can_generate(&errors) {
            for l in listeners.iter_mut() {
                l.on_after_module(&scope);
            }
        }

        errors
    }
}

fn first_source_location<'src>(ast: &[Declaration<'src>]) -> TokenData<'src> {
    ast.iter()
        .find_map(|declaration| match declaration {
            Declaration::Import { location, .. } => Some((*location).clone()),
            Declaration::Type { from, .. } => Some((*from).clone()),
            Declaration::Custom { name, .. } => name.map(Clone::clone),
        })
        .unwrap_or(TokenData { v: "", l: 0, c: 0 })
}

fn can_generate(diagnostics: &[XenoDiagnostic<'_>]) -> bool {
    !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::XenoDiagSeverity::Err)
}

// ── Walk functions (free functions to avoid &mut self borrow issues) ─

type Listeners<'src> = [Box<dyn AnalyzerListener<'src>>];

/// Runs one plugin listener without host semantic validators.
pub fn run_plugin_listener<'src>(
    listener: Box<dyn AnalyzerListener<'src>>,
    ast: &[Declaration<'src>],
    scope: &ScopeInfo,
) -> Vec<XenoDiagnostic<'src>> {
    let mut listeners = vec![listener];
    listeners[0].on_before_module(scope);
    let mut diagnostics = Vec::new();
    walk_ast(&mut listeners, ast, &mut diagnostics);
    listeners[0].on_after_module(scope);
    diagnostics
}

pub(crate) fn walk_ast<'src>(
    ls: &mut Listeners<'src>,
    ast: &[Declaration<'src>],
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_ast(ast, errors);
    }
    for decl in ast {
        walk_decl(ls, decl, errors);
    }
    for l in ls.iter_mut() {
        l.on_after_ast(ast, errors);
    }
}

fn walk_decl<'src>(
    ls: &mut Listeners<'src>,
    decl: &Declaration<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_decl(decl, errors);
    }
    match decl {
        Declaration::Type { ty: t, .. } => {
            walk_type(ls, t, errors);
        }
        _ => {} // Declaration::Custom {
                //     plugin_id,
                //     decl_id,
                //     name,
                //     docs,
                //     value,
                // } => walk_custom(plugin_id, decl_id, name, docs, value, ls, errors),
    }
    for l in ls.iter_mut() {
        l.on_after_decl(decl, errors);
    }
}

fn walk_type<'src>(
    ls: &mut Listeners<'src>,
    ty: &XenoType<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_type(ty, errors);
    }
    walk_type_expr(ls, &ty.0, errors);
    for annotation in &ty.1 {
        walk_annotation(ls, annotation, errors);
    }
    for l in ls.iter_mut() {
        l.on_after_type(ty, errors);
    }
}

// fn walk_custom<'src>(
//     plugin_id: &str,
//     decl_id: &str,
//     name: &Option<&TokenData<'src>>,
//     docs: &Option<&'src str>,
//     value: &Box<dyn Any>,
//     ls: &mut Listeners<'src>,
//     errors: &mut Vec<XenoError<'src>>,
// ) {
//     for l in ls.iter_mut() {
//         l.on_before_custom(plugin_id, decl_id, name, docs, value, errors);
//     }
//     for l in ls.iter_mut() {
//         l.on_after_custom(errors);
//     }
// }

fn walk_expr<'src>(
    ls: &mut Listeners<'src>,
    expr: &Expr<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_expr(expr, errors);
    }
    match expr {
        Expr::Regex(_) => {}
        Expr::Annotation(annotation) => walk_annotation(ls, annotation, errors),
        Expr::Type(ty) => walk_type_expr(ls, ty, errors),
    }
    for l in ls.iter_mut() {
        l.on_after_expr(expr, errors);
    }
}

fn walk_annotation<'src>(
    ls: &mut Listeners<'src>,
    annotation: &Annotation<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_annotation(annotation.ident, &annotation.params, errors);
    }
    for param in &annotation.params {
        walk_expr(ls, param, errors);
    }
    for l in ls.iter_mut() {
        l.on_after_annotation(annotation.ident, &annotation.params, errors);
    }
}

fn walk_type_expr<'src>(
    ls: &mut Listeners<'src>,
    ty: &Type<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    match ty {
        Type::Simple(simple) => walk_simple_type(ls, simple, errors),
        Type::Struct(fields) => {
            for l in ls.iter_mut() {
                l.on_before_struct(fields, errors);
            }
            for (key, value, _) in fields {
                for l in ls.iter_mut() {
                    l.on_before_field(key, value, errors);
                }
                walk_simple_type(ls, value, errors);
                for l in ls.iter_mut() {
                    l.on_after_field(key, value, errors);
                }
            }
            for l in ls.iter_mut() {
                l.on_after_struct(fields, errors);
            }
        }
        Type::Enum(variants) => {
            for l in ls.iter_mut() {
                l.on_before_enum(variants, errors);
            }
            for (key, value, _) in variants {
                for l in ls.iter_mut() {
                    l.on_before_field(key, value, errors);
                }
                walk_simple_type(ls, value, errors);
                for l in ls.iter_mut() {
                    l.on_after_field(key, value, errors);
                }
            }
            for l in ls.iter_mut() {
                l.on_after_enum(variants, errors);
            }
        }
        Type::Tuple(inner) => {
            for l in ls.iter_mut() {
                l.on_before_list(inner, errors);
            }
            for simple in inner {
                walk_simple_type(ls, simple, errors);
            }
            for l in ls.iter_mut() {
                l.on_after_list(inner, errors);
            }
        }
        Type::Set(set) => {
            for l in ls.iter_mut() {
                l.on_before_set(set, errors);
            }
            if let Some(element_type) = &set.element_type {
                walk_simple_type(ls, element_type, errors);
            }
            for literal in set.values.as_deref().unwrap_or_default() {
                walk_simple_type(ls, &SimpleType::Literal(literal.clone()), errors);
            }
            for l in ls.iter_mut() {
                l.on_after_set(set, errors);
            }
        }
        Type::Sum(inner) | Type::Intersection(inner) => {
            for simple in inner {
                walk_simple_type(ls, simple, errors);
            }
        }
    }
}

fn walk_simple_type<'src>(
    ls: &mut Listeners<'src>,
    ty: &SimpleType<'src>,
    errors: &mut Vec<XenoDiagnostic<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_simple_type(ty, errors);
    }
    let arguments = match ty.inner() {
        SimpleType::Identifier(_, arguments) | SimpleType::Array(_, arguments) => arguments,
        SimpleType::Literal(_) | SimpleType::Optional(_) => return,
    };
    for argument in arguments.as_deref().unwrap_or(&[]) {
        walk_simple_type(ls, argument, errors);
    }
    if let SimpleType::Array(ident, _) = ty.inner() {
        for l in ls.iter_mut() {
            l.on_array(ident, errors);
            l.on_after_array(ident, errors);
        }
    }
}

// ── Def tree (unchanged, kept for plugin use) ───────────────────────

type XenoDefTree<'src> = HashMap<&'src str, XenoDefNode<'src>>;
pub struct XenoDefNode<'src> {
    pub name: &'src str,
    pub docs: Option<&'src str>,
    pub fields: Option<XenoDefTree<'src>>,
    // TODO from-to
    /** Can contain any data, for plugin developers */
    pub meta: Option<Box<dyn std::any::Any>>,
}

impl XenoDefNode<'_> {
    pub fn ast_to_def_tree<'src>(ast: &'src Vec<Declaration>) -> XenoDefTree<'src> {
        let mut def_tree: XenoDefTree = HashMap::new();

        for declaration in ast {
            match declaration {
                Declaration::Type { name, docs, .. } => {
                    let node = XenoDefNode {
                        name: name.v,
                        docs: *docs,
                        fields: None,
                        meta: Some(Box::new(Some(true))),
                    };
                    def_tree.insert(name.v, node);
                }
                _ => {} // Declaration::Custom { docs, name, .. } => {
                        //     if let Some(n) = name {
                        //         def_tree.insert(
                        //             n.v,
                        //             XenoDefNode {
                        //                 name: n.v,
                        //                 docs: *docs,
                        //                 fields: None,
                        //                 meta: None,
                        //             },
                        //         );
                        //     }
                        // }
            }
        }

        def_tree
    }

    pub fn find_definition<'src>(
        location: &'src TokenData<'src>,
        def_tree: &'src XenoDefTree<'src>,
    ) -> Option<&'src XenoDefNode<'src>> {
        for node in def_tree.values() {
            if node.name == location.v {
                return Some(node);
            }
            if let Some(children) = &node.fields {
                if let Some(found) = Self::find_definition(location, children) {
                    return Some(found);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        semantic::{TypeDeclarationInfo, HAS_LENGTH, NUMERIC},
        XenoDiagSeverity,
    };

    fn diagnostic(severity: XenoDiagSeverity) -> XenoDiagnostic<'static> {
        XenoDiagnostic {
            location: TokenData::default(),
            message: "diagnostic".to_string(),
            severity,
        }
    }

    #[test]
    fn warnings_and_infos_allow_generation() {
        let diagnostics = [
            diagnostic(XenoDiagSeverity::Warn),
            diagnostic(XenoDiagSeverity::Info),
        ];

        assert!(can_generate(&diagnostics));
    }

    #[test]
    fn errors_block_generation() {
        assert!(!can_generate(&[diagnostic(XenoDiagSeverity::Err)]));
    }

    #[test]
    fn sdk_listener_runs_module_lifecycle_without_host_validators() {
        use std::sync::{Arc, Mutex};
        struct Listener(Arc<Mutex<Vec<&'static str>>>);
        impl<'src> AnalyzerListener<'src> for Listener {
            fn on_before_module(&mut self, _: &ScopeInfo) {
                self.0.lock().unwrap().push("before_module");
            }
            fn on_before_ast(
                &mut self,
                _: &[Declaration<'src>],
                _: &mut Vec<XenoDiagnostic<'src>>,
            ) {
                self.0.lock().unwrap().push("before_ast");
            }
            fn on_after_ast(&mut self, _: &[Declaration<'src>], _: &mut Vec<XenoDiagnostic<'src>>) {
                self.0.lock().unwrap().push("after_ast");
            }
            fn on_after_module(&mut self, _: &ScopeInfo) {
                self.0.lock().unwrap().push("after_module");
            }
        }
        let tokens = crate::lexer::Lexer::tokenize("type Value = Unknown @missing;").unwrap();
        let (ast, diagnostics) = crate::parser::Parser::parse(&tokens);
        assert!(diagnostics.is_empty());
        let events = Arc::new(Mutex::new(Vec::new()));
        let diagnostics = run_plugin_listener(
            Box::new(Listener(events.clone())),
            &ast,
            &scope_with_declarations(HashMap::new()),
        );
        assert!(diagnostics.is_empty());
        assert_eq!(
            *events.lock().unwrap(),
            ["before_module", "before_ast", "after_ast", "after_module"]
        );
        assert_eq!(first_source_location(&ast).v, "type");
        assert_eq!(first_source_location(&[]).v, "");
    }

    fn scope_with_declarations(
        type_declarations: HashMap<String, TypeDeclarationInfo>,
    ) -> ScopeInfo {
        let own_types = type_declarations.keys().cloned().collect();
        let mut type_hierarchy = TypeHierarchy::default();
        type_hierarchy.set_current_module("test");
        type_hierarchy.register_module("test", Vec::new());
        for semantic_type in BUILTIN_TYPES {
            type_hierarchy.insert_semantic_type(semantic_type);
        }
        for (name, declaration) in type_declarations {
            type_hierarchy.insert_declaration("test", name, declaration);
        }
        ScopeInfo {
            module_path: "test".to_string(),
            abs_path: PathBuf::new(),
            own_types,
            imported_types: HashMap::new(),
            builtin_types: BUILTIN_TYPES
                .iter()
                .map(|semantic_type| semantic_type.name.to_string())
                .collect(),
            known_annotations: HashSet::new(),
            type_hierarchy,
            annotations: HashMap::new(),
        }
    }

    #[test]
    fn user_types_inherit_traits_through_specialized_parent_chains() {
        let generic = GenericParameterInfo {
            name: "T".to_string(),
            constraint: None,
            constraint_scope: None,
        };
        let mut declarations = HashMap::new();
        declarations.insert(
            "Wrapped".to_string(),
            TypeDeclarationInfo {
                generic_params: vec![generic.clone()],
                parents: vec![OwnedType::named("T")],
                body: OwnedType::named("T"),
                transparent_alias: true,
            },
        );
        declarations.insert(
            "Outer".to_string(),
            TypeDeclarationInfo {
                generic_params: vec![generic],
                parents: vec![OwnedType::Named {
                    name: "Wrapped".to_string(),
                    arguments: vec![OwnedType::named("T")],
                }],
                body: OwnedType::Named {
                    name: "Wrapped".to_string(),
                    arguments: vec![OwnedType::named("T")],
                },
                transparent_alias: true,
            },
        );
        let scope = scope_with_declarations(declarations);

        let outer_string = OwnedType::Named {
            name: "Outer".to_string(),
            arguments: vec![OwnedType::named("string")],
        };
        let outer_integer = OwnedType::Named {
            name: "Outer".to_string(),
            arguments: vec![OwnedType::named("u8")],
        };

        assert!(scope.type_implements_trait(&outer_string, &HAS_LENGTH));
        assert!(!scope.type_implements_trait(&outer_string, &NUMERIC));
        assert!(scope.type_implements_trait(&outer_integer, &NUMERIC));
        assert!(!scope.type_implements_trait(&outer_integer, &HAS_LENGTH));
    }
}
