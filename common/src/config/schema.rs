use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

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
            "description": "Directory containing the compiled plugin libraries, relative to the workspace root."
        }),
    );
    plugins_properties.insert(
        "plugins".to_string(),
        json!({
            "type": "array",
            "description": "Plugin library names to load (without the platform-specific `lib` prefix or file extension).",
            "items": { "type": "string" },
            "uniqueItems": true
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
        "$id": "https://wingsmc.github.io/xenomorph/xenomorph.schema.json",
        "title": "Xenomorph configuration (xenomorph.toml)",
        "description": "Configuration file for the Xenomorph toolchain.",
        "type": "object",
        "properties": {
            "parser": {
                "type": "object",
                "description": "Parser and entry-point configuration.",
                "properties": {
                    "entry": {
                        "type": "string",
                        "description": "Entry module path relative to the workspace root, without the `.xen` extension.",
                        "default": "index.xen"
                    }
                },
                "additionalProperties": false
            },
            "plugins": plugins_section,
            "debug": {
                "type": "object",
                "description": "Debug output toggles.",
                "properties": {
                    "plugins": {
                        "type": "boolean",
                        "description": "Print plugin loading diagnostics.",
                        "default": false
                    },
                    "tokens": {
                        "type": "boolean",
                        "description": "Print the token stream for each module.",
                        "default": false
                    },
                    "ast": {
                        "type": "boolean",
                        "description": "Print the parsed AST for each module.",
                        "default": false
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
