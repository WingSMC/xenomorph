use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

use super::{Config, DEFAULT_CONFIG_IS_ABSTRACT};
use crate::plugins::XenoPlugin;

/// Default location (relative to the workspace root) where the generated
/// `xenomorph.toml` JSON Schema is written.
pub const RC_SCHEMA_RELATIVE_PATH: &str = ".xenomorph/xenomorph.schema.json";

/// Builds the JSON Schema describing the `xenomorph.toml` config file, merging
/// in each plugin's contributed `[plugins.<name>]` configuration schema.
///
/// Plugins extend the schema by implementing
/// [`XenoPlugin::provide_config_schema`], returning a JSON Schema object for
/// their own config section. The returned object is inserted under
/// `properties.plugins.properties.<plugin-name>`.
pub fn build_rc_schema(plugins: &[&'static XenoPlugin<'static>]) -> Value {
    let defaults = Config::default();

    // Collect plugin-provided config schemas keyed by plugin name.
    let mut plugin_sections: Map<String, Value> = Map::new();
    for plugin in plugins {
        let Some(provide) = plugin.provide_config_schema else {
            continue;
        };
        match serde_json::from_str::<Value>(provide()) {
            Ok(schema) => {
                plugin_sections.insert(plugin.name.to_string(), schema);
            }
            Err(e) => {
                eprintln!(
                    "Plugin '{}' provided an invalid config schema: {}",
                    plugin.name, e
                );
            }
        }
    }

    // `[plugins]` section: built-in keys plus per-plugin config sections.
    let mut plugins_properties: Map<String, Value> = Map::new();
    plugins_properties.insert(
        "path".to_string(),
        json!({
            "type": "string",
            "description": "Directory containing the compiled plugin libraries, relative to the workspace root.",
            "default": &defaults.plugins.path
        }),
    );
    plugins_properties.insert(
        "plugins".to_string(),
        json!({
            "type": "array",
            "description": "Plugin library names to load (without the platform-specific `lib` prefix or file extension).",
            "items": { "type": "string" },
            "uniqueItems": true,
            "default": &defaults.plugins.plugins
        }),
    );
    for (name, schema) in plugin_sections {
        plugins_properties.insert(name, schema);
    }

    let plugins_section = json!({
        "type": "object",
        "description": "Plugin loading and per-plugin configuration.",
        "properties": Value::Object(plugins_properties),
        "additionalProperties": true
    });

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Xenomorph configuration (xenomorph.toml)",
        "description": "Configuration file for the Xenomorph toolchain.",
        "type": "object",
        "properties": {
            "abstract": {
                "type": "boolean",
                "description": "When true, directory discovery skips this file and continues upward to an authoritative config. The file can still be used through `extends`.",
                "default": DEFAULT_CONFIG_IS_ABSTRACT
            },
            "extends": {
                "type": "string",
                "description": "Path to another xenomorph.toml to inherit, relative to this config. This config recursively overrides colliding keys and subkeys."
            },
            "parser": {
                "type": "object",
                "description": "Parser and entry-point configuration.",
                "properties": {
                    "entry": {
                        "type": "string",
                        "description": "Entry module path relative to the workspace root, without the `.xen` extension.",
                        "default": &defaults.parser.entry
                    }
                },
                "additionalProperties": false
            },
            "formatter": {
                "type": "object",
                "description": "Source formatter layout configuration.",
                "properties": {
                    "indent_kind": {
                        "type": "string",
                        "description": "Indent with spaces or tab characters.",
                        "enum": ["space", "tab"],
                        "default": defaults.formatter.indent_kind
                    },
                    "indent_width": {
                        "type": "integer",
                        "description": "Visual width of one indentation level.",
                        "minimum": 1,
                        "default": defaults.formatter.indent_width
                    },
                    "max_line_length": {
                        "type": "integer",
                        "description": "Preferred maximum formatted line length before declarations are wrapped.",
                        "minimum": 1,
                        "default": defaults.formatter.max_line_length
                    },
                    "line_ending": {
                        "type": "string",
                        "description": "Line ending emitted by the formatter. Auto preserves the first line ending found in the source.",
                        "enum": ["lf", "crlf", "auto"],
                        "default": defaults.formatter.line_ending
                    }
                },
                "additionalProperties": false
            },
            "plugins": plugins_section,
            "debug": {
                "type": "object",
                "description": "Debug output and CLI diagnostic logging configuration.",
                "properties": {
                    "plugins": {
                        "type": "boolean",
                        "description": "Print plugin loading diagnostics.",
                        "default": defaults.debug.plugins
                    },
                    "tokens": {
                        "type": "boolean",
                        "description": "Print the token stream for each module.",
                        "default": defaults.debug.tokens
                    },
                    "ast": {
                        "type": "boolean",
                        "description": "Print the parsed AST for each module.",
                        "default": defaults.debug.ast
                    },
                    "loglevel": {
                        "type": "string",
                        "description": "Minimum diagnostic severity displayed by the xeno CLI.",
                        "enum": ["error", "warning", "info"],
                        "default": defaults.debug.loglevel
                    }
                },
                "additionalProperties": false
            },
            "workdir": {
                "type": "string",
                "description": "Workspace root override. Normally detected automatically from the location of `xenomorph.toml`."
            }
        },
        "additionalProperties": false
    })
}

/// Builds the `xenomorph.toml` schema and writes it (pretty-printed) to
/// `out_path`, creating parent directories as needed.
pub fn write_rc_schema(
    plugins: &[&'static XenoPlugin<'static>],
    out_path: &Path,
) -> std::io::Result<()> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let schema = build_rc_schema(plugins);
    let contents = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string());
    fs::write(out_path, contents)
}

#[cfg(test)]
mod tests {
    use super::{build_rc_schema, Config, DEFAULT_CONFIG_IS_ABSTRACT};
    use serde_json::json;

    #[test]
    fn formatter_schema_describes_layout_options() {
        let schema = build_rc_schema(&[]);
        let formatter = schema
            .pointer("/properties/formatter")
            .expect("formatter section should be present");

        assert_eq!(formatter["additionalProperties"], false);
        assert_eq!(
            formatter["properties"]["line_ending"]["enum"],
            json!(["lf", "crlf", "auto"])
        );
    }

    #[test]
    fn schema_uses_the_runtime_config_defaults() {
        let schema = build_rc_schema(&[]);
        let defaults = serde_json::to_value(Config::default())
            .expect("runtime config defaults should serialize");

        assert_eq!(
            schema["properties"]["parser"]["properties"]["entry"]["default"],
            defaults["parser"]["entry"]
        );
        assert_eq!(
            schema["properties"]["formatter"]["properties"]["indent_kind"]["default"],
            defaults["formatter"]["indent_kind"]
        );
        assert_eq!(
            schema["properties"]["formatter"]["properties"]["indent_width"]["default"],
            defaults["formatter"]["indent_width"]
        );
        assert_eq!(
            schema["properties"]["formatter"]["properties"]["max_line_length"]["default"],
            defaults["formatter"]["max_line_length"]
        );
        assert_eq!(
            schema["properties"]["formatter"]["properties"]["line_ending"]["default"],
            defaults["formatter"]["line_ending"]
        );
        assert_eq!(
            schema["properties"]["plugins"]["properties"]["path"]["default"],
            defaults["plugins"]["path"]
        );
        assert_eq!(
            schema["properties"]["plugins"]["properties"]["plugins"]["default"],
            defaults["plugins"]["plugins"]
        );
        assert_eq!(
            schema["properties"]["debug"]["properties"]["plugins"]["default"],
            defaults["debug"]["plugins"]
        );
        assert_eq!(
            schema["properties"]["debug"]["properties"]["tokens"]["default"],
            defaults["debug"]["tokens"]
        );
        assert_eq!(
            schema["properties"]["debug"]["properties"]["ast"]["default"],
            defaults["debug"]["ast"]
        );
        assert_eq!(
            schema["properties"]["debug"]["properties"]["loglevel"]["default"],
            defaults["debug"]["loglevel"]
        );
        assert_eq!(
            schema["properties"]["abstract"]["default"],
            DEFAULT_CONFIG_IS_ABSTRACT
        );
    }

    #[test]
    fn debug_schema_describes_loglevel() {
        let schema = build_rc_schema(&[]);
        let loglevel = schema
            .pointer("/properties/debug/properties/loglevel")
            .expect("debug.loglevel should be present");

        assert_eq!(loglevel["type"], "string");
        assert_eq!(loglevel["enum"], json!(["error", "warning", "info"]));
    }

    #[test]
    fn schema_describes_hierarchy_and_only_permits_unknown_plugin_sections() {
        let schema = build_rc_schema(&[]);

        assert_eq!(schema["properties"]["abstract"]["type"], "boolean");
        assert_eq!(schema["properties"]["extends"]["type"], "string");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["parser"]["additionalProperties"],
            false
        );
        assert_eq!(
            schema["properties"]["formatter"]["additionalProperties"],
            false
        );
        assert_eq!(
            schema["properties"]["plugins"]["additionalProperties"],
            true
        );
        assert_eq!(schema["properties"]["debug"]["additionalProperties"], false);
    }
}
