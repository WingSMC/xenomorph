use crate::{
    config::{load_config_key, workdir},
    Plugin,
};
use libloading::{Library, Symbol};
use std::path::{Path, PathBuf};

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

fn plugins_directory(plugin_config: toml::Value) -> PathBuf {
    let workdir = workdir().expect("No workdir found");
    let plugin_path = plugin_config
        .get("path")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();

    workdir.join(plugin_path)
}

fn load_plugin_library(path: &Path) -> Result<Library, String> {
    unsafe { Library::new(path) }
        .map_err(|e| format!("Library load error\n{}:\n{}", path.display(), e))
}

fn load_plugin(lib: Library) -> Result<&'static Plugin<'static>, libloading::Error> {
    let lib_ref = Box::leak(Box::new(lib));
    let load: Symbol<fn() -> &'static Plugin<'static>> = unsafe { lib_ref.get(b"load")? };
    Ok(load())
}

fn create_plugin_instance(lib: Library, name: &String) -> Result<&'static Plugin<'static>, String> {
    load_plugin(lib).map_err(|e| {
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

pub fn load_plugins() -> Vec<&'static Plugin<'static>> {
    let plugin_config = load_config_key("plugins");
    let plugin_names = plugin_config
        .get("plugins")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<String>>();
    let plugins_dir = plugins_directory(plugin_config);

    plugin_names
        .iter()
        .filter_map(|plugin_name| {
            let lib_path = plugins_dir.join(lib_filename!(&plugin_name));

            load_plugin_library(&lib_path)
                .and_then(|lib| create_plugin_instance(lib, &plugin_name))
                .map_err(|e| log_loading_error(plugin_name, &e))
                .ok()
        })
        .collect()
}
