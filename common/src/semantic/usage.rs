use std::collections::{HashMap, HashSet};

use super::declaration_dependencies;
use crate::{parser::Declaration, TokenData, XenoDiagSeverity, XenoDiagnostic, XenoDiagnosticTag};

/// Local import usage and optional reachability from the configured entry exports.
/// A public import is used by definition, even when no local declaration refers to it.
pub fn analyze_usage(
    ast: &[Declaration],
    source: &str,
    module: &str,
    imported_types: &HashMap<String, Vec<String>>,
    reachable: Option<&HashSet<(String, String)>>,
) -> Vec<XenoDiagnostic> {
    let references: HashSet<_> = ast
        .iter()
        .flat_map(|d| declaration_dependencies(d, source))
        .collect();
    let known_imports: HashSet<_> = imported_types
        .values()
        .flatten()
        .map(String::as_str)
        .collect();
    let mut diagnostics = vec![];
    for declaration in ast {
        match declaration {
            Declaration::Import {
                path,
                names,
                public: None,
                ..
            } => {
                for token in names {
                    let name = token.text(source);
                    if name == "*" {
                        if let Some(names) = imported_types.get(&path.join("/")) {
                            if !names.is_empty()
                                && names.iter().all(|name| !references.contains(name))
                            {
                                diagnostics.push(XenoDiagnostic {
                                    location: *token,
                                    message: "Unused wildcard import.".into(),
                                    severity: XenoDiagSeverity::Warn,
                                    tags: vec![XenoDiagnosticTag::Unnecessary],
                                });
                            }
                        }
                        continue;
                    }
                    if known_imports.contains(name) && !references.contains(name) {
                        diagnostics.push(XenoDiagnostic {
                            location: *token,
                            message: format!("Unused import '{name}'."),
                            severity: XenoDiagSeverity::Warn,
                            tags: vec![XenoDiagnosticTag::Unnecessary],
                        });
                    }
                }
            }
            Declaration::Type {
                name,
                from,
                to,
                public,
                ..
            } if reachable
                .is_some_and(|set| !set.contains(&(module.into(), name.text(source).into()))) =>
            {
                diagnostics.push(XenoDiagnostic {
                    location: TokenData {
                        start: public.unwrap_or(*from).start,
                        end: to.end,
                    },
                    message: "Not used by the entry module's exports.".into(),
                    severity: XenoDiagSeverity::Info,
                    tags: vec![XenoDiagnosticTag::Unnecessary],
                });
            }
            _ => {}
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::Lexer, parser::Parser};

    fn usage(source: &str, imported: &[&str], reachable: Option<&[&str]>) -> Vec<XenoDiagnostic> {
        let tokens = Lexer::tokenize(source).unwrap();
        let (ast, errors) = Parser::parse(&tokens, source);
        assert!(
            !errors.iter().any(|d| d.severity == XenoDiagSeverity::Err),
            "{errors:?}"
        );
        let imported_types = HashMap::from([(
            "other".into(),
            imported.iter().map(|name| (*name).into()).collect(),
        )]);
        let reachable = reachable.map(|names| {
            names
                .iter()
                .map(|name| ("index".into(), (*name).into()))
                .collect()
        });
        analyze_usage(&ast, source, "index", &imported_types, reachable.as_ref())
    }

    #[test]
    fn wildcard_usage_warns_only_when_no_selected_name_is_used() {
        assert!(usage(
            "import other::*; pub type Root = Used;",
            &["Used", "Spare"],
            None
        )
        .is_empty());
        assert!(usage("pub import other::*;", &["Used", "Spare"], None).is_empty());
        assert!(usage("import other::*;", &[], None).is_empty());
        let diagnostics = usage("import other::*;", &["Used", "Spare"], None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Unused wildcard import.");
        assert_eq!(diagnostics[0].location.text("import other::*;"), "*");
    }

    #[test]
    fn grouped_imports_mark_only_unreferenced_names_and_ignore_field_keys() {
        let source = "import other::{Used, Spare}; pub type Root = { Spare: Used };";
        let diagnostics = usage(source, &["Used", "Spare"], None);
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.message, "Unused import 'Spare'.");
        assert_eq!(diagnostic.location.text(source), "Spare");
        assert_eq!(
            diagnostic.location.start as usize,
            source.find("Spare").unwrap()
        );
        assert_eq!(diagnostic.severity, XenoDiagSeverity::Warn);
        assert_eq!(diagnostic.tags, [XenoDiagnosticTag::Unnecessary]);
    }

    #[test]
    fn private_and_unreachable_declarations_still_use_their_imports() {
        let source = "import other::{Bound, Argument, Annotated}; type Dead<T: Bound> = ?Argument[] @custom(@nested(Annotated));";
        let diagnostics = usage(source, &["Bound", "Argument", "Annotated"], Some(&[]));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "Not used by the entry module's exports."
        );
        assert!(diagnostics[0]
            .location
            .text(source)
            .starts_with("type Dead<T: Bound>"));
        assert_eq!(diagnostics[0].severity, XenoDiagSeverity::Info);
        assert_eq!(diagnostics[0].tags, [XenoDiagnosticTag::Unnecessary]);
    }

    #[test]
    fn public_and_unresolved_imports_do_not_receive_unused_warnings() {
        assert!(usage(
            "pub import other::{Api, Spare};",
            &["Api", "Spare"],
            Some(&[])
        )
        .is_empty());
        assert!(usage("import other::Missing;", &[], None).is_empty());
    }

    #[test]
    fn generic_parameters_annotation_names_and_string_literals_are_not_import_usage() {
        let diagnostics = usage(
            "import other::{T, custom, Text}; type Dead<T> = { Text: T, label: \"Text\" } @custom;",
            &["T", "custom", "Text"],
            None,
        );
        assert_eq!(diagnostics.len(), 3);
        assert!(diagnostics
            .iter()
            .all(|d| d.message.starts_with("Unused import")));
    }

    #[test]
    fn reachable_private_helpers_are_preserved_and_unused_public_types_are_marked() {
        let source = "type Helper = string; pub type Root = Helper; pub type Spare = string;";
        let diagnostics = usage(source, &[], Some(&["Root", "Helper"]));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].location.text(source),
            "pub type Spare = string"
        );
    }
}
