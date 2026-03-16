use xenomorph_common::plugins::{PluginCompletion, XenoPlugin};

static PROVIDED_TYPES: [PluginCompletion; 1] = [PluginCompletion {
    label: "TestType",
    detail: Some("A test type provided by the test plugin"),
    documentation: Some("**TestType** is a type provided by the __test plugin__ for testing purposes.\n\n- It has no special properties.\n- It is used to demonstrate how plugins can provide types to the Xenomorph ecosystem."),
}];

fn provide_types() -> &'static [PluginCompletion] {
    &PROVIDED_TYPES
}

static NAME: &str = "test_plugin";
static VERSION: &str = "1.0";
static PLUGIN: XenoPlugin = XenoPlugin {
    name: NAME,
    version: VERSION,
    initialize: None,
    provide_types: Some(provide_types),
    provide_annotations: None,
    generate: None,
};

#[no_mangle]
fn load() -> &'static XenoPlugin<'static> {
    &PLUGIN
}
