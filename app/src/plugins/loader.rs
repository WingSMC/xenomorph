use libloading::{Library, Symbol};
use std::{
    env,
    path::{Path, PathBuf},
};
use xenomorph_common::Plugin;

pub fn load_plugin<'a>(lib: Library) -> Result<Plugin<'a>, libloading::Error> {
    let lib_ref = Box::leak(Box::new(lib));
    let plugin: Symbol<'a, Plugin<'a>> = unsafe { lib_ref.get(b"PLUGIN")? };

    Ok(*plugin)
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

fn plugins_directory() -> PathBuf {
    env::current_exe()
        .expect("Failed to get executable path for loading plugins")
        .with_file_name("")
}

fn load_plugin_library(path: &Path) -> Result<Library, String> {
    unsafe { Library::new(path) }
        .map_err(|e| format!("Library load error\n{}:\n{}", path.display(), e))
}

fn log_loading_error(plugin_name: &String, e: &String) {
    eprintln!("Failed to load plugin '{}':\n{}", plugin_name, e);
}

fn create_plugin_instance<'a>(lib: Library, name: &str) -> Result<Plugin<'a>, String> {
    load_plugin(lib).map_err(|e| {
        format!(
            "Symbol resolution error in plugin '{}'. Make sure it's compatible with the current version!{}",
            name,
            e
        )
    })
}

pub fn load_plugins<'a>(plugin_names: &Vec<String>) -> Vec<Plugin<'a>> {
    let plugins_dir = plugins_directory();

    plugin_names
        .iter()
        .filter_map(|plugin_name| {
                let lib_path = plugins_dir.join(lib_filename!(plugin_name));

            load_plugin_library(&lib_path)
                .and_then(|lib| create_plugin_instance(lib, &plugin_name))
                .map_err(|e| log_loading_error(plugin_name, &e))
                .ok()
        })
        .collect()
}
