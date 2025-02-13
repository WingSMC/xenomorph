use libloading::{Library, Symbol};
use std::{
    env,
    path::{Path, PathBuf},
};

#[allow(dead_code)]
pub trait XenoPlugin {
    fn name(&self) -> &str;
    //fn version(&self) -> &str;
    //
    //fn initialize(&self);
    //fn lint(&self);
    //fn generate(&self) -> String;
    //fn execute(&self, data: &str);
    //fn cleanup(&self);
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Plugin {
    name: String,
    // version: String,
    //
    // initialize: fn(),
    // lint: fn(),
    // generate: fn() -> String,
    // execute: fn(&str),
    // cleanup: fn(),
}

impl Plugin {
    pub fn new(lib: Library) -> Result<Self, libloading::Error> {
        let name: Symbol<fn() -> String> = unsafe { lib.get(b"name")? };
        //let version: Symbol<fn() -> String> = unsafe { lib.get(b"version")? };
        //let initialize: Symbol<fn()> = unsafe { lib.get(b"initialize")? };
        //let lint: Symbol<fn()> = unsafe { lib.get(b"lint")? };
        //let generate: Symbol<fn() -> String> = unsafe { lib.get(b"generate")? };
        //let execute: Symbol<fn(&str)> = unsafe { lib.get(b"execute")? };
        //let cleanup: Symbol<fn()> = unsafe { lib.get(b"cleanup")? };

        Ok(Plugin {
            name: name(),
            //version: version(),
            //
            //initialize: *initialize,
            //lint: *lint,
            //execute: *execute,
            //generate: *generate,
            //cleanup: *cleanup,
        })
    }
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
        .with_file_name("plugins")
}

fn load_plugin_library(path: &Path) -> Result<Library, String> {
    unsafe { Library::new(path) }
        .map_err(|e| format!("Library load error\n{}:\n{}", path.display(), e))
}

fn log_loading_error(plugin_name: &String, e: &String) {
    eprintln!("Failed to load plugin '{}':\n{}", plugin_name, e);
}

fn create_plugin_instance(lib: Library, name: &str) -> Result<Plugin, String> {
    Plugin::new(lib).map_err(|e| {
        format!(
            "Symbol resolution error in plugin '{}'. Make sure it's compatible with the current version!{}",
            name,
            e
        )
    })
}

pub fn load_plugins(plugin_names: Vec<String>) -> Vec<Plugin> {
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
