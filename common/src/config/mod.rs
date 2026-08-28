use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::{collections::HashMap, path::Path};

use crate::XenoDiagSeverity;

pub mod schema;
pub use schema::{build_rc_schema, write_rc_schema, RC_SCHEMA_RELATIVE_PATH};

static CONFIG: OnceLock<Config> = OnceLock::new();

#[repr(Rust)]
#[derive(Deserialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub parser: ParserConfig,

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
    "index".to_string()
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
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            entry: default_parser_path(),
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

fn find_workspace_root(wd: &Path) -> Option<PathBuf> {
    let mut current_dir = wd.to_path_buf();

    loop {
        let config_path = current_dir.join("xenomorph.toml");
        if config_path.exists() {
            return Some(current_dir);
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

    match find_workspace_root(&current_dir) {
        None => Config::default_with_workdir(current_dir),
        Some(workdir) => {
            let content = match fs::read_to_string(workdir.join("xenomorph.toml")) {
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
    use super::{Config, LogLevel};
    use crate::XenoDiagSeverity;

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
}
