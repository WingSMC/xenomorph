use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[repr(Rust)]
#[derive(Deserialize, Debug, Clone)]
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
    pub path: String,
}
#[repr(Rust)]
#[derive(Deserialize, Debug, Clone)]
pub struct PluginsConfig {
    #[serde(default = "default_plugins_path")]
    pub path: String,

    #[serde(default = "default_plugins_list")]
    pub plugins: Vec<String>,
}
#[repr(Rust)]
#[derive(Deserialize, Debug, Clone)]
pub struct DebugConfig {
    #[serde(default)]
    pub plugins: bool,

    #[serde(default)]
    pub tokens: bool,

    #[serde(default)]
    pub ast: bool,
}

fn default_parser_path() -> String {
    "./tests/examples/parser/p1.xen".to_string()
}
fn default_plugins_path() -> String {
    "./target/release/".to_string()
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

impl Default for Config {
    fn default() -> Self {
        Self {
            parser: ParserConfig::default(),
            plugins: PluginsConfig::default(),
            debug: DebugConfig::default(),
            workdir: PathBuf::default(),
        }
    }
}
impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            path: default_parser_path(),
        }
    }
}
impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            plugins: false,
            tokens: false,
            ast: false,
        }
    }
}
impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            path: default_plugins_path(),
            plugins: default_plugins_list(),
        }
    }
}

fn find_workspace_root(wd: &PathBuf) -> Option<PathBuf> {
    let mut current_dir = wd.clone();

    loop {
        let config_path = current_dir.join(".xenomorphrc");
        if config_path.exists() {
            return Some(current_dir);
        }

        if !current_dir.pop() {
            break;
        }
    }

    None
}

pub fn init_config() -> Config {
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
            let content = match fs::read_to_string(workdir.join(".xenomorphrc")) {
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
                    return Config::default_with_workdir(workdir);
                }
            }
        }
    }
}
