mod config;
mod format;
mod graph;
mod init;
mod inspector;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use config::run_config;
use format::run_format;
use graph::run_graph;
use init::{run_graft, run_init};
use inspector::run_inspector;
use xenomorph_common::config::{write_rc_schema, Config, LogLevel, RC_SCHEMA_RELATIVE_PATH};
use xenomorph_common::module::{types::ModuleDiagnostic, XenoRegistry};
use xenomorph_common::plugins::XenoPlugin;
use xenomorph_common::XenoDiagSeverity;

/// Parse Xenomorph workspaces and run developer tools.
#[derive(Debug, Parser)]
#[command(
    name = "xeno",
    version,
    arg_required_else_help = true,
    infer_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize a new Xenomorph project in a Git repository.
    Init,
    /// Attach a Xenomorph Git repository as a submodule.
    Graft {
        /// URL of the Xenomorph Git repository to attach.
        #[arg(value_name = "REPO_URL")]
        repo_url: String,
    },
    /// Parse the configured workspace and run its generators.
    #[command(aliases = ["g"])]
    Generate,
    /// Generate the xenomorph.toml JSON Schema.
    Schema {
        /// Load a specific config, including an abstract config.
        #[arg(long, value_name = "FILE")]
        config: Option<PathBuf>,
    },
    /// Inspect the workspace configuration.
    Config {
        /// Print the resolved, deeply merged TOML configuration.
        #[arg(long, required = true)]
        inspect: bool,
    },
    /// Inspect standalone Xenomorph source from standard input as JSON.
    /// e.g. `cat my_module.xen | xeno inspect`
    Inspect,
    /// Print the configured workspace's module graph.
    Graph {
        /// Emit the versioned JSON representation instead of simple text.
        #[arg(long)]
        json: bool,
    },
    /// Format one Xenomorph file or all files in the configured workspace.
    Format {
        /// A .xen file to format; omit it to format the entire workspace.
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
    },
}

fn main() {
    match Cli::parse().command {
        Commands::Init => {
            if let Err(error) = run_init() {
                eprintln!("✗ {error}");
                std::process::exit(1);
            }
        }
        Commands::Graft { repo_url } => {
            if let Err(error) = run_graft(&repo_url) {
                eprintln!("✗ {error}");
                std::process::exit(1);
            }
        }
        Commands::Generate => run_generate(),
        Commands::Schema { config } => generate_rc_schema(config.as_deref()),
        Commands::Config { inspect } => {
            if let Err(error) = run_config(inspect) {
                eprintln!("✗ {error}");
                std::process::exit(1);
            }
        }
        Commands::Inspect => run_inspector(),
        Commands::Graph { json } => run_graph(json),
        Commands::Format { file } => match run_format(file.as_deref()) {
            Ok(summary) => println!(
                "✓ Formatted {} file(s); {} changed",
                summary.files, summary.changed
            ),
            Err(error) => {
                eprintln!("✗ {error}");
                std::process::exit(1);
            }
        },
    }
}

/// Generates the `xenomorph.toml` JSON Schema (base + plugin contributions) and
/// writes it beside the deepest config in the inheritance chain.
fn generate_rc_schema(config_path: Option<&std::path::Path>) {
    if let Some(config_path) = config_path {
        if let Err(error) = Config::initialize_from_path(config_path) {
            eprintln!("✗ Failed to load xenomorph.toml config: {error}");
            std::process::exit(1);
        }
    }
    let plugins = XenoPlugin::get_plugins();
    let out_path = Config::get().schema_workdir().join(RC_SCHEMA_RELATIVE_PATH);

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

fn run_generate() {
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
            .chain(module.borrow_collision_errors())
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
    use clap::CommandFactory;
    use xenomorph_common::module::types::ErrorPhase;

    #[test]
    fn clap_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn clap_parses_graph_json_option() {
        let cli =
            Cli::try_parse_from(["xeno", "graph", "--json"]).expect("graph arguments should parse");

        assert!(matches!(cli.command, Commands::Graph { json: true }));
    }

    #[test]
    fn clap_parses_generate_command() {
        let cli = Cli::try_parse_from(["xeno", "generate"]).expect("generate command should parse");

        assert!(matches!(cli.command, Commands::Generate));
    }

    #[test]
    fn clap_parses_init_command() {
        let cli = Cli::try_parse_from(["xeno", "init"]).expect("init command should parse");

        assert!(matches!(cli.command, Commands::Init));
    }

    #[test]
    fn clap_parses_graft_repository_url() {
        let cli =
            Cli::try_parse_from(["xeno", "graft", "https://example.com/team/tda-schemas.git"])
                .expect("graft command should parse");

        assert!(matches!(
            cli.command,
            Commands::Graft { repo_url }
                if repo_url == "https://example.com/team/tda-schemas.git"
        ));
    }

    #[test]
    fn clap_parses_schema_config_path() {
        let cli = Cli::try_parse_from([
            "xeno",
            "schema",
            "--config",
            "schemas/linked/xenomorph.toml",
        ])
        .expect("schema config path should parse");

        assert!(matches!(
            cli.command,
            Commands::Schema { config: Some(path) }
                if path == PathBuf::from("schemas/linked/xenomorph.toml")
        ));
    }

    #[test]
    fn clap_parses_config_inspect_option() {
        let cli = Cli::try_parse_from(["xeno", "config", "--inspect"])
            .expect("config inspection arguments should parse");

        assert!(matches!(cli.command, Commands::Config { .. }));
    }

    #[test]
    fn config_command_requires_inspect_option() {
        let error = Cli::try_parse_from(["xeno", "config"])
            .expect_err("config command without an operation should fail");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn clap_shows_help_when_no_command_is_provided() {
        let error = Cli::try_parse_from(["xeno"]).expect_err("a command should be required");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert!(error.to_string().contains("Usage: xeno <COMMAND>"));
    }

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
