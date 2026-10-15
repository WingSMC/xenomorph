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
        fs::write(
            path.join("index.xen"),
            "pub type Example = { value: string };",
        )
        .unwrap();
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
fn sdk_namespaces_declarations_that_share_builtin_names() {
    use crate::semantic::{XenoAnnotationKind, XenoType};
    static TYPE: XenoType = XenoType {
        name: "string",
        documentation: Some("Plugin string"),
        generic_params: None,
        parents: None,
    };
    static ANNOTATION: XenoAnnotation = XenoAnnotation {
        name: "min",
        documentation: None,
        kind: XenoAnnotationKind::Meta,
        params: &[&crate::semantic::ANY_TARGET_PARAM],
        variadic: false,
    };
    static TYPES: &[&XenoType] = &[&TYPE];
    static ANNOTATIONS: &[&XenoAnnotation] = &[&ANNOTATION];
    static PLUGIN: XenoPlugin = XenoPlugin {
        name: "fixture",
        namespace: "fixture",
        version: "1",
        initialize: None,
        shutdown: None,
        provide_types: Some(|| TYPES),
        provide_annotations: Some(|| ANNOTATIONS),
        provide_config_schema: None,
        register_generator: None,
        register_analyzer: None,
    };
    let output = sdk::describe(&PLUGIN);
    assert_eq!(output.status, 0);
    let bytes = unsafe { std::slice::from_raw_parts(output.ptr, output.len) }.to_vec();
    unsafe { sdk::release(output) };
    let description: sdk::Description = abi::codec::decode(&bytes).unwrap();
    assert!(description.metadata.types.contains_key("fixture::string"));
    assert!(description
        .metadata
        .annotations
        .contains_key("fixture::min"));
    assert!(!description.metadata.types.contains_key("string"));
    assert_eq!(
        description.metadata.annotations["fixture::min"].params[0]
            .constraint
            .name(),
        "any"
    );
}

#[test]
fn plugin_graphs_keep_core_and_other_plugin_declarations_distinct() {
    use crate::semantic::*;
    static TRAIT: XenoTrait = XenoTrait {
        name: "KeyTrait",
        documentation: None,
        kind: XenoTraitKind::Sum(&KEY_TRAIT),
        parents: None,
    };
    static TYPE: XenoType = XenoType {
        name: "string",
        documentation: Some("Plugin string"),
        generic_params: Some(&[&GenericParam {
            name: "T",
            constraint: Some(XenoConstraint::Trait(&TRAIT)),
        }]),
        parents: Some(&[XenoParent::Trait(&TRAIT)]),
    };
    static ANNOTATION: XenoAnnotation = XenoAnnotation {
        name: "min",
        documentation: None,
        kind: XenoAnnotationKind::Meta,
        params: &[
            &crate::semantic::ANY_TARGET_PARAM,
            &XenoParam {
                name: "value",
                constraint: XenoConstraint::Type(&TYPE),
            },
        ],
        variadic: false,
    };
    let core = MetadataGraph::builtins();
    let mut merged = core.clone();
    let mut namespaces = std::collections::HashSet::new();
    for namespace in ["first", "second"] {
        let graph = metadata::from_descriptors(namespace, &[&TYPE], &[&ANNOTATION]).unwrap();
        assert_eq!(graph.types.len(), 1);
        assert_eq!(graph.traits.len(), 1);
        assert_eq!(graph.annotations.len(), 1);
        let name = format!("{namespace}::string");
        let tr = format!("{namespace}::KeyTrait");
        assert_eq!(
            graph.types[&name].parents,
            vec![OwnedConstraint::Trait(tr.clone())]
        );
        assert_eq!(
            graph.types[&name].generic_params[0].constraint,
            Some(OwnedConstraint::Trait(tr.clone()))
        );
        assert_eq!(
            graph.traits[&tr].kind,
            OwnedTraitKind::Sum("KeyTrait".into())
        );
        assert_eq!(
            graph.annotations[&format!("{namespace}::min")].params[1].constraint,
            OwnedConstraint::Type(name)
        );
        validate_description(
            &sdk::Description {
                name: namespace.into(),
                namespace: namespace.into(),
                version: "1".into(),
                metadata: graph,
                config_schema: None,
            },
            &mut namespaces,
            &mut merged,
        )
        .unwrap();
    }
    merged.validate().unwrap();
    assert_eq!(merged.types["string"], core.types["string"]);
    assert_eq!(merged.traits["KeyTrait"], core.traits["KeyTrait"]);
    assert_eq!(merged.annotations["min"], core.annotations["min"]);
    let mut hierarchy = TypeHierarchy::default();
    hierarchy.insert_metadata(&merged).unwrap();
    assert!(hierarchy.has_type("first::string"));
    assert!(hierarchy.has_type("second::string"));
    assert!(hierarchy.has_trait("first::KeyTrait"));
}

#[test]
fn loader_rejects_duplicate_namespaces_even_without_declarations() {
    let mut namespaces = std::collections::HashSet::new();
    let mut merged = crate::semantic::MetadataGraph::builtins();
    let mut description = sdk::Description {
        name: "first".into(),
        namespace: "shared".into(),
        version: "1".into(),
        metadata: crate::semantic::MetadataGraph::default(),
        config_schema: None,
    };
    validate_description(&description, &mut namespaces, &mut merged).unwrap();
    description.name = "second".into();
    assert_eq!(
        validate_description(&description, &mut namespaces, &mut merged).unwrap_err(),
        "Duplicate plugin namespace 'shared'"
    );
}

#[test]
#[ignore = "Build workspace cdylibs and the namespace-collision fixture first"]
fn dynamic_loader_reports_namespace_collisions_before_initializing_the_plugin() {
    let workspace = Workspace::new();
    let directory = plugin_directory();
    fs::copy(
        directory.join(lib_filename!("xenomorph_typescript")),
        workspace.0.join(lib_filename!("xenomorph_typescript")),
    )
    .unwrap();
    fs::copy(
        directory
            .join("../plugin-fixture-collision/debug")
            .join(lib_filename!("xenomorph_test_plugin")),
        workspace.0.join(lib_filename!("xenomorph_test_plugin")),
    )
    .unwrap();
    let config = workspace.config(
        &workspace.0,
        &["xenomorph_typescript", "xenomorph_test_plugin"],
    );
    let error = match load_plugins(&config, PluginMode::Analysis) {
        Ok(_) => panic!("namespace collision was accepted"),
        Err(error) => error,
    };
    assert!(
        error.contains("Duplicate plugin namespace 'typescript'"),
        "{error}"
    );
    assert!(
        !workspace.0.join("events").exists(),
        "colliding plugin initialized"
    );
    assert!(XenoRegistry::with_config(config, false)
        .err()
        .unwrap()
        .message
        .contains("Duplicate plugin namespace 'typescript'"));
}

#[test]
fn loader_rejects_invalid_namespaces_and_declarations_outside_the_namespace() {
    let core = crate::semantic::MetadataGraph::builtins();
    for namespace in ["", " ", "with spaces", "nested::namespace", "type"] {
        let description = sdk::Description {
            name: "plugin".into(),
            namespace: namespace.into(),
            version: "1".into(),
            metadata: crate::semantic::MetadataGraph::default(),
            config_schema: None,
        };
        assert!(
            validate_description(&description, &mut Default::default(), &mut core.clone()).is_err()
        );
    }
    for kind in ["type", "trait", "annotation"] {
        let mut graph = crate::semantic::MetadataGraph::default();
        match kind {
            "type" => {
                graph
                    .types
                    .insert("string".into(), core.types["string"].clone());
            }
            "trait" => {
                graph
                    .traits
                    .insert("KeyTrait".into(), core.traits["KeyTrait"].clone());
            }
            _ => {
                graph
                    .annotations
                    .insert("min".into(), core.annotations["min"].clone());
            }
        }
        let description = sdk::Description {
            name: "plugin".into(),
            namespace: "plugin".into(),
            version: "1".into(),
            metadata: graph,
            config_schema: None,
        };
        let error = validate_description(&description, &mut Default::default(), &mut core.clone())
            .unwrap_err();
        assert!(
            error.contains("outside plugin namespace 'plugin'"),
            "{error}"
        );
    }
}

#[test]
fn metadata_merge_and_consumers_reject_overwrites_without_mutating_state() {
    use crate::semantic::*;
    let core = MetadataGraph::builtins();
    let mut analyzer = Analyzer::new(false, &[]).unwrap();
    let mut scope = ScopeInfo {
        module_path: "test".into(),
        abs_path: PathBuf::new(),
        own_types: vec![],
        imported_types: Default::default(),
        builtin_types: Default::default(),
        known_annotations: Default::default(),
        type_hierarchy: TypeHierarchy::default(),
        annotations: Default::default(),
    };
    scope.insert_metadata(&core).unwrap();
    for kind in ["type", "trait", "annotation"] {
        let mut graph = MetadataGraph::default();
        match kind {
            "type" => {
                let mut ty = core.types["string"].clone();
                ty.documentation = Some("Overwrite".into());
                graph.types.insert(ty.name.clone(), ty);
            }
            "trait" => {
                let mut tr = core.traits["KeyTrait"].clone();
                tr.documentation = Some("Overwrite".into());
                graph.traits.insert(tr.name.clone(), tr);
            }
            _ => {
                let mut annotation = core.annotations["min"].clone();
                annotation.documentation = Some("Overwrite".into());
                graph
                    .annotations
                    .insert(annotation.name.clone(), annotation);
            }
        }
        let mut merged = core.clone();
        assert!(merged
            .merge(&graph)
            .unwrap_err()
            .contains("Duplicate metadata"));
        assert_eq!(merged, core);
        assert!(analyzer
            .add_metadata(&graph)
            .unwrap_err()
            .contains("Duplicate metadata"));
        assert!(scope
            .insert_metadata(&graph)
            .unwrap_err()
            .contains("Duplicate metadata"));
        assert_eq!(scope.annotations, core.annotations);
        assert_eq!(
            scope
                .type_hierarchy
                .get_type_in(None, "string")
                .unwrap()
                .documentation,
            core.types["string"].documentation
        );
        scope.validate_metadata().unwrap();
    }
    let mut invalid = MetadataGraph::default();
    let mut ty = core.types["string"].clone();
    ty.name = "plugin::Invalid".into();
    ty.parents = vec![OwnedConstraint::Type("Missing".into())];
    invalid.types.insert(ty.name.clone(), ty);
    let mut merged = core.clone();
    assert!(merged
        .merge(&invalid)
        .unwrap_err()
        .contains("missing type 'Missing'"));
    assert_eq!(merged, core);
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
fn generators_only_receive_entry_exports_and_their_dependencies() {
    let workspace = Workspace::new();
    fs::write(workspace.0.join("index.xen"), "pub import barrel::Api; import other::Spare; type Local = Api; pub type Root = Local; type Unused = Missing;").unwrap();
    fs::write(workspace.0.join("barrel.xen"), "pub import other::Api;").unwrap();
    fs::write(
        workspace.0.join("other.xen"),
        "type Helper = string; pub type Api = { id: Helper }; pub type Spare = number;",
    )
    .unwrap();
    let config = workspace.config(
        &plugin_directory(),
        &[
            "xenomorph_typescript",
            "xenomorph_java",
            "xenomorph_json_schema",
        ],
    );
    let registry = XenoRegistry::load_workspace_with_config(config, true).unwrap();
    let diagnostics = registry.get_all_errors_for("index");
    assert!(diagnostics
        .iter()
        .any(|d| d.message == "Unused import 'Spare'."
            && d.severity == crate::XenoDiagSeverity::Warn
            && d.tags.contains(&crate::XenoDiagnosticTag::Unnecessary)));
    assert!(diagnostics
        .iter()
        .any(|d| d.message == "Not used by the entry module's exports."));
    assert!(!diagnostics
        .iter()
        .any(|d| d.message == "Unused import 'Api'."));
    let entry = fs::read_to_string(workspace.0.join("index.ts")).unwrap();
    assert!(
        entry.contains("export type { Api } from \"./other\";"),
        "{entry}"
    );
    assert!(entry.contains("export type Root = Local;"));
    assert!(!entry.contains("Spare") && !entry.contains("Unused") && !entry.contains("Missing"));
    let other = fs::read_to_string(workspace.0.join("other.ts")).unwrap();
    assert!(other.contains("type Helper = string;"));
    assert!(!other.contains("export type Helper"));
    assert!(other.contains("export interface Api"));
    assert!(!other.contains("Spare"));
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(workspace.0.join("index.schema.json")).unwrap())
            .unwrap();
    assert_eq!(
        schema["$defs"]["Api"]["$ref"],
        "./other.schema.json#/$defs/Api"
    );
    assert_eq!(
        schema["$defs"]["Local"]["$ref"],
        "./other.schema.json#/$defs/Api"
    );
    assert_eq!(schema["$defs"]["Root"]["$ref"], "#/$defs/Local");
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(workspace.0.join("other.schema.json")).unwrap())
            .unwrap();
    assert_eq!(schema["$defs"]["Helper"]["type"], "string");
    assert_eq!(
        schema["$defs"]["Api"]["properties"]["id"]["$ref"],
        "#/$defs/Helper"
    );
    assert!(schema["$defs"].get("Spare").is_none());
    assert!(workspace.0.join("Api.java").is_file());
    assert!(!workspace.0.join("Unused.java").exists());
    assert!(!workspace.0.join("Spare.java").exists());
    registry.load_module_from_source(
        &workspace.0.join("index.xen"),
        "pub import other::Spare;".into(),
    );
    assert!(registry.get_all_errors_for("index").is_empty());
    let other = fs::read_to_string(workspace.0.join("other.ts")).unwrap();
    assert!(other.contains("Spare"));
    assert!(
        !other.contains("Api") && !other.contains("Helper"),
        "{other}"
    );
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
    assert!(plugins[0]
        .description
        .metadata
        .types
        .contains_key("TS::symbol"));
    assert!(plugins[1]
        .description
        .metadata
        .annotations
        .contains_key("java::Lombok"));
    let registry = XenoRegistry::with_config(config, false).unwrap();
    let diagnostics = registry.load_module_from_source(
        &workspace.0.join("index.xen"),
        "type Example = TS::symbol @java::Lombok(java::Data);".into(),
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
