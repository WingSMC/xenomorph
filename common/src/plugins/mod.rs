use crate::{config::Config, parser::XenoAst};
use libloading::{Library, Symbol};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Debug, Clone)]
pub struct PluginCompletion {
    pub label: &'static str,
    pub detail: Option<&'static str>,
    pub documentation: Option<&'static str>,
}

#[derive(Debug)]
pub struct XenoPlugin<'a> {
    pub name: &'a str,
    pub version: &'a str,

    pub initialize: Option<fn() -> ()>,
    pub provide_types: Option<fn() -> &'static [PluginCompletion]>,
    pub provide_annotations: Option<fn() -> &'static [PluginCompletion]>,
    pub generate: Option<fn(ast: &XenoAst) -> ()>,
    // pub lint: fn(&Self) -> (),
    // execute: fn(&str),
    // cleanup: fn(),

    // parse_custom_declaration
    // parse_custom_expression
}

macro_rules! lib_filename {
    ($lib_name: expr) => {{
        #[cfg(target_os = "windows")]
        {
            format!("{}.dll", $lib_name)
        }
        #[cfg(target_os = "macos")]
        {
            format!("lib{}.dylib", $lib_name)
        }
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        {
            format!("lib{}.so", $lib_name)
        }
    }};
}

pub static PLUGINS: OnceLock<Vec<&'static XenoPlugin<'static>>> = OnceLock::new();
impl<'a> XenoPlugin<'a> {
    pub fn get_plugins() -> &'static Vec<&'static XenoPlugin<'static>> {
        PLUGINS.get_or_init(|| Self::load_plugins())
    }

    fn plugins_directory() -> PathBuf {
        let config = Config::get();
        config.workdir.join(&config.plugins.path)
    }

    fn load_plugin_library(path: &Path) -> Result<Library, String> {
        unsafe { Library::new(path) }
            .map_err(|e| format!("Library load error\n{}:\n{}", path.display(), e))
    }

    fn load_plugin(lib: Library) -> Result<&'static XenoPlugin<'static>, libloading::Error> {
        let lib_ref = Box::leak(Box::new(lib));
        let load: Symbol<fn() -> &'static XenoPlugin<'static>> = unsafe { lib_ref.get(b"load")? };
        Ok(load())
    }

    fn create_plugin_instance(
        lib: Library,
        name: &String,
    ) -> Result<&'static XenoPlugin<'static>, String> {
        Self::load_plugin(lib).map_err(|e| {
            format!(
                "Symbol resolution error in plugin '{}'. Make sure it's compatible with the current version!\n{}",
                name,
                e
            )
        })
    }

    fn log_loading_error(plugin_name: &String, e: &String) {
        eprintln!("Failed to load plugin '{}':\n{}", plugin_name, e);
    }

    fn load_plugins() -> Vec<&'static XenoPlugin<'static>> {
        let plugin_config = &Config::get().plugins;
        let plugins_dir = Self::plugins_directory();

        plugin_config
            .plugins
            .iter()
            .filter_map(|plugin_name| {
                let lib_path = plugins_dir.join(lib_filename!(&plugin_name));

                Self::load_plugin_library(&lib_path)
                    .and_then(|lib| Self::create_plugin_instance(lib, &plugin_name))
                    .map_err(|e| Self::log_loading_error(plugin_name, &e))
                    .ok()
            })
            .collect()
    }
}
