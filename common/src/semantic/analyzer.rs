// use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::plugins::ListenerFactory;
use crate::{
    config::PluginConfigs,
    module::ModuleData,
    parser::{Annotation, Declaration, Expr, KeyValExpr, SetType, SimpleType, Type, XenoType},
    plugins::XenoPlugin,
    semantic::{
        annotation_validator::AnnotationValidator, if_validator::IfChainValidator,
        intersection_validator::IntersectionValidator, name_validator::NameValidator,
        GenericParameterInfo, OwnedType, TypeHierarchy, BUILTIN_ANNOTATIONS, BUILTIN_TRAITS,
        BUILTIN_TYPES,
    },
    TokenData, XenoDiagnostic,
};

/// Scope information built by the analyzer and passed to listeners.
/// Contains all known types/annotations with their provenance.
#[derive(Debug, Clone)]
pub struct ScopeInfo {
    /// Module path of the current module.
    pub module_path: String,
    /// Absolute filesystem path of the current module.
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
    pub annotations: HashMap<String, &'static crate::semantic::XenoAnnotation>,
}

impl ScopeInfo {
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

    pub fn find_annotation(&self, name: &str) -> Option<&'static crate::semantic::XenoAnnotation> {
        self.annotations.get(name).copied()
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
        target: &'static crate::semantic::XenoType,
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

/// Stateless analyzer that holds registered listener factories.
/// Created once during registry construction, reused for every module analysis.
pub struct Analyzer {
    /// Factories for listeners that run on every analysis (builtins + plugins).
    listener_factories: Vec<ListenerFactory>,
    /// Whether to use generation mode (true) or analyzer/LSP mode (false).
    pub generation_mode: bool,
}

impl Analyzer {
    pub fn new(generation_mode: bool, plugins: &[&'static XenoPlugin<'static>]) -> Self {
        let mut factories: Vec<ListenerFactory> = Vec::new();

        // Register plugin listeners
        for plugin in plugins {
            let register_fn = if generation_mode {
                plugin.register_generator
            } else {
                plugin.register_analyzer
            };
            if let Some(factory) = register_fn {
                factories.push(factory);
            }
        }

        Analyzer {
            listener_factories: factories,
            generation_mode,
        }
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
        plugins: &[&'static XenoPlugin<'static>],
        plugin_configs: &PluginConfigs,
    ) -> Vec<XenoDiagnostic<'src>> {
        // ── Pass 1: collect every type and trait before validation ──
        let mut builtin_types: HashSet<String> = HashSet::new();
        let mut known_annotations: HashSet<String> = HashSet::new();
        let mut type_hierarchy = TypeHierarchy::default();
        let mut annotations = HashMap::new();

        // Builtins
        for t in BUILTIN_TYPES {
            builtin_types.insert(t.name.to_string());
            type_hierarchy.insert_semantic_type(t);
        }
        for xeno_trait in BUILTIN_TRAITS {
            type_hierarchy.insert_trait(xeno_trait);
        }
        for a in BUILTIN_ANNOTATIONS {
            known_annotations.insert(a.name.to_string());
            annotations.insert(a.name.to_string(), *a);
            for parameter in a.params {
                match parameter.constraint {
                    crate::semantic::XenoConstraint::Type(required) => {
                        type_hierarchy.insert_semantic_type(required)
                    }
                    crate::semantic::XenoConstraint::Trait(required) => {
                        type_hierarchy.insert_trait(required)
                    }
                }
            }
        }

        // Plugin-provided names
        for plugin in plugins {
            if let Some(provide) = plugin.provide_types {
                for semantic_type in provide() {
                    builtin_types.insert(semantic_type.name.to_string());
                    type_hierarchy.insert_semantic_type(semantic_type);
                }
            }
            if let Some(provide) = plugin.provide_annotations {
                for annotation in provide() {
                    known_annotations.insert(annotation.name.to_string());
                    annotations.insert(annotation.name.to_string(), *annotation);
                    for parameter in annotation.params {
                        match parameter.constraint {
                            crate::semantic::XenoConstraint::Type(required) => {
                                type_hierarchy.insert_semantic_type(required)
                            }
                            crate::semantic::XenoConstraint::Trait(required) => {
                                type_hierarchy.insert_trait(required)
                            }
                        }
                    }
                }
            }
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
        let mut listeners: Vec<Box<dyn AnalyzerListener<'src>>> = Vec::new();
        for f in &self.listener_factories {
            let listener: Box<dyn AnalyzerListener<'src>> = f(plugin_configs);
            listeners.push(listener);
        }

        // Add the name validator (always present)
        listeners.push(Box::new(NameValidator::new(&scope)));
        listeners.push(Box::new(IntersectionValidator::new(&scope)));
        listeners.push(Box::new(AnnotationValidator::new(&scope)));
        listeners.push(Box::new(IfChainValidator::new()));

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

fn can_generate(diagnostics: &[XenoDiagnostic<'_>]) -> bool {
    !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::XenoDiagSeverity::Err)
}

// ── Walk functions (free functions to avoid &mut self borrow issues) ─

type Listeners<'src> = [Box<dyn AnalyzerListener<'src>>];

fn walk_ast<'src>(
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
