// use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::{
    config::PluginConfigs,
    module::ModuleData,
    parser::{AnonymType, Declaration, Expr, KeyValExpr, TypeList},
    plugins::XenoPlugin,
    semantic::{
        if_validator::IfChainValidator, name_validator::NameValidator, BUILTIN_ANNOTATIONS,
        BUILTIN_TYPES,
    },
    TokenData, XenoError,
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
}

impl ScopeInfo {
    /// Returns true if `name` is a known type (own, imported, or builtin).
    pub fn has_type(&self, name: &str) -> bool {
        self.builtin_types.contains(name)
            || self.own_types.iter().any(|n| n == name)
            || self
                .imported_types
                .values()
                .any(|names| names.iter().any(|n| n == name))
    }

    /// Returns true if `name` is a known annotation.
    pub fn has_annotation(&self, name: &str) -> bool {
        self.known_annotations.contains(name)
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
    /// Called once per analysis run with all plugin configs from `.xenomorphrc`.
    fn on_init(&mut self, plugin_configs: &PluginConfigs) {}

    /// Called before the AST walk begins, with full scope information.
    fn on_before_module(&mut self, scope: &ScopeInfo) {}
    /// Called after the full AST walk completes, with scope information.
    fn on_after_module(&mut self, scope: &ScopeInfo) {}

    fn on_before_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoError<'src>>) {}

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

    fn on_before_type(&mut self, exprs: &AnonymType<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_type(&mut self, exprs: &AnonymType<'src>, errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_expr(&mut self, expr: &Expr<'src>, errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_struct(&mut self, fields: &[KeyValExpr<'src>], errors: &mut Vec<XenoError<'src>>) {
    }
    fn on_after_struct(&mut self, fields: &[KeyValExpr<'src>], errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_field(
        &mut self,
        key: &TokenData<'src>,
        value: &AnonymType<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
    }
    fn on_after_field(
        &mut self,
        key: &TokenData<'src>,
        value: &AnonymType<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
    }

    fn on_before_enum(&mut self, variants: &[KeyValExpr<'src>], errors: &mut Vec<XenoError<'src>>) {
    }
    fn on_after_enum(&mut self, variants: &[KeyValExpr<'src>], errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_list(&mut self, inner: &TypeList<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_list(&mut self, inner: &TypeList<'src>, errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_set(&mut self, inner: &TypeList<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_set(&mut self, inner: &TypeList<'src>, errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
    }
    fn on_after_annotation(
        &mut self,
        name: &TokenData<'src>,
        args: &TypeList<'src>,
        errors: &mut Vec<XenoError<'src>>,
    ) {
    }
}

/// A factory function that creates a fresh listener instance for each analysis run.
pub type ListenerFactory = fn() -> Box<dyn for<'a> AnalyzerListener<'a>>;

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

        // Register builtin listeners
        factories.push(|| Box::new(IfChainValidator::new()));

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
    ) -> Vec<XenoError<'src>> {
        // ── Build ScopeInfo ──
        let mut builtin_types: HashSet<String> = HashSet::new();
        let mut known_annotations: HashSet<String> = HashSet::new();

        // Builtins
        for t in &BUILTIN_TYPES {
            builtin_types.insert(t.name.to_string());
        }
        for a in &BUILTIN_ANNOTATIONS {
            known_annotations.insert(a.name.to_string());
        }

        // Plugin-provided names
        for plugin in plugins {
            if let Some(provide) = plugin.provide_types {
                for pc in provide() {
                    builtin_types.insert(pc.label.to_string());
                }
            }
            if let Some(provide) = plugin.provide_annotations {
                for pc in provide() {
                    known_annotations.insert(pc.label.to_string());
                }
            }
        }

        // Own declarations
        let own_types: Vec<String> = module_data
            .borrow_declarations()
            .keys()
            .map(|k| k.to_string())
            .collect();

        // Imported declarations grouped by module
        let mut imported_types: HashMap<String, Vec<String>> = HashMap::new();
        for import in imports {
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
            module_path: module_data.borrow_module_path().to_string(),
            abs_path: module_data.borrow_abs_path().to_path_buf(),
            own_types,
            imported_types,
            builtin_types,
            known_annotations,
        };

        // ── Create listeners ──
        let mut listeners: Vec<Box<dyn AnalyzerListener<'src>>> = Vec::new();
        for f in &self.listener_factories {
            let listener: Box<dyn AnalyzerListener<'src>> = f();
            listeners.push(listener);
        }

        // Add the name validator (always present)
        listeners.push(Box::new(NameValidator::new(&scope)));

        // Pass plugin configs to all listeners
        for l in listeners.iter_mut() {
            l.on_init(plugin_configs);
        }

        // Notify listeners of module context + scope
        for l in listeners.iter_mut() {
            l.on_before_module(&scope);
        }

        // Walk the AST
        let mut errors = Vec::new();
        walk_ast(&mut listeners, ast, &mut errors);

        // Notify listeners that the module is done
        for l in listeners.iter_mut() {
            l.on_after_module(&scope);
        }

        errors
    }
}

// ── Walk functions (free functions to avoid &mut self borrow issues) ─

type Listeners<'src> = [Box<dyn AnalyzerListener<'src>>];

fn walk_ast<'src>(
    ls: &mut Listeners<'src>,
    ast: &[Declaration<'src>],
    errors: &mut Vec<XenoError<'src>>,
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
    errors: &mut Vec<XenoError<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_decl(decl, errors);
    }
    match decl {
        Declaration::TypeDecl { t, .. } => {
            walk_type(ls, t, errors);
        }
        Declaration::Import { .. } => {} // Declaration::Custom {
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
    exprs: &AnonymType<'src>,
    errors: &mut Vec<XenoError<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_type(exprs, errors);
    }
    for expr in exprs {
        walk_expr(ls, expr, errors);
    }
    for l in ls.iter_mut() {
        l.on_after_type(exprs, errors);
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

fn walk_expr<'src>(ls: &mut Listeners<'src>, expr: &Expr<'src>, errors: &mut Vec<XenoError<'src>>) {
    for l in ls.iter_mut() {
        l.on_before_expr(expr, errors);
    }
    match expr {
        Expr::Struct(fields) => {
            for l in ls.iter_mut() {
                l.on_before_struct(fields, errors);
            }
            for (key, value) in fields {
                for l in ls.iter_mut() {
                    l.on_before_field(key, value, errors);
                }
                walk_type(ls, value, errors);
                for l in ls.iter_mut() {
                    l.on_after_field(key, value, errors);
                }
            }
            for l in ls.iter_mut() {
                l.on_after_struct(fields, errors);
            }
        }
        Expr::Enum(variants) => {
            for l in ls.iter_mut() {
                l.on_before_enum(variants, errors);
            }
            for (key, value) in variants {
                for l in ls.iter_mut() {
                    l.on_before_field(key, value, errors);
                }
                walk_type(ls, value, errors);
                for l in ls.iter_mut() {
                    l.on_after_field(key, value, errors);
                }
            }
            for l in ls.iter_mut() {
                l.on_after_enum(variants, errors);
            }
        }
        Expr::List(inner) => {
            for l in ls.iter_mut() {
                l.on_before_list(inner, errors);
            }
            for anon_type in inner {
                walk_type(ls, anon_type, errors);
            }
            for l in ls.iter_mut() {
                l.on_after_list(inner, errors);
            }
        }
        Expr::Set(inner) => {
            for l in ls.iter_mut() {
                l.on_before_set(inner, errors);
            }
            for anon_type in inner {
                walk_type(ls, anon_type, errors);
            }
            for l in ls.iter_mut() {
                l.on_after_set(inner, errors);
            }
        }
        Expr::Annotation(name, args) => {
            for l in ls.iter_mut() {
                l.on_before_annotation(name, args, errors);
            }
            for anon_type in args {
                walk_type(ls, anon_type, errors);
            }
            for l in ls.iter_mut() {
                l.on_after_annotation(name, args, errors);
            }
        }
        Expr::Not(inner) => {
            walk_expr(ls, inner, errors);
        }
        Expr::BinaryExpr(_, pair) => {
            walk_expr(ls, &pair.0, errors);
            walk_expr(ls, &pair.1, errors);
        }
        Expr::Identifier(_) | Expr::Literal(_) | Expr::Regex(_) | Expr::FieldAccess(_) => {}
    }
    for l in ls.iter_mut() {
        l.on_after_expr(expr, errors);
    }
}

// ── Def tree (unchanged, kept for plugin use) ───────────────────────

type XenoDefTree<'src> = HashMap<&'src str, XenoDefNode<'src>>;
pub struct XenoDefNode<'src> {
    pub name: &'src str,
    pub docs: Option<&'src str>,
    pub fields: Option<XenoDefTree<'src>>,
    /** Can contain any data, for plugin developers */
    pub meta: Option<Box<dyn std::any::Any>>,
}

impl XenoDefNode<'_> {
    pub fn ast_to_def_tree<'src>(ast: &'src Vec<Declaration>) -> XenoDefTree<'src> {
        let mut def_tree: XenoDefTree = HashMap::new();

        for declaration in ast {
            match declaration {
                Declaration::Import { .. } => {}
                Declaration::TypeDecl { name, docs, t } => {
                    let node = XenoDefNode {
                        name: name.v,
                        docs: *docs,
                        fields: None,
                        meta: Some(Box::new(match t {
                            _ => Some(true),
                        })),
                    };
                    def_tree.insert(name.v, node);
                } // Declaration::Custom { docs, name, .. } => {
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
