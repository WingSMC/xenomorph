use xenomorph_common::Plugin;

extern "Rust" fn provide() -> Vec<&'static str> {
    vec!["Hello", "World"]
}

static NAME: &'static str = "test_plugin";
static VERSION: &'static str = "1.0";
static PLUGIN: Plugin = Plugin {
    name: NAME,
    version: VERSION,
    provide,
};

#[no_mangle]
pub extern "Rust" fn load() -> &'static Plugin<'static> {
    &PLUGIN
}
