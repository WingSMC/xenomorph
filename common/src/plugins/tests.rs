use super::*;
use crate::{
    config::{ConfigValue, PluginConfigs},
    module::XenoRegistry,
};
use std::{
    fs,
    time::{Duration, Instant},
};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "xenomorph-abi-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("index.xen"), "type Example = { value: string };").unwrap();
        Self(path)
    }

    fn config(&self, directory: &Path, names: &[&str]) -> Config {
        let mut config = Config::default_with_workdir(self.0.clone());
        config.parser.entry = "index".into();
        config.plugins.path = directory.to_str().unwrap().to_string();
        config.plugins.plugins = names.iter().map(|name| name.to_string()).collect();
        config
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // Remove only this test's explicitly created workspace.
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn plugin_directory() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn wait_for_event(path: &Path, event: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(path).unwrap_or_default().contains(event) {
        assert!(Instant::now() < deadline, "Timed out waiting for {event}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn authoritative_config_drives_all_generator_output_paths() {
    let workspace = Workspace::new();
    let mut config = workspace.config(
        &plugin_directory(),
        &[
            "xenomorph_typescript",
            "xenomorph_java",
            "xenomorph_json_schema",
        ],
    );
    for name in ["typescript", "java", "json_schema"] {
        let mut options = toml::map::Map::new();
        options.insert(
            "output".into(),
            ConfigValue::String(format!("generated/{name}")),
        );
        config
            .plugins
            .config
            .insert(name.into(), ConfigValue::Table(options));
    }
    let registry = XenoRegistry::load_workspace_with_config(config, true).unwrap();
    assert!(workspace
        .0
        .join("generated")
        .join("typescript")
        .join("index.ts")
        .is_file());
    assert!(workspace
        .0
        .join("generated")
        .join("java")
        .join("Example.java")
        .is_file());
    assert!(workspace
        .0
        .join("generated")
        .join("json_schema")
        .join("index.schema.json")
        .is_file());
    assert_eq!(registry.plugins.len(), 3);
    assert!(registry.plugins.iter().all(LoadedPlugin::is_loaded));
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn lsp_metadata_survives_generator_library_release() {
    let workspace = Workspace::new();
    let config = workspace.config(
        &plugin_directory(),
        &[
            "xenomorph_typescript",
            "xenomorph_java",
            "xenomorph_json_schema",
        ],
    );
    let plugins = load_plugins(&config, PluginMode::Analysis).unwrap();
    assert!(plugins.iter().all(|plugin| !plugin.is_loaded()));
    assert!(plugins[0].description.metadata.types.contains_key("symbol"));
    assert!(plugins[1]
        .description
        .metadata
        .annotations
        .contains_key("Lombok"));
    let registry = XenoRegistry::with_config(config, false).unwrap();
    let diagnostics = registry.load_module_from_source(
        &workspace.0.join("index.xen"),
        "type Example = symbol @Lombok(Data);".into(),
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == crate::XenoDiagSeverity::Err),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn in_flight_callback_keeps_library_alive_until_ordered_shutdown() {
    let workspace = Workspace::new();
    let config = workspace.config(&plugin_directory(), &["xenomorph_test_plugin"]);
    fs::write(workspace.0.join("block"), "").unwrap();
    let registry = Arc::new(XenoRegistry::with_config(config, false).unwrap());
    let shadow = registry.plugins[0]
        .runtime
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .artifact
        .path
        .clone();
    let worker_registry = Arc::clone(&registry);
    let worker = std::thread::spawn(move || {
        worker_registry.load_module(&[&worker_registry.entry], true, None)
    });
    wait_for_event(&workspace.0.join("events"), "callback-start");
    drop(registry);
    assert!(shadow.exists(), "In-flight code was unloaded");
    fs::remove_file(workspace.0.join("block")).unwrap();
    worker.join().unwrap();
    assert!(!shadow.exists(), "Shadow copy was not cleaned up");
    let events = fs::read_to_string(workspace.0.join("events")).unwrap();
    assert!(events.find("callback-end").unwrap() < events.find("listener-drop").unwrap());
    assert!(events.find("listener-drop").unwrap() < events.find("shutdown").unwrap());
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn initialization_failure_and_panic_are_explicit() {
    let workspace = Workspace::new();
    let mut config = workspace.config(&plugin_directory(), &["xenomorph_test_plugin"]);
    config.plugins.config = PluginConfigs::from([("fixture".into(), ConfigValue::Boolean(false))]);
    let error = match load_plugins(&config, PluginMode::Analysis) {
        Ok(_) => panic!("Expected initialization failure"),
        Err(error) => error,
    };
    assert!(error.contains("initialization rejected"), "{error}");
    config.plugins.config.clear();
    let registry = XenoRegistry::with_config(config, false).unwrap();
    fs::write(workspace.0.join("panic"), "").unwrap();
    let diagnostics = registry.load_module(&[&registry.entry], true, None);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("fixture panic")),
        "{diagnostics:?}"
    );
    fs::remove_file(workspace.0.join("panic")).unwrap();
    let diagnostics = registry.load_module_from_source(
        &workspace.0.join("index.xen"),
        "type Changed = string;".into(),
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("reload")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "Build stable fixture: cargo +stable build -p xenomorph_test_plugin --target-dir target\\plugin-fixture-stable"]
fn independently_compiled_stable_plugin_interoperates_with_nightly_host() {
    let workspace = Workspace::new();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target")
        .join("plugin-fixture-stable")
        .join("debug");
    let config = workspace.config(&directory, &["xenomorph_test_plugin"]);
    let registry = XenoRegistry::with_config(config, false).unwrap();
    let diagnostics = registry.load_module(&[&registry.entry], true, None);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("fixture initialized")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "Build incompatible fixture with --features xenomorph_test_plugin/bad-abi --target-dir target\\plugin-fixture-bad"]
fn incompatible_dll_is_rejected_before_reading_callbacks() {
    let workspace = Workspace::new();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target")
        .join("plugin-fixture-bad")
        .join("debug");
    let config = workspace.config(&directory, &["xenomorph_test_plugin"]);
    let error = match load_plugins(&config, PluginMode::Analysis) {
        Ok(_) => panic!("Expected ABI failure"),
        Err(error) => error,
    };
    assert!(error.contains("Incompatible plugin ABI"), "{error}");
    assert!(!workspace.0.join("events").exists());
}

#[test]
#[ignore = "Build workspace cdylibs and independent stable fixture first"]
fn shadow_copy_allows_replacing_binary_and_reloading() {
    let workspace = Workspace::new();
    let original = workspace.0.join(lib_filename!("xenomorph_test_plugin"));
    let current_binary = plugin_directory().join(lib_filename!("xenomorph_test_plugin"));
    let stable_binary = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target")
        .join("plugin-fixture-stable")
        .join("debug")
        .join(lib_filename!("xenomorph_test_plugin"));
    assert!(
        fs::read(&current_binary).unwrap() != fs::read(&stable_binary).unwrap(),
        "Fixture binaries are not independent builds"
    );
    fs::copy(current_binary, &original).unwrap();
    let config = workspace.config(&workspace.0, &["xenomorph_test_plugin"]);
    let old = load_plugins(&config, PluginMode::Analysis).unwrap();
    assert_eq!(old[0].description.version, "1");
    let old_shadow = old[0]
        .runtime
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .artifact
        .path
        .clone();
    fs::copy(stable_binary, &original).expect("Loaded plugin locked its original artifact");
    let new = load_plugins(&config, PluginMode::Analysis).unwrap();
    assert_eq!(
        new[0].description.version, "2",
        "Reload reused the old native image"
    );
    let new_shadow = new[0]
        .runtime
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .artifact
        .path
        .clone();
    assert_ne!(old_shadow, new_shadow);
    assert!(old_shadow.exists() && new_shadow.exists());
    drop(old);
    assert!(!old_shadow.exists() && new_shadow.exists());
    drop(new);
    assert!(!new_shadow.exists());
    let events = fs::read_to_string(workspace.0.join("events")).unwrap();
    assert_eq!(
        events
            .lines()
            .filter(|event| *event == "initialize")
            .count(),
        2
    );
    assert_eq!(
        events.lines().filter(|event| *event == "shutdown").count(),
        2
    );
}
