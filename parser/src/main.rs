mod inspector;

use inspector::run_inspector;
use xenomorph_common::config::{write_rc_schema, Config, LogLevel, RC_SCHEMA_RELATIVE_PATH};
use xenomorph_common::module::{types::ModuleDiagnostic, XenoRegistry};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::XenoDiagSeverity;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("schema") => generate_rc_schema(),
        Some("inspect") => run_inspector(),
        _ => run_parser(),
    }
}

/// Generates the `xenomorph.toml` JSON Schema (base + plugin contributions) and
/// writes it to `.xenomorph/xenomorph.schema.json` in the workspace root.
fn generate_rc_schema() {
    let plugins = XenoPlugin::get_plugins();
    let out_path = Config::get().workdir.join(RC_SCHEMA_RELATIVE_PATH);

    match write_rc_schema(plugins, &out_path) {
        Ok(()) => println!("✓ Wrote xenomorph.toml schema → {}", out_path.display()),
        Err(e) => {
            eprintln!("✗ Failed to write xenomorph.toml schema: {}", e);
            std::process::exit(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DiagnosticCounts {
    errors: usize,
    warnings: usize,
    infos: usize,
}

impl DiagnosticCounts {
    fn add(&mut self, severity: XenoDiagSeverity) {
        match severity {
            XenoDiagSeverity::Err => self.errors += 1,
            XenoDiagSeverity::Warn => self.warnings += 1,
            XenoDiagSeverity::Info => self.infos += 1,
        }
    }

    fn include(&mut self, other: Self) {
        self.errors += other.errors;
        self.warnings += other.warnings;
        self.infos += other.infos;
    }

    fn is_empty(self) -> bool {
        self.errors == 0 && self.warnings == 0 && self.infos == 0
    }

    fn marker(self) -> &'static str {
        if self.errors > 0 {
            "✗"
        } else {
            "✓"
        }
    }

    fn summary(self, loglevel: LogLevel) -> String {
        let mut parts = vec![format!("{} error(s)", self.errors)];
        if loglevel != LogLevel::Error {
            parts.push(format!("{} warning(s)", self.warnings));
        }
        if loglevel == LogLevel::Info {
            parts.push(format!("{} info(s)", self.infos));
        }
        parts.join(", ")
    }
}

fn count_diagnostics<'a>(
    diagnostics: impl IntoIterator<Item = &'a ModuleDiagnostic>,
) -> DiagnosticCounts {
    let mut counts = DiagnosticCounts::default();
    for diagnostic in diagnostics {
        counts.add(diagnostic.severity);
    }
    counts
}

fn severity_name(severity: XenoDiagSeverity) -> &'static str {
    match severity {
        XenoDiagSeverity::Err => "error",
        XenoDiagSeverity::Warn => "warning",
        XenoDiagSeverity::Info => "info",
    }
}

fn format_cli_diagnostic(diagnostic: &ModuleDiagnostic) -> String {
    let module_path = if diagnostic.module_path.is_empty() {
        "unknown module"
    } else {
        &diagnostic.module_path
    };

    format!(
        "[{}] [{}] {}",
        severity_name(diagnostic.severity),
        module_path,
        diagnostic.message
    )
}

fn run_parser() {
    let loglevel = Config::get().debug.loglevel;
    let reg = match XenoRegistry::load_workspace(true) {
        Ok(r) => r,
        Err(e) => {
            for diagnostic in e
                .iter()
                .filter(|diagnostic| loglevel.allows(diagnostic.severity))
            {
                eprintln!("{}", format_cli_diagnostic(diagnostic));
            }
            std::process::exit(1);
        }
    };

    let cache = reg.module_cache.read().unwrap();
    let module_count = cache.len();
    let mut total_counts = DiagnosticCounts::default();

    for module in cache.values() {
        let path = module.borrow_module_path();
        let decl_count = module.borrow_declarations().len();
        let diagnostics: Vec<_> = module
            .borrow_analyzer_errors()
            .iter()
            .chain(module.borrow_parser_errors())
            .chain(module.borrow_lexer_errors())
            .chain(module.borrow_module_errors())
            .filter(|diagnostic| loglevel.allows(diagnostic.severity))
            .collect();
        let counts = count_diagnostics(diagnostics.iter().copied());
        total_counts.include(counts);

        if counts.is_empty() {
            println!("✓ {} ({} declarations)", path, decl_count);
        } else {
            eprintln!(
                "{} {} ({} declarations; {})",
                counts.marker(),
                path,
                decl_count,
                counts.summary(loglevel)
            );
            for diagnostic in diagnostics {
                eprintln!("  └ {}", format_cli_diagnostic(diagnostic));
            }
        }
    }

    println!(
        "\n{} module(s) processed, {}",
        module_count,
        total_counts.summary(loglevel)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::module::types::ErrorPhase;

    #[test]
    fn cli_diagnostic_format_includes_severity_and_module() {
        let diagnostic = ModuleDiagnostic {
            module_path: "models/user".to_string(),
            message: "Unknown annotation '@example'".to_string(),
            location: None,
            phase: ErrorPhase::Analyzer,
            severity: XenoDiagSeverity::Warn,
        };

        assert_eq!(
            format_cli_diagnostic(&diagnostic),
            "[warning] [models/user] Unknown annotation '@example'"
        );
    }

    #[test]
    fn cli_diagnostic_format_has_a_module_fallback() {
        let diagnostic = ModuleDiagnostic {
            module_path: String::new(),
            message: "diagnostic".to_string(),
            location: None,
            phase: ErrorPhase::Module,
            severity: XenoDiagSeverity::Err,
        };

        assert_eq!(
            format_cli_diagnostic(&diagnostic),
            "[error] [unknown module] diagnostic"
        );
    }

    #[test]
    fn diagnostic_summary_matches_loglevel() {
        let counts = DiagnosticCounts {
            errors: 1,
            warnings: 2,
            infos: 3,
        };

        assert_eq!(counts.summary(LogLevel::Error), "1 error(s)");
        assert_eq!(
            counts.summary(LogLevel::Warning),
            "1 error(s), 2 warning(s)"
        );
        assert_eq!(
            counts.summary(LogLevel::Info),
            "1 error(s), 2 warning(s), 3 info(s)"
        );
    }

    #[test]
    fn only_errors_use_the_failure_marker() {
        assert_eq!(
            DiagnosticCounts {
                errors: 1,
                warnings: 0,
                infos: 0,
            }
            .marker(),
            "✗"
        );
        assert_eq!(
            DiagnosticCounts {
                errors: 0,
                warnings: 1,
                infos: 1,
            }
            .marker(),
            "✓"
        );
    }
}
