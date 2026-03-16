use std::collections::HashMap;

use crate::{
    parser::{Declaration, Expr},
    TokenData,
};

pub fn analyze<'src>(ast: &Vec<Declaration<'src>>) {
    for declaration in ast {
        match declaration {
            Declaration::TypeDecl { t, .. } => {
                validate_annotations(&t);
                //print!("{:?}", t)
            }
        }
    }
}

pub fn find_token_under_cursor<'src>(
    _line: u32,
    _column: u32,
    ast: &'src Vec<Declaration>,
) -> Option<&'src TokenData<'src>> {
    for declaration in ast {
        match declaration {
            Declaration::TypeDecl { name: _, t: _, .. } => {
                return None;
            }
        }
    }
    None
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

fn validate_annotations(exprs: &Vec<Expr>) {
    let mut valid_context = false; // Tracks if @if or @elseif has been encountered.

    for expr in exprs {
        match expr {
            Expr::Annotation(id, _) => {
                match id.v {
                    "if" => {
                        valid_context = true; // Start of a valid context.
                    }
                    "elseif" | "else" => {
                        if !valid_context {
                            panic!(
                                "Semantic error: '{}' annotation must follow an '@if' or '@elseif'",
                                id.v
                            );
                        }
                    }
                    _ => {
                        valid_context = false;
                    }
                }
            }
            _ => {
                valid_context = false;
            }
        }
    }
}
