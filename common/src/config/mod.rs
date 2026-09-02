use serde::{Deserialize, Serialize};
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
pub(crate) const DEFAULT_CONFIG_IS_ABSTRACT: bool = false;
const INITIALIZED_CONFIG_IS_ABSTRACT: bool = true;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[repr(Rust)]
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub parser: ParserConfig,

    #[serde(default)]
    pub formatter: FormatterConfig,

    #[serde(default)]
    pub plugins: PluginsConfig,

    #[serde(default)]
    pub debug: DebugConfig,

    #[serde(default = "default_workdir", skip_serializing)]
    pub workdir: PathBuf,

    /// The non-abstract config selected by directory discovery.
    #[serde(skip)]
    config_path: Option<PathBuf>,

    /// The deepest config in the `extends` chain. Generated editor metadata
    /// belongs beside this reusable project config.
    #[serde(skip)]
    schema_config_path: Option<PathBuf>,

    /// Every config contributing to the effective configuration, ordered from
    /// the authoritative config to its deepest base.
    #[serde(skip)]
    config_paths: Vec<PathBuf>,
}
#[repr(Rust)]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ParserConfig {
    #[serde(default = "default_parser_path")]
    pub entry: String,
}

#[repr(Rust)]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
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

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentKind {
    #[default]
    #[serde(rename = "space", alias = "spaces")]
    Space,
    #[serde(rename = "tab", alias = "tabs")]
    Tab,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Deserialize, Serialize, Debug, Clone)]
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
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
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

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    /// Initializes the process-wide configuration from an explicit file,
    /// bypassing directory discovery so abstract configs can be operated on
    /// directly by commands such as `xeno schema --config`.
    pub fn initialize_from_path(config_path: &Path) -> Result<(), String> {
        let config = load_config(config_path)?;
        CONFIG
            .set(config)
            .map_err(|_| "Xenomorph configuration is already initialized.".to_string())
    }

    pub fn workspace_config_path(&self) -> PathBuf {
        self.config_path
            .clone()
            .unwrap_or_else(|| self.workdir.join(WORKSPACE_CONFIG_FILE))
    }

    /// Returns every file that contributes to this effective configuration.
    pub fn workspace_config_paths(&self) -> Vec<PathBuf> {
        if self.config_paths.is_empty() {
            vec![self.workspace_config_path()]
        } else {
            self.config_paths.clone()
        }
    }

    /// Returns the directory where `.xenomorph` metadata must be generated.
    /// This is the deepest extended config's directory, or the authoritative
    /// config's directory when the config is standalone.
    pub fn schema_workdir(&self) -> &Path {
        self.schema_config_path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(&self.workdir)
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

#[derive(Serialize)]
struct DefaultConfigDocument {
    #[serde(rename = "abstract")]
    is_abstract: bool,

    formatter: FormatterConfig,
    plugins: PluginsConfig,
    debug: DebugConfig,
}

#[derive(Serialize)]
struct GraftConfigDocument {
    extends: String,
    parser: ParserConfig,
    plugins: PluginsConfig,
}

/// Serializes an abstract `xenomorph.toml` containing the canonical shared
/// defaults and its editor schema directive. The project-specific parser entry
/// is intentionally omitted.
pub fn default_config_toml() -> Result<String, String> {
    let defaults = Config::default();
    let document = DefaultConfigDocument {
        is_abstract: INITIALIZED_CONFIG_IS_ABSTRACT,
        formatter: defaults.formatter,
        plugins: defaults.plugins,
        debug: defaults.debug,
    };
    let config = serialize_config_toml(&document)?;

    Ok(format!(
        "#:schema ./{}\n\n{config}",
        RC_SCHEMA_RELATIVE_PATH
    ))
}

/// Serializes the authoritative config that links the current project to a
/// grafted Xenomorph project. Parser and plugin settings use their canonical
/// defaults so the current project can override the grafted config explicitly.
pub fn graft_config_toml(grafted_project: &Path, entry_module: &Path) -> Result<String, String> {
    let grafted_project = grafted_project.to_string_lossy().replace('\\', "/");
    let entry_module = entry_module.to_string_lossy().replace('\\', "/");
    let document = GraftConfigDocument {
        extends: format!("./{grafted_project}/{WORKSPACE_CONFIG_FILE}"),
        parser: ParserConfig {
            entry: format!("{grafted_project}/{entry_module}"),
        },
        plugins: PluginsConfig::default(),
    };
    let config = serialize_config_toml(&document)?;

    Ok(format!(
        "#:schema ./{grafted_project}/{RC_SCHEMA_RELATIVE_PATH}\n\n{config}"
    ))
}

fn serialize_config_toml(config: &impl Serialize) -> Result<String, String> {
    let mut output = toml::to_string_pretty(config)
        .map_err(|error| format!("Unable to serialize configuration: {error}"))?;
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

pub fn find_workspace_config(wd: &Path) -> Option<PathBuf> {
    let mut current_dir = wd.to_path_buf();

    loop {
        let config_path = current_dir.join(WORKSPACE_CONFIG_FILE);
        if config_path.is_file() && !config_is_abstract(&config_path) {
            return Some(config_path);
        }

        if !current_dir.pop() {
            return None;
        }
    }
}

/// `abstract` is local discovery metadata: it directs tools to keep walking
/// upward, but does not stop another config from explicitly extending this
/// file. An unreadable or malformed config remains authoritative so its error
/// is not silently hidden by a parent config.
fn config_is_abstract(config_path: &Path) -> bool {
    fs::read_to_string(config_path)
        .ok()
        .and_then(|content| toml::from_str::<ConfigValue>(&content).ok())
        .and_then(|value| value.get("abstract").and_then(ConfigValue::as_bool))
        .unwrap_or(false)
}

fn merge_config_values(base: &mut ConfigValue, overriding: ConfigValue) {
    match (base, overriding) {
        (ConfigValue::Table(base), ConfigValue::Table(overriding)) => {
            for (key, value) in overriding {
                match base.get_mut(&key) {
                    Some(existing) => merge_config_values(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overriding) => *base = overriding,
    }
}

fn normalized_config_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "Unable to resolve config file '{}': {error}",
            path.display()
        )
    })?;

    #[cfg(windows)]
    {
        let value = canonical.to_string_lossy();
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        if let Some(local) = value.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(local));
        }
    }

    Ok(canonical)
}

fn load_config_value(
    config_path: &Path,
    resolving: &mut Vec<PathBuf>,
    config_paths: &mut Vec<PathBuf>,
) -> Result<(ConfigValue, PathBuf), String> {
    let config_path = normalized_config_path(config_path)?;
    if let Some(cycle_start) = resolving.iter().position(|path| path == &config_path) {
        let mut cycle = resolving[cycle_start..]
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(config_path.display().to_string());
        return Err(format!(
            "Cyclic xenomorph.toml inheritance: {}",
            cycle.join(" -> ")
        ));
    }

    resolving.push(config_path.clone());
    config_paths.push(config_path.clone());

    let result = (|| {
        let content = fs::read_to_string(&config_path).map_err(|error| {
            format!(
                "Unable to read config file '{}': {error}",
                config_path.display()
            )
        })?;
        let mut current = toml::from_str::<ConfigValue>(&content).map_err(|error| {
            format!(
                "Unable to parse config file '{}': {error}",
                config_path.display()
            )
        })?;
        let table = current.as_table_mut().ok_or_else(|| {
            format!(
                "Config file '{}' must contain a TOML table",
                config_path.display()
            )
        })?;
        let extends = match table.remove("extends") {
            Some(ConfigValue::String(path)) => Some(path),
            Some(_) => {
                return Err(format!(
                    "Config key 'extends' in '{}' must be a string",
                    config_path.display()
                ))
            }
            None => None,
        };
        // Discovery metadata is deliberately file-local and is not inherited.
        table.remove("abstract");

        let Some(extends) = extends else {
            return Ok((current, config_path.clone()));
        };
        let parent = config_path
            .parent()
            .ok_or_else(|| format!("Config path '{}' has no parent", config_path.display()))?;
        let (mut base, schema_config_path) =
            load_config_value(&parent.join(extends), resolving, config_paths)?;
        merge_config_values(&mut base, current);
        Ok((base, schema_config_path))
    })();

    resolving.pop();
    result
}

fn load_config(config_path: &Path) -> Result<Config, String> {
    let authoritative_path = normalized_config_path(config_path)?;
    let workdir = authoritative_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "Config path '{}' has no parent directory",
                authoritative_path.display()
            )
        })?;
    let mut config_paths = Vec::new();
    let (value, schema_config_path) =
        load_config_value(&authoritative_path, &mut Vec::new(), &mut config_paths)?;
    let mut config = value.try_into::<Config>().map_err(|error| {
        format!(
            "Unable to deserialize merged config '{}': {error}",
            authoritative_path.display()
        )
    })?;
    config.workdir = workdir;
    config.config_path = Some(authoritative_path);
    config.schema_config_path = Some(schema_config_path);
    config.config_paths = config_paths;
    Ok(config)
}

/// Discovers and resolves the authoritative config for `wd`, then serializes
/// the complete effective configuration as TOML. Hierarchy control keys are
/// consumed during resolution, defaults are materialized, and derived runtime
/// paths are not added.
pub fn inspect_merged_config(wd: &Path) -> Result<String, String> {
    let config_path = find_workspace_config(wd).ok_or_else(|| {
        format!(
            "Unable to find a non-abstract '{}' from '{}' or any parent directory",
            WORKSPACE_CONFIG_FILE,
            wd.display()
        )
    })?;
    let config = load_config(&config_path)?;
    serialize_config_toml(&config)
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
        Some(config_path) => load_config(&config_path).unwrap_or_else(|error| {
            eprintln!("Error: {error}");
            let workdir = config_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(current_dir);
            Config::default_with_workdir(workdir)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_config_toml, find_workspace_config, graft_config_toml, inspect_merged_config,
        load_config, Config, ConfigValue, FormatterConfig, IndentKind, LineEnding, LogLevel,
        WORKSPACE_CONFIG_FILE,
    };
    use crate::XenoDiagSeverity;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn formatter_uses_stable_defaults() {
        let config: Config = toml::from_str("").expect("empty config should use defaults");

        assert_eq!(config.formatter, FormatterConfig::default());
        assert_eq!(config.formatter.indent_kind, IndentKind::Space);
        assert_eq!(config.formatter.indent_width, 4);
        assert_eq!(config.formatter.max_line_length, 80);
        assert_eq!(config.formatter.line_ending, LineEnding::Lf);
    }

    #[test]
    fn generated_default_config_uses_canonical_runtime_defaults() {
        let output = default_config_toml().expect("default config should serialize");
        let value: ConfigValue = toml::from_str(&output).expect("default config should be TOML");
        let generated: Config = toml::from_str(&output).expect("default config should load");
        let expected = Config::default();

        assert!(output.starts_with("#:schema ./.xenomorph/xenomorph.schema.json\n\n"));
        assert!(output.ends_with('\n'));
        assert!(value["abstract"].as_bool().unwrap_or_default());
        assert!(value.get("workdir").is_none());
        assert!(value.get("parser").is_none());
        assert_eq!(generated.parser.entry, expected.parser.entry);
        assert_eq!(generated.formatter, expected.formatter);
        assert_eq!(generated.plugins.path, expected.plugins.path);
        assert_eq!(generated.plugins.plugins, expected.plugins.plugins);
        assert_eq!(generated.debug.plugins, expected.debug.plugins);
        assert_eq!(generated.debug.tokens, expected.debug.tokens);
        assert_eq!(generated.debug.ast, expected.debug.ast);
        assert_eq!(generated.debug.loglevel, expected.debug.loglevel);
    }

    #[test]
    fn graft_config_uses_linked_schema_and_canonical_overrides() {
        let output =
            graft_config_toml(Path::new("schemas/linked-schema"), Path::new("models/root"))
                .expect("graft config should serialize");
        let value: ConfigValue = toml::from_str(&output).expect("graft config should be TOML");
        let defaults = ConfigValue::try_from(Config::default())
            .expect("runtime config defaults should serialize");

        assert!(output
            .starts_with("#:schema ./schemas/linked-schema/.xenomorph/xenomorph.schema.json\n\n"));
        assert_eq!(
            value["extends"].as_str(),
            Some("./schemas/linked-schema/xenomorph.toml")
        );
        assert_eq!(
            value["parser"]["entry"].as_str(),
            Some("schemas/linked-schema/models/root")
        );
        assert_eq!(value["plugins"], defaults["plugins"]);
        assert!(value.get("formatter").is_none());
        assert!(value.get("debug").is_none());
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

    fn temporary_directory(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("xenomorph-{name}-{unique}"));
        fs::create_dir_all(&root).expect("test directory should be created");
        root
    }

    #[test]
    fn workspace_config_discovery_skips_abstract_configs() {
        let root = temporary_directory("abstract-discovery");
        let inner = root.join("shared");
        fs::create_dir_all(&inner).expect("inner directory should be created");
        let authoritative = root.join(WORKSPACE_CONFIG_FILE);
        fs::write(&authoritative, "[parser]\nentry = \"backend\"\n")
            .expect("authoritative config should be written");
        fs::write(inner.join(WORKSPACE_CONFIG_FILE), "abstract = true\n")
            .expect("abstract config should be written");

        assert_eq!(find_workspace_config(&inner), Some(authoritative));

        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn standalone_abstract_config_is_not_authoritative() {
        let root = temporary_directory("standalone-abstract");
        fs::write(root.join(WORKSPACE_CONFIG_FILE), "abstract = true\n")
            .expect("abstract config should be written");

        assert_eq!(find_workspace_config(&root), None);

        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn extends_deep_merges_tables_and_uses_outer_precedence() {
        let root = temporary_directory("extends-merge");
        let inner = root.join("shared");
        fs::create_dir_all(&inner).expect("inner directory should be created");
        let inner_config = inner.join(WORKSPACE_CONFIG_FILE);
        let outer_config = root.join(WORKSPACE_CONFIG_FILE);
        fs::write(
            &inner_config,
            concat!(
                "abstract = true\n",
                "[parser]\nentry = \"ui\"\n",
                "[formatter]\nindent_width = 2\nline_ending = \"crlf\"\n",
                "[plugins.typescript]\noutput = \"./generated/ts\"\n",
                "[plugins.java]\npackage = \"example.base\"\ndata = true\n",
            ),
        )
        .expect("inner config should be written");
        fs::write(
            &outer_config,
            concat!(
                "extends = \"./shared/xenomorph.toml\"\n",
                "[parser]\nentry = \"backend\"\n",
                "[formatter]\nindent_width = 4\n",
                "[plugins]\nplugins = [\"xenomorph_typescript\"]\n",
                "[plugins.java]\npackage = \"example.service\"\n",
            ),
        )
        .expect("outer config should be written");

        let config = load_config(&outer_config).expect("config hierarchy should load");

        assert_eq!(config.parser.entry, "backend");
        assert_eq!(config.formatter.indent_width, 4);
        assert_eq!(config.formatter.line_ending, LineEnding::Crlf);
        assert_eq!(config.plugins.plugins, vec!["xenomorph_typescript"]);
        assert_eq!(
            config.plugins.config["typescript"]["output"].as_str(),
            Some("./generated/ts")
        );
        assert_eq!(
            config.plugins.config["java"]["package"].as_str(),
            Some("example.service")
        );
        assert_eq!(config.plugins.config["java"]["data"].as_bool(), Some(true));
        assert_eq!(config.workdir, root);
        assert_eq!(config.workspace_config_path(), outer_config);
        assert_eq!(config.schema_workdir(), inner.as_path());
        assert_eq!(
            config.workspace_config_paths(),
            vec![outer_config, inner_config]
        );

        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn extends_rejects_inheritance_cycles() {
        let root = temporary_directory("extends-cycle");
        let first = root.join("first.toml");
        let second = root.join("second.toml");
        fs::write(&first, "extends = \"./second.toml\"\n").expect("first config should be written");
        fs::write(&second, "extends = \"./first.toml\"\n")
            .expect("second config should be written");

        let error = load_config(&first).expect_err("inheritance cycle should fail");
        assert!(error.contains("Cyclic xenomorph.toml inheritance"));
        assert!(error.contains("first.toml"));
        assert!(error.contains("second.toml"));

        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn config_inspection_outputs_the_complete_effective_configuration() {
        let root = temporary_directory("inspect-merged");
        let inner = root.join("shared");
        fs::create_dir_all(&inner).expect("inner directory should be created");
        fs::write(
            inner.join(WORKSPACE_CONFIG_FILE),
            concat!(
                "abstract = true\n",
                "[formatter]\nindent_width = 2\nline_ending = \"crlf\"\n",
                "[plugins.java]\ndata = true\npackage = \"example.base\"\n",
            ),
        )
        .expect("inner config should be written");
        fs::write(
            root.join(WORKSPACE_CONFIG_FILE),
            concat!(
                "extends = \"./shared/xenomorph.toml\"\n",
                "[formatter]\nindent_width = 4\n",
                "[plugins.java]\npackage = \"example.service\"\n",
            ),
        )
        .expect("outer config should be written");

        let output = inspect_merged_config(&inner).expect("merged config should serialize");
        let value: ConfigValue = toml::from_str(&output).expect("output should be valid TOML");
        let defaults = ConfigValue::try_from(Config::default())
            .expect("runtime config defaults should serialize");

        assert!(output.ends_with('\n'));
        assert!(value.get("abstract").is_none());
        assert!(value.get("extends").is_none());
        assert_eq!(value["parser"], defaults["parser"]);
        assert_eq!(value["formatter"]["indent_width"].as_integer(), Some(4));
        assert_eq!(value["formatter"]["line_ending"].as_str(), Some("crlf"));
        assert_eq!(
            value["formatter"]["max_line_length"],
            defaults["formatter"]["max_line_length"]
        );
        assert_eq!(value["plugins"]["path"], defaults["plugins"]["path"]);
        assert_eq!(value["plugins"]["plugins"], defaults["plugins"]["plugins"]);
        assert_eq!(value["plugins"]["java"]["data"].as_bool(), Some(true));
        assert_eq!(
            value["plugins"]["java"]["package"].as_str(),
            Some("example.service")
        );
        assert_eq!(value["debug"], defaults["debug"]);
        assert!(value.get("workdir").is_none());

        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn config_inspection_reports_missing_config() {
        let root = temporary_directory("inspect-missing");

        let error = inspect_merged_config(&root).expect_err("missing config should fail");

        assert!(error.contains("Unable to find a non-abstract 'xenomorph.toml'"));
        fs::remove_dir_all(root).expect("test directory should be removed");
    }
}
