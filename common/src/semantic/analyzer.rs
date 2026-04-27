use std::collections::{HashMap, HashSet};

use crate::{
    parser::{AnonymType, Declaration, Expr, KeyValExpr, TypeList},
    semantic::{if_validator::IfChainValidator, name_validator::NameValidator},
    TokenData, XenoError,
};

/// Trait for AST walk event listeners. All methods have default no-op
/// implementations so listeners only need to override the events they
/// care about.
#[allow(unused_variables)]
pub trait AnalyzerListener<'src> {
    fn on_before_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_ast(&mut self, ast: &[Declaration<'src>], errors: &mut Vec<XenoError<'src>>) {}

    fn on_before_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoError<'src>>) {}
    fn on_after_decl(&mut self, decl: &Declaration<'src>, errors: &mut Vec<XenoError<'src>>) {}

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

pub struct Analyzer<'src, 'a> {
    listeners: Vec<Box<dyn AnalyzerListener<'src> + 'a>>,
}

impl<'src, 'a> Analyzer<'src, 'a> {
    pub fn new() -> Self {
        Analyzer {
            listeners: Vec::new(),
        }
    }

    pub fn add_listener(&mut self, listener: impl AnalyzerListener<'src> + 'a) {
        self.listeners.push(Box::new(listener));
    }

    pub fn analyze(mut self, ast: &[Declaration<'src>]) -> Vec<XenoError<'src>> {
        let mut errors = Vec::new();
        walk_ast(&mut self.listeners, ast, &mut errors);
        errors
    }
}

// ── Walk functions (free functions to avoid &mut self borrow issues) ─

type Listeners<'src, 'a> = [Box<dyn AnalyzerListener<'src> + 'a>];

fn walk_ast<'src>(
    ls: &mut Listeners<'src, '_>,
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
    ls: &mut Listeners<'src, '_>,
    decl: &Declaration<'src>,
    errors: &mut Vec<XenoError<'src>>,
) {
    for l in ls.iter_mut() {
        l.on_before_decl(decl, errors);
    }
    match decl {
        Declaration::Import { .. } => {}
        Declaration::TypeDecl { t, .. } => {
            walk_type(ls, t, errors);
        }
    }
    for l in ls.iter_mut() {
        l.on_after_decl(decl, errors);
    }
}

fn walk_type<'src>(
    ls: &mut Listeners<'src, '_>,
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

fn walk_expr<'src>(
    ls: &mut Listeners<'src, '_>,
    expr: &Expr<'src>,
    errors: &mut Vec<XenoError<'src>>,
) {
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

// ── Convenience: default analyzer with built-in listeners ───────────

pub fn analyze<'src>(
    ast: &[Declaration<'src>],
    known_types: &HashSet<&str>,
    known_annotations: &HashSet<&str>,
) -> Vec<XenoError<'src>> {
    let mut analyzer = Analyzer::new();
    analyzer.add_listener(IfChainValidator::new());
    analyzer.add_listener(NameValidator::new(
        known_types.clone(),
        known_annotations.clone(),
    ));
    analyzer.analyze(ast)
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
                }
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
