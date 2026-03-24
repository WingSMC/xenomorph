use std::collections::HashMap;

use crate::{
    parser::{Declaration, Expr},
    TokenData, XenoError,
};

pub struct Analyzer {
    // on_before_ast, on_after_ast

    // on_before_custom_decl, on_after_custom_decl
    // on_before_decl, on_after_decl
    //   on_before_type, on_after_type
    //     on_before_expr, on_after_expr
    //       on_before_struct, on_after_struct
    //         on_before_field, on_after_field
    //       on_before_enum, on_after_enum
    //       on_before_list, on_after_list
    //       on_before_set, on_after_set
    //       on_before_annotation, on_after_annotation

    // cache: HashMap<filename, {
    //   text,
    //   tokens,
    //   ast,
    //   def_tree,
    //   errors,
    //}>,
}

pub fn analyze<'src>(ast: &Vec<Declaration<'src>>) -> Vec<XenoError<'src>> {
    let mut errors = Vec::new();

    for declaration in ast {
        match declaration {
            Declaration::TypeDecl { t, .. } => {
                validate_annotations(t, &mut errors);
            }
        }
    }

    errors
}

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

/// Tracks the state of an @if / @elseif / @else annotation chain.
#[derive(Clone, Copy, PartialEq)]
enum IfChainState {
    /// No active chain — we are outside any @if block.
    None,
    /// Just saw @if (or @elseif) — can be followed by @elseif or @else.
    AfterIf,
    /// Just saw @else — chain is closed, nothing else may follow.
    AfterElse,
}

fn validate_annotations<'src>(exprs: &Vec<Expr<'src>>, errors: &mut Vec<XenoError<'src>>) {
    let mut chain = IfChainState::None;

    for expr in exprs {
        match expr {
            Expr::Struct(fields) => {
                for field in fields {
                    validate_annotations(&field.1, errors);
                }
            }
            Expr::Annotation(id, _) => {
                match id.v {
                    "if" => chain = IfChainState::AfterIf,
                    "elseif" => match chain {
                        IfChainState::AfterIf => {}
                        IfChainState::None | IfChainState::AfterElse => {
                            errors.push(XenoError {
                                location: (*id).clone(),
                                message: "'@elseif' must follow an '@if' or another '@elseif'."
                                    .to_string(),
                            });
                            chain = IfChainState::None; // Chain is broken; reset
                        }
                    },
                    "else" => match chain {
                        IfChainState::AfterIf => chain = IfChainState::AfterElse,
                        IfChainState::None | IfChainState::AfterElse => {
                            errors.push(XenoError {
                                location: (*id).clone(),
                                message: "'@else' must follow an '@if' or '@elseif'.".to_string(),
                            });
                            chain = IfChainState::None; // Chain is broken; reset
                        }
                    },
                    _ => chain = IfChainState::None,
                }
            }
            _ => chain = IfChainState::None,
        }
    }
}
