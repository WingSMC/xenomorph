#[no_mangle]
static NAME: &'static str = "test_plugin";

#[no_mangle]
static VERSION: &'static str = "1.0";

pub static PLUGIN: xenomorph_common::Plugin = xenomorph_common::Plugin {
	name: NAME,
	version: VERSION,
	// initialize: initialize,
	// lint: lint,
	// generate: generate,
	// execute: execute,
	// cleanup: cleanup,
};