use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use xenomorph_common::{
    config::{Config, ConfigValue},
    parser::Declaration,
    plugins::XenoPlugin,
    semantic::{AnalyzerListener, ScopeInfo},
    XenoDiagSeverity, XenoDiagnostic,
};

static PLUGIN: XenoPlugin = XenoPlugin {
    name: "fixture",
    namespace: if cfg!(feature = "namespace-collision") {
        "TS"
    } else {
        "fixture"
    },
    version: if cfg!(feature = "alternate-version") {
        "2"
    } else {
        "1"
    },
    initialize: Some(initialize),
    shutdown: Some(shutdown),
    provide_types: None,
    provide_annotations: None,
    provide_config_schema: None,
    register_generator: Some(listener),
    register_analyzer: Some(listener),
};

#[cfg(not(feature = "bad-abi"))]
xenomorph_common::export_plugin!(PLUGIN);

#[cfg(feature = "bad-abi")]
#[no_mangle]
pub extern "C" fn xenomorph_plugin_v1() -> *const xenomorph_common::plugins::abi::Header {
    use xenomorph_common::plugins::abi::{Header, ABI_MAGIC};
    static HEADER: Header = Header {
        magic: ABI_MAGIC,
        major: 2,
        minor: 0,
        table_size: 0,
        protocol: 1,
        capabilities: 0,
    };
    &HEADER
}

fn event(workdir: &std::path::Path, message: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(workdir.join("events"))
        .map_err(|error| error.to_string())?;
    writeln!(file, "{message}").map_err(|error| error.to_string())
}

fn initialize(config: &Config) -> Result<(), String> {
    if config.plugins.config.get("fixture") == Some(&ConfigValue::Boolean(false)) {
        return Err("Fixture initialization rejected".into());
    }
    event(&config.workdir, "initialize")
}

fn shutdown(config: &Config) -> Result<(), String> {
    event(&config.workdir, "shutdown")
}

struct Listener {
    workdir: PathBuf,
}

fn listener(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    Box::new(Listener {
        workdir: config.workdir.clone(),
    })
}

impl Drop for Listener {
    fn drop(&mut self) {
        event(&self.workdir, "listener-drop").expect("fixture lifecycle event");
    }
}

impl<'src> AnalyzerListener<'src> for Listener {
    fn on_before_module(&mut self, _: &ScopeInfo) {
        event(&self.workdir, "callback-start").expect("fixture lifecycle event");
        if self.workdir.join("panic").exists() {
            panic!("fixture panic");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.workdir.join("block").exists() {
            assert!(
                Instant::now() < deadline,
                "fixture callback barrier timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn on_before_decl(&mut self, declaration: &Declaration, errors: &mut Vec<XenoDiagnostic>) {
        if let Declaration::Type { name, .. } = declaration {
            errors.push(XenoDiagnostic {
                tags: vec![],
                location: (*name),
                severity: XenoDiagSeverity::Info,
                message: format!("fixture initialized for {}", self.workdir.display()),
            });
        }
    }

    fn on_after_module(&mut self, _: &ScopeInfo) {
        event(&self.workdir, "callback-end").expect("fixture lifecycle event");
    }
}
