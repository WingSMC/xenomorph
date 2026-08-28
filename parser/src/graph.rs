use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use xenomorph_common::module::{types::ModuleDiagnostic, XenoRegistry};
use xenomorph_common::XenoDiagSeverity;

pub fn run_graph(args: &[String]) {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("Usage: xeno graph [--json]");
            std::process::exit(2);
        }
    };

    let registry = match XenoRegistry::load_workspace(false) {
        Ok(registry) => registry,
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprintln!("{}", format_graph_diagnostic(&diagnostic));
            }
            std::process::exit(1);
        }
    };
    let graph = ModuleGraph::from_registry(&registry);

    if json {
        match serde_json::to_writer_pretty(std::io::stdout().lock(), &graph) {
            Ok(()) => println!(),
            Err(error) => {
                eprintln!("Failed to serialize module graph: {error}");
                std::process::exit(1);
            }
        }
    } else {
        print!("{}", graph.to_cli_string());
    }
}

fn format_graph_diagnostic(diagnostic: &ModuleDiagnostic) -> String {
    let severity = match diagnostic.severity {
        XenoDiagSeverity::Err => "error",
        XenoDiagSeverity::Warn => "warning",
        XenoDiagSeverity::Info => "info",
    };
    let module_path = if diagnostic.module_path.is_empty() {
        "unknown module"
    } else {
        &diagnostic.module_path
    };

    format!("[{severity}] [{module_path}] {}", diagnostic.message)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphDiagnosticCounts {
    errors: usize,
    warnings: usize,
    infos: usize,
}

impl GraphDiagnosticCounts {
    fn add(&mut self, severity: XenoDiagSeverity) {
        match severity {
            XenoDiagSeverity::Err => self.errors += 1,
            XenoDiagSeverity::Warn => self.warnings += 1,
            XenoDiagSeverity::Info => self.infos += 1,
        }
    }

    fn total(self) -> usize {
        self.errors + self.warnings + self.infos
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleGraphNode {
    path: String,
    absolute_path: String,
    entry: bool,
    declarations: usize,
    diagnostics: GraphDiagnosticCounts,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleGraphEdge {
    importer: String,
    imported: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleGraph {
    schema_version: u32,
    workspace_root: String,
    entry: String,
    module_count: usize,
    import_count: usize,
    modules: Vec<ModuleGraphNode>,
    imports: Vec<ModuleGraphEdge>,
}

impl ModuleGraph {
    fn from_registry(registry: &XenoRegistry) -> Self {
        let cache = registry.module_cache.read().unwrap();
        let mut modules = cache
            .iter()
            .map(|(module_path, module)| {
                let mut diagnostics = GraphDiagnosticCounts::default();
                for diagnostic in module
                    .borrow_lexer_errors()
                    .iter()
                    .chain(module.borrow_parser_errors())
                    .chain(module.borrow_analyzer_errors())
                    .chain(module.borrow_module_errors())
                {
                    diagnostics.add(diagnostic.severity);
                }

                ModuleGraphNode {
                    path: module_path.clone(),
                    absolute_path: module.borrow_abs_path().to_string_lossy().into_owned(),
                    entry: module_path == &registry.entry,
                    declarations: module.borrow_declarations().len(),
                    diagnostics,
                }
            })
            .collect::<Vec<_>>();
        modules.sort_by(|left, right| left.path.cmp(&right.path));

        let imports = cache
            .iter()
            .flat_map(|(module_path, module)| {
                module
                    .borrow_imports()
                    .iter()
                    .map(|imported| ModuleGraphEdge {
                        importer: module_path.clone(),
                        imported: imported.clone(),
                    })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        Self {
            schema_version: 1,
            workspace_root: registry.root.to_string_lossy().into_owned(),
            entry: registry.entry.clone(),
            module_count: modules.len(),
            import_count: imports.len(),
            modules,
            imports,
        }
    }

    fn to_cli_string(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "Workspace: {}", self.workspace_root);
        let _ = writeln!(output, "Entry: {}", self.entry);
        let _ = writeln!(
            output,
            "Graph: {} module(s), {} import(s)",
            self.module_count, self.import_count
        );

        for module in &self.modules {
            let entry_marker = if module.entry { " [entry]" } else { "" };
            let _ = write!(output, "\n{}{}", module.path, entry_marker);
            if module.diagnostics.total() > 0 {
                let _ = write!(
                    output,
                    " ({} error(s), {} warning(s), {} info(s))",
                    module.diagnostics.errors,
                    module.diagnostics.warnings,
                    module.diagnostics.infos
                );
            }
            let _ = writeln!(output);

            let imported = self
                .imports
                .iter()
                .filter(|edge| edge.importer == module.path)
                .map(|edge| edge.imported.as_str())
                .collect::<Vec<_>>();
            if imported.is_empty() {
                let _ = writeln!(output, "  -> (none)");
            } else {
                for import in imported {
                    let _ = writeln!(output, "  -> {import}");
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_graph() -> ModuleGraph {
        ModuleGraph {
            schema_version: 1,
            workspace_root: "/workspace".to_string(),
            entry: "index".to_string(),
            module_count: 2,
            import_count: 1,
            modules: vec![
                ModuleGraphNode {
                    path: "index".to_string(),
                    absolute_path: "/workspace/index.xen".to_string(),
                    entry: true,
                    declarations: 1,
                    diagnostics: GraphDiagnosticCounts::default(),
                },
                ModuleGraphNode {
                    path: "models/user".to_string(),
                    absolute_path: "/workspace/models/user.xen".to_string(),
                    entry: false,
                    declarations: 2,
                    diagnostics: GraphDiagnosticCounts {
                        errors: 0,
                        warnings: 1,
                        infos: 0,
                    },
                },
            ],
            imports: vec![ModuleGraphEdge {
                importer: "index".to_string(),
                imported: "models/user".to_string(),
            }],
        }
    }

    #[test]
    fn cli_output_lists_directed_imports_and_leaf_modules() {
        assert_eq!(
            sample_graph().to_cli_string(),
            "Workspace: /workspace\nEntry: index\nGraph: 2 module(s), 1 import(s)\n\nindex [entry]\n  -> models/user\n\nmodels/user (0 error(s), 1 warning(s), 0 info(s))\n  -> (none)\n"
        );
    }

    #[test]
    fn json_output_uses_a_versioned_camel_case_contract() {
        let value = serde_json::to_value(sample_graph()).expect("graph should serialize");

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["workspaceRoot"], "/workspace");
        assert_eq!(value["modules"][0]["absolutePath"], "/workspace/index.xen");
        assert_eq!(value["imports"][0]["importer"], "index");
        assert_eq!(value["imports"][0]["imported"], "models/user");
    }
}
