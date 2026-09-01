use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::{collections::HashMap, path::Path};

use crate::XenoDiagSeverity;

pub mod schema;
pub mod watcher;
pub use schema::{build_rc_schema, write_rc_schema, RC_SCHEMA_RELATIVE_PATH};
pub use watcher::WorkspaceConfigWatcher;

pub const WORKSPACE_CONFIG_FILE: &str = "xenomorph.toml";

static CONFIG: OnceLock<Config> = OnceLock::new();

#[repr(Rust)]
#[derive(Deserialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub parser: ParserConfig,

    #[serde(default)]
    pub formatter: FormatterConfig,

    #[serde(default)]
    pub plugins: PluginsConfig,

    #[serde(default)]
    pub debug: DebugConfig,

    #[serde(default = "default_workdir")]
    pub workdir: PathBuf,
}
#[repr(Rust)]
#[derive(Deserialize, Debug, Clone)]
pub struct ParserConfig {
    #[serde(default = "default_parser_path")]
    pub entry: String,
}

#[repr(Rust)]
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FormatterConfig {
    #[serde(default)]
    pub indent_kind: IndentKind,

    #[serde(default = "default_indent_width")]
    pub indent_width: usize,

    #[serde(default = "default_max_line_length")]
    pub max_line_length: usize,

    #[serde(default)]
    pub line_ending: LineEnding,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentKind {
    #[default]
    #[serde(rename = "space", alias = "spaces")]
    Space,
    #[serde(rename = "tab", alias = "tabs")]
    Tab,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
    Auto,
}

/// Re-export for plugins to use without adding toml as a direct dependency.
pub use toml::Value as ConfigValue;
pub type PluginConfigs = HashMap<String, ConfigValue>;

#[repr(Rust)]
#[derive(Deserialize, Debug, Clone)]
pub struct PluginsConfig {
    #[serde(default = "default_plugins_path")]
    pub path: String,

    #[serde(default = "default_plugins_list")]
    pub plugins: Vec<String>,

    /// Per-plugin configuration sections, e.g. `[plugins.typescript]`.
    /// Plugins can read their own config and other plugins' configs.
    #[serde(flatten)]
    pub config: PluginConfigs,
}
#[repr(Rust)]
#[derive(Deserialize, Debug, Clone, Default)]
pub struct DebugConfig {
    #[serde(default)]
    pub plugins: bool,

    #[serde(default)]
    pub tokens: bool,

    #[serde(default)]
    pub ast: bool,

    #[serde(default)]
    pub loglevel: LogLevel,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warning,
    #[default]
    Info,
}

impl LogLevel {
    pub fn allows(self, severity: XenoDiagSeverity) -> bool {
        match self {
            Self::Error => severity == XenoDiagSeverity::Err,
            Self::Warning => severity != XenoDiagSeverity::Info,
            Self::Info => true,
        }
    }
}

fn default_parser_path() -> String {
    "xeno/index".to_string()
}
fn default_indent_width() -> usize {
    4
}
fn default_max_line_length() -> usize {
    80
}
fn default_plugins_path() -> String {
    "".to_string()
}
fn default_plugins_list() -> Vec<String> {
    vec![]
}
fn default_workdir() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

impl Config {
    pub fn default_with_workdir(workdir: PathBuf) -> Self {
        Self {
            workdir,
            ..Default::default()
        }
    }

    pub fn get() -> &'static Config {
        CONFIG.get_or_init(init_config)
    }

    pub fn workspace_config_path(&self) -> PathBuf {
        self.workdir.join(WORKSPACE_CONFIG_FILE)
    }
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            entry: default_parser_path(),
        }
    }
}
impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            indent_kind: IndentKind::default(),
            indent_width: default_indent_width(),
            max_line_length: default_max_line_length(),
            line_ending: LineEnding::default(),
        }
    }
}
impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            path: default_plugins_path(),
            plugins: default_plugins_list(),
            config: HashMap::new(),
        }
    }
}

pub fn find_workspace_config(wd: &Path) -> Option<PathBuf> {
    let mut current_dir = wd.to_path_buf();

    loop {
        let config_path = current_dir.join(WORKSPACE_CONFIG_FILE);
        if config_path.exists() {
            return Some(config_path);
        }

        if !current_dir.pop() {
            return None;
        }
    }
}

fn init_config() -> Config {
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error: Unable to get current directory.");
            return Config::default();
        }
    };

    match find_workspace_config(&current_dir) {
        None => Config::default_with_workdir(current_dir),
        Some(config_path) => {
            let workdir = config_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| current_dir.clone());
            let content = match fs::read_to_string(&config_path) {
                Ok(content) => content,
                Err(_) => {
                    eprintln!("Error: Unable to read config file.");
                    return Config::default_with_workdir(workdir);
                }
            };

            match toml::de::from_str::<Config>(&content) {
                Ok(mut config) => {
                    config.workdir = workdir;
                    config
                }
                Err(_) => {
                    eprintln!("Error: Unable to parse config file.");
                    Config::default_with_workdir(workdir)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_workspace_config, Config, FormatterConfig, IndentKind, LineEnding, LogLevel,
        WORKSPACE_CONFIG_FILE,
    };
    use crate::XenoDiagSeverity;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn formatter_uses_stable_defaults() {
        let config: Config = toml::from_str("").expect("empty config should use defaults");

        assert_eq!(config.formatter, FormatterConfig::default());
        assert_eq!(config.formatter.indent_kind, IndentKind::Space);
        assert_eq!(config.formatter.indent_width, 4);
        assert_eq!(config.formatter.max_line_length, 100);
        assert_eq!(config.formatter.line_ending, LineEnding::Lf);
    }

    #[test]
    fn formatter_deserializes_all_options() {
        let config: Config = toml::from_str(
            "[formatter]\nindent_kind = \"tab\"\nindent_width = 8\nmax_line_length = 120\nline_ending = \"crlf\"\n",
        )
        .expect("formatter options should deserialize");

        assert_eq!(config.formatter.indent_kind, IndentKind::Tab);
        assert_eq!(config.formatter.indent_width, 8);
        assert_eq!(config.formatter.max_line_length, 120);
        assert_eq!(config.formatter.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn loglevel_defaults_to_info() {
        let config: Config = toml::from_str("").expect("empty config should use defaults");

        assert_eq!(config.debug.loglevel, LogLevel::Info);
    }

    #[test]
    fn loglevel_deserializes_supported_values() {
        for (value, expected) in [
            ("error", LogLevel::Error),
            ("warning", LogLevel::Warning),
            ("info", LogLevel::Info),
        ] {
            let config: Config = toml::from_str(&format!("[debug]\nloglevel = \"{value}\"\n"))
                .expect("supported loglevel should deserialize");

            assert_eq!(config.debug.loglevel, expected);
        }
    }

    #[test]
    fn loglevel_filters_diagnostic_severities() {
        assert!(LogLevel::Error.allows(XenoDiagSeverity::Err));
        assert!(!LogLevel::Error.allows(XenoDiagSeverity::Warn));
        assert!(!LogLevel::Error.allows(XenoDiagSeverity::Info));

        assert!(LogLevel::Warning.allows(XenoDiagSeverity::Err));
        assert!(LogLevel::Warning.allows(XenoDiagSeverity::Warn));
        assert!(!LogLevel::Warning.allows(XenoDiagSeverity::Info));

        assert!(LogLevel::Info.allows(XenoDiagSeverity::Err));
        assert!(LogLevel::Info.allows(XenoDiagSeverity::Warn));
        assert!(LogLevel::Info.allows(XenoDiagSeverity::Info));
    }

    #[test]
    fn workspace_config_path_uses_config_workdir() {
        let workdir = std::path::PathBuf::from("workspace");
        let config = Config::default_with_workdir(workdir.clone());

        assert_eq!(
            config.workspace_config_path(),
            workdir.join(WORKSPACE_CONFIG_FILE)
        );
    }

    #[test]
    fn workspace_config_discovery_returns_the_nearest_config_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("xenomorph-config-{unique}"));
        let nested = root.join("src").join("models");
        let config_path = root.join(WORKSPACE_CONFIG_FILE);
        fs::create_dir_all(&nested).expect("test directory should be created");
        fs::write(&config_path, "").expect("test config should be written");

        assert_eq!(find_workspace_config(&nested), Some(config_path));

        fs::remove_dir_all(root).expect("test directory should be removed");
    }
}
