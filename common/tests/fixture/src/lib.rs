use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use xenomorph_common::{
    config::{Config, ConfigValue},
    parser::Declaration,
    plugins::{
        protocol::CustomDeclarationRequest, CustomDeclarationMatcher, CustomDeclarationResult,
        XenoPlugin,
    },
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
    provide_custom_declarations: Some(|| MATCHERS),
    provide_config_schema: None,
    register_generator: if cfg!(feature = "custom-only") {
        None
    } else {
        Some(generator_listener)
    },
    register_analyzer: if cfg!(feature = "custom-only") {
        None
    } else {
        Some(listener)
    },
};

static MATCHERS: &[CustomDeclarationMatcher] = &[
    CustomDeclarationMatcher {
        identifier: "fixture_consume",
        parse: parse_custom,
    },
    CustomDeclarationMatcher {
        identifier: "fixture_error",
        parse: parse_custom,
    },
    CustomDeclarationMatcher {
        identifier: "fixture_invalid",
        parse: parse_custom,
    },
];

fn parse_custom(config: &Config, request: &CustomDeclarationRequest) -> CustomDeclarationResult {
    if request.identifier == "fixture_error" {
        return Err(XenoDiagnostic {
            tags: vec![],
            location: request.content,
            severity: XenoDiagSeverity::Err,
            message: "Fixture custom syntax error".into(),
        });
    }
    if request.identifier == "fixture_invalid" {
        return Err(XenoDiagnostic {
            tags: vec![],
            location: xenomorph_common::TextRange::new(0, 1),
            severity: XenoDiagSeverity::Err,
            message: "Invalid diagnostic anchor".into(),
        });
    }
    let failure = |message| XenoDiagnostic {
        tags: vec![],
        location: request.location,
        severity: XenoDiagSeverity::Err,
        message,
    };
    event(&config.workdir, &format!("consume:{}", request.content())).map_err(failure)?;
    event(
        &config.workdir,
        if request.public {
            "custom-public"
        } else {
            "custom-private"
        },
    )
    .map_err(failure)?;
    fs::write(config.workdir.join("custom-state"), request.content())
        .map_err(|error| failure(error.to_string()))?;
    Ok(())
}
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
    generation: bool,
}

fn listener(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    Box::new(Listener {
        workdir: config.workdir.clone(),
        generation: false,
    })
}

fn generator_listener(config: &Config) -> Box<dyn for<'a> AnalyzerListener<'a>> {
    Box::new(Listener {
        workdir: config.workdir.clone(),
        generation: true,
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
                location: *name,
                severity: XenoDiagSeverity::Info,
                message: format!("fixture initialized for {}", self.workdir.display()),
            });
        }
    }

    fn on_after_module(&mut self, _: &ScopeInfo) {
        if self.generation {
            if let Ok(state) = fs::read(self.workdir.join("custom-state")) {
                fs::write(self.workdir.join("custom-output"), state)
                    .expect("fixture custom output");
            }
        }
        event(&self.workdir, "callback-end").expect("fixture lifecycle event");
    }
}
