use super::{
    protocol::{Diagnostic, Initialization, ModuleRequest},
    syntax, XenoPlugin,
};
use crate::{
    config::Config,
    semantic::{run_plugin_listener, MetadataGraph},
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
};
use xenomorph_plugin_api::{Buffer, Bytes};

#[derive(Serialize, Deserialize)]
pub struct Description {
    pub name: String,
    pub namespace: String,
    pub version: String,
    pub metadata: MetadataGraph,
    #[serde(with = "super::protocol::json_schema")]
    pub config_schema: Option<serde_json::Value>,
}

struct Instance {
    config: Config,
    failed: bool,
}

fn buffer(status: u32, bytes: Vec<u8>) -> Buffer {
    let bytes = bytes.into_boxed_slice();
    let len = bytes.len();
    Buffer {
        ptr: Box::into_raw(bytes).cast::<u8>(),
        len,
        status,
    }
}

fn invoke(operation: impl FnOnce() -> Result<Vec<u8>, String>) -> Buffer {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(bytes)) => buffer(0, bytes),
        Ok(Err(error)) => buffer(1, error.into_bytes()),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic");
            let output = buffer(2, format!("Plugin panicked: {message}").into_bytes());
            // A panic payload can have a panicking destructor. Never unwind across FFI.
            std::mem::forget(payload);
            output
        }
    }
}

unsafe fn input<'a>(bytes: Bytes) -> Result<&'a [u8], String> {
    if bytes.len == 0 {
        return Ok(&[]);
    }
    if bytes.ptr.is_null() || bytes.len > isize::MAX as usize {
        return Err("Invalid plugin input buffer".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) })
}

pub fn describe(plugin: &'static XenoPlugin<'static>) -> Buffer {
    invoke(|| {
        let types = plugin.provide_types.map(|provide| provide()).unwrap_or(&[]);
        let annotations = plugin
            .provide_annotations
            .map(|provide| provide())
            .unwrap_or(&[]);
        let description = Description {
            name: plugin.name.to_string(),
            namespace: plugin.namespace.to_string(),
            version: plugin.version.to_string(),
            metadata: super::metadata::from_descriptors(plugin.namespace, types, annotations)?,
            config_schema: plugin
                .provide_config_schema
                .map(|provide| {
                    serde_json::from_str(provide())
                        .map_err(|error| format!("Invalid plugin config schema: {error}"))
                })
                .transpose()?,
        };
        xenomorph_plugin_api::codec::encode(&description)
    })
}

/// # Safety
/// Input must remain readable during this call; output must point to writable handle storage.
pub unsafe fn initialize(
    plugin: &'static XenoPlugin<'static>,
    bytes: Bytes,
    output: *mut *mut c_void,
) -> Buffer {
    invoke(|| {
        if output.is_null() {
            return Err("Missing plugin instance output".into());
        }
        unsafe {
            *output = std::ptr::null_mut();
        }
        let config =
            xenomorph_plugin_api::codec::decode::<Initialization>(unsafe { input(bytes)? })
                .map_err(|error| format!("Invalid plugin initialization: {error}"))?
                .into_config()?;
        if let Some(initialize) = plugin.initialize {
            initialize(&config)?;
        }
        let instance = Box::new(Instance {
            config,
            failed: false,
        });
        unsafe {
            *output = Box::into_raw(instance).cast();
        }
        Ok(Vec::new())
    })
}

/// # Safety
/// Handle must be a live instance from this SDK. Calls on one instance must be serialized.
pub unsafe fn run(
    plugin: &'static XenoPlugin<'static>,
    handle: *mut c_void,
    bytes: Bytes,
) -> Buffer {
    if handle.is_null() {
        return buffer(1, b"Missing plugin instance".to_vec());
    }
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    let result = invoke(|| {
        if instance.failed {
            return Err("Plugin instance is failed; reload it before calling again".into());
        }
        let request: ModuleRequest = xenomorph_plugin_api::codec::decode(unsafe { input(bytes)? })
            .map_err(|error| format!("Invalid plugin module request: {error}"))?;
        request.scope.validate()?;
        let factory = if request.generation {
            plugin.register_generator
        } else {
            plugin.register_analyzer
        }
        .ok_or_else(|| "Plugin does not provide the requested capability".to_string())?;
        syntax::validate(&request.syntax.source, &request.syntax.declarations)?;
        let ast = &request.syntax.declarations;
        let diagnostics = run_plugin_listener(
            factory(&instance.config),
            ast,
            &request.scope,
            &request.syntax.source,
        )
        .iter()
        .map(Diagnostic::from)
        .collect::<Vec<_>>();
        xenomorph_plugin_api::codec::encode(&diagnostics)
    });
    if result.status != 0 {
        instance.failed = true;
    }
    result
}

/// # Safety
/// Handle must be a live instance from this SDK and cannot be used or destroyed again.
pub unsafe fn destroy(plugin: &'static XenoPlugin<'static>, handle: *mut c_void) -> Buffer {
    invoke(|| {
        if !handle.is_null() {
            let instance = unsafe { Box::from_raw(handle.cast::<Instance>()) };
            if let Some(shutdown) = plugin.shutdown {
                shutdown(&instance.config)?;
            }
            drop(instance);
        }
        Ok(Vec::new())
    })
}

/// # Safety
/// Buffer must have been returned by this SDK and must be released exactly once.
pub unsafe fn release(output: Buffer) {
    if !output.ptr.is_null() {
        let slice = std::ptr::slice_from_raw_parts_mut(output.ptr, output.len);
        drop(unsafe { Box::from_raw(slice) });
    }
}

#[macro_export]
macro_rules! export_plugin {
    ($plugin:ident) => {
        #[no_mangle]
        pub extern "C" fn xenomorph_plugin_v1() -> *const $crate::plugins::abi::Header {
            unsafe extern "C" fn describe() -> $crate::plugins::abi::Buffer {
                $crate::plugins::sdk::describe(&$plugin)
            }
            unsafe extern "C" fn initialize(
                input: $crate::plugins::abi::Bytes,
                output: *mut *mut std::ffi::c_void,
            ) -> $crate::plugins::abi::Buffer {
                unsafe { $crate::plugins::sdk::initialize(&$plugin, input, output) }
            }
            unsafe extern "C" fn run(
                handle: *mut std::ffi::c_void,
                input: $crate::plugins::abi::Bytes,
            ) -> $crate::plugins::abi::Buffer {
                unsafe { $crate::plugins::sdk::run(&$plugin, handle, input) }
            }
            unsafe extern "C" fn destroy(
                handle: *mut std::ffi::c_void,
            ) -> $crate::plugins::abi::Buffer {
                unsafe { $crate::plugins::sdk::destroy(&$plugin, handle) }
            }
            unsafe extern "C" fn release(output: $crate::plugins::abi::Buffer) {
                unsafe { $crate::plugins::sdk::release(output) }
            }
            static API: $crate::plugins::abi::Api = $crate::plugins::abi::Api {
                header: $crate::plugins::abi::Header::new(
                    if $plugin.register_generator.is_some() {
                        $crate::plugins::abi::GENERATOR
                    } else {
                        0
                    } | if $plugin.register_analyzer.is_some() {
                        $crate::plugins::abi::ANALYZER
                    } else {
                        0
                    },
                ),
                describe,
                initialize,
                run,
                destroy,
                release,
            };
            &API.header
        }
    };
}
