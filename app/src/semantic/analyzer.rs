use crate::parser::parser_expr::{Declaration, Expr};


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