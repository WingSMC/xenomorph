use tower_lsp::lsp_types::CompletionItem;
use xenomorph_common::plugins::XenoPlugin;

fn provide_types() -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: "TestType".to_string(),
        kind: Some(tower_lsp::lsp_types::CompletionItemKind::CLASS),
        ..Default::default()
    }]
}

static NAME: &'static str = "test_plugin";
static VERSION: &'static str = "1.0";
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
