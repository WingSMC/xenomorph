use crate::{
    config::Config,
    semantic::{AnalyzerListener, XenoAnnotation, XenoType},
};
pub mod protocol;
pub mod sdk;
pub mod syntax;
use libloading::Library;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
pub use xenomorph_plugin_api as abi;

/// A factory function that creates a fresh listener instance for each analysis run.
pub type ListenerFactory = fn(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>>;
pub type LifecycleCallback = fn(&Config) -> Result<(), String>;

#[derive(Debug)]
pub struct XenoPlugin<'a> {
    pub name: &'a str,
    pub version: &'a str,

    pub initialize: Option<LifecycleCallback>,
    pub shutdown: Option<LifecycleCallback>,
    pub provide_types: Option<fn() -> &'static [&'static XenoType]>,
    pub provide_annotations: Option<fn() -> &'static [&'static XenoAnnotation]>,
    /// Returns a JSON Schema (as a string) describing this plugin's
    /// `[plugins.<name>]` configuration section in `xenomorph.toml`.
    pub provide_config_schema: Option<fn() -> &'static str>,
    pub register_generator: Option<ListenerFactory>,
    pub register_analyzer: Option<ListenerFactory>,
    // execute: fn(&[&str]),

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginMode {
    Analysis,
    Generation,
    Schema,
}

#[derive(Clone)]
pub struct LoadedPlugin {
    pub description: Arc<sdk::Description>,
    pub capabilities: u64,
    runtime: Option<Arc<Mutex<Runtime>>>,
}

impl LoadedPlugin {
    pub fn is_loaded(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn run(
        &self,
        generation: bool,
        ast: &[crate::parser::Declaration],
        scope: &crate::semantic::ScopeInfo,
        source: &str,
    ) -> Result<Vec<crate::XenoDiagnostic>, String> {
        let capability = if generation {
            abi::GENERATOR
        } else {
            abi::ANALYZER
        };
        if self.capabilities & capability == 0 {
            return Ok(Vec::new());
        }
        self.runtime.as_ref().ok_or_else(|| {
            format!(
                "Plugin '{}' is not loaded for the requested capability",
                self.description.name
            )
        })?;
        let tokens = syntax::source_tokens(ast);
        syntax::validate(source, ast)?;
        let input = encode_request(generation, source, ast, scope)?;
        self.run_encoded(&input, &tokens, source)
    }

    pub fn run_encoded(
        &self,
        input: &[u8],
        tokens: &[crate::TokenData],
        source: &str,
    ) -> Result<Vec<crate::XenoDiagnostic>, String> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| format!("Plugin '{}' is not loaded", self.description.name))?;
        let mut runtime = runtime
            .lock()
            .map_err(|_| "Plugin instance lock is poisoned".to_string())?;
        if runtime.failed {
            return Err(format!(
                "Plugin '{}' failed; reload it",
                self.description.name
            ));
        }
        let output = unsafe { (runtime.api().run)(runtime.instance, abi::Bytes::borrowed(input)) };
        let bytes = runtime.read_output(output);
        let result = bytes.and_then(|bytes| {
            let diagnostics: Vec<protocol::Diagnostic> = abi::codec::decode(&bytes)
                .map_err(|error| format!("Invalid plugin diagnostics: {error}"))?;
            diagnostics
                .into_iter()
                .map(|diagnostic| {
                    let severity = diagnostic.get_as_xeno_severity()?;
                    if diagnostic.location.start > diagnostic.location.end
                        || source
                            .get(
                                diagnostic.location.start as usize
                                    ..diagnostic.location.end as usize,
                            )
                            .is_none()
                    {
                        return Err("Invalid plugin diagnostic source range".into());
                    }
                    let _location = tokens
                        .iter()
                        .find(|token| {
                            token.start <= diagnostic.location.start
                                && token.end >= diagnostic.location.end
                        })
                        .cloned()
                        .ok_or_else(|| {
                            "Plugin diagnostic references a token outside the submitted AST"
                                .to_string()
                        })?;
                    Ok(crate::XenoDiagnostic {
                        location: diagnostic.location,
                        message: diagnostic.message,
                        severity,
                    })
                })
                .collect()
        });
        if result.is_err() {
            runtime.failed = true;
        }
        result
    }
}

pub fn encode_request(
    generation: bool,
    source: &str,
    ast: &[crate::parser::Declaration],
    scope: &crate::semantic::ScopeInfo,
) -> Result<Vec<u8>, String> {
    #[derive(serde::Serialize)]
    struct Syntax<'a> {
        source: &'a str,
        tokens: &'a [crate::lexer::Token],
        declarations: &'a [crate::parser::Declaration],
    }
    #[derive(serde::Serialize)]
    struct Request<'a> {
        generation: bool,
        syntax: Syntax<'a>,
        scope: &'a crate::semantic::ScopeInfo,
    }
    abi::codec::encode(&Request {
        generation,
        syntax: Syntax {
            source,
            tokens: &[],
            declarations: ast,
        },
        scope,
    })
}

pub fn load_plugins(config: &Config, mode: PluginMode) -> Result<Vec<LoadedPlugin>, String> {
    let mut plugins = Vec::new();
    let mut names = std::collections::HashSet::new();
    for library_name in &config.plugins.plugins {
        let path = config
            .workdir
            .join(&config.plugins.path)
            .join(lib_filename!(library_name));
        let mut runtime = Runtime::open(&path)
            .map_err(|error| format!("Plugin '{library_name}' ({}): {error}", path.display()))?;
        let capabilities = runtime.api().header.capabilities;
        let output = unsafe { (runtime.api().describe)() };
        let description: sdk::Description = abi::codec::decode(&runtime.read_output(output)?)
            .map_err(|error| {
                format!("Plugin '{library_name}' provided invalid metadata: {error}")
            })?;
        description.metadata.validate().map_err(|error| {
            format!("Plugin '{library_name}' provided invalid metadata: {error}")
        })?;
        if description.name.is_empty() || !names.insert(description.name.clone()) {
            return Err(format!(
                "Plugin '{library_name}' has an empty or duplicate name '{}'",
                description.name
            ));
        }
        let required = match mode {
            PluginMode::Analysis => abi::ANALYZER,
            PluginMode::Generation => abi::GENERATOR | abi::ANALYZER,
            PluginMode::Schema => 0,
        };
        let runtime = if capabilities & required != 0 {
            let input = abi::codec::encode(&protocol::Initialization::new(config))
                .map_err(|error| error.to_string())?;
            let mut instance = std::ptr::null_mut();
            let output =
                unsafe { (runtime.api().initialize)(abi::Bytes::borrowed(&input), &mut instance) };
            runtime.instance = instance;
            runtime.read_output(output)?;
            if instance.is_null() {
                return Err(format!(
                    "Plugin '{}' initialized without an instance",
                    description.name
                ));
            }
            Some(Arc::new(Mutex::new(runtime)))
        } else {
            runtime.close()?;
            None
        };
        plugins.push(LoadedPlugin {
            description: Arc::new(description),
            capabilities,
            runtime,
        });
    }
    Ok(plugins)
}

static NEXT_COPY: AtomicU64 = AtomicU64::new(0);

struct Artifact {
    path: PathBuf,
    directory: PathBuf,
    retained: bool,
}

impl Artifact {
    fn copy(path: &Path) -> Result<Self, String> {
        let filename = path
            .file_name()
            .ok_or_else(|| "Plugin path has no filename".to_string())?;
        let directory = std::env::temp_dir().join(format!(
            "xenomorph-plugin-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos(),
            NEXT_COPY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory)
            .map_err(|error| format!("Cannot create plugin shadow directory: {error}"))?;
        let artifact = Self {
            path: directory.join(filename),
            directory,
            retained: false,
        };
        std::fs::copy(path, &artifact.path)
            .map_err(|error| format!("Cannot copy plugin '{}': {error}", path.display()))?;
        Ok(artifact)
    }
}

impl Drop for Artifact {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "Cannot remove plugin shadow copy '{}': {error}",
                    self.path.display()
                );
            }
        }
        if let Err(error) = std::fs::remove_dir(&self.directory) {
            eprintln!(
                "Cannot remove plugin shadow directory '{}': {error}",
                self.directory.display()
            );
        }
    }
}

struct Runtime {
    library: Option<Library>,
    api: *const abi::Api,
    instance: *mut std::ffi::c_void,
    failed: bool,
    artifact: Artifact,
}

// ABI v1 permits moving instances between threads, but requires serialized calls.
unsafe impl Send for Runtime {}

impl Runtime {
    fn open(path: &Path) -> Result<Self, String> {
        let artifact = Artifact::copy(path)?;
        let library = unsafe { Library::new(&artifact.path) }.map_err(|error| error.to_string())?;
        let mut runtime = Self {
            library: Some(library),
            api: std::ptr::null(),
            instance: std::ptr::null_mut(),
            failed: false,
            artifact,
        };
        let entry = unsafe {
            runtime
                .library
                .as_ref()
                .unwrap()
                .get::<unsafe extern "C" fn() -> *const abi::Header>(abi::ENTRY_POINT)
        }
        .map_err(|error| {
            format!("Missing versioned plugin entry point; rebuild this plugin: {error}")
        })?;
        let header = unsafe { entry() };
        if header.is_null() {
            return Err("Plugin returned a null ABI header".into());
        }
        unsafe { &*header }.validate()?;
        runtime.api = header.cast();
        Ok(runtime)
    }

    fn api(&self) -> &abi::Api {
        // The table is validated before publication; this owner retains its library.
        unsafe { &*self.api }
    }

    fn read_output(&self, output: abi::Buffer) -> Result<Vec<u8>, String> {
        let valid = output.len <= isize::MAX as usize && (!output.ptr.is_null() || output.len == 0);
        if !valid {
            return Err("Plugin returned an invalid buffer".into());
        }
        let bytes = if output.len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(output.ptr, output.len) }.to_vec()
        };
        unsafe { (self.api().release)(output) };
        if output.status != 0 {
            return Err(String::from_utf8(bytes)
                .map_err(|error| format!("Plugin error is not UTF-8: {error}"))?);
        }
        Ok(bytes)
    }

    fn close(&mut self) -> Result<(), String> {
        if !self.instance.is_null() {
            let instance = std::mem::replace(&mut self.instance, std::ptr::null_mut());
            let output = unsafe { (self.api().destroy)(instance) };
            if let Err(error) = self.read_output(output) {
                if let Some(library) = self.library.take() {
                    // Failed cleanup cannot establish that unloading native code is safe.
                    std::mem::forget(library);
                    self.artifact.retained = true;
                }
                return Err(format!(
                    "{error}; retaining library and shadow copy '{}' because plugin shutdown failed",
                    self.artifact.path.display()
                ));
            }
        }
        if let Some(library) = self.library.take() {
            if let Err(error) = library.close() {
                self.artifact.retained = true;
                return Err(format!(
                    "Plugin library close failed: {error}; retaining shadow copy '{}'",
                    self.artifact.path.display()
                ));
            }
        }
        Ok(())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!(
                "Plugin '{}' teardown failed: {error}",
                self.artifact.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests;
