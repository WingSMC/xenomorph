use std::sync::LazyLock;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};
use xenomorph_common::semantic::{BUILTIN_ANNOTATIONS, BUILTIN_TYPES};

pub fn create_completion_item(
    label: &str,
    detail: Option<&str>,
    kind: CompletionItemKind,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: detail.map(|d| d.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: detail.map(|d| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: d.to_string(),
            })
        }),
        ..Default::default()
    }
}

pub static BUILTIN_ANNOTATION_COMPLETIONS: LazyLock<Vec<CompletionItem>> = LazyLock::new(|| {
    BUILTIN_ANNOTATIONS
        .iter()
        .map(|annotation| {
            create_completion_item(
                annotation.name,
                annotation.documentation,
                CompletionItemKind::FUNCTION,
            )
        })
        .collect()
});

pub static BUILTIN_TYPE_COMPLETIONS: LazyLock<Vec<CompletionItem>> = LazyLock::new(|| {
    BUILTIN_TYPES
        .iter()
        .map(|t| create_completion_item(t.name, t.documentation, CompletionItemKind::CLASS))
        .collect()
});
