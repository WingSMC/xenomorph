use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_lsp::lsp_types::{
    Command, CompletionItemKind, InsertTextFormat, MarkupContent, TextEdit,
};

/// A type that represents a completion token with rich semantic data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionToken {
    /// The label of the completion item (display text).
    pub label: String,
    /// The kind of completion item (e.g., variable, function, etc.).
    pub kind: Option<CompletionItemKind>,
    /// A short description for the completion item.
    pub detail: Option<String>,
    /// Detailed documentation for the item, possibly in Markdown or plaintext.
    pub documentation: Option<MarkupContent>,
    /// Indicates if the item is deprecated.
    pub deprecated: Option<bool>,
    /// If true, this item is preselected when the completion list is shown.
    pub preselect: Option<bool>,
    /// Text used for sorting the completion items.
    pub sort_text: Option<String>,
    /// Text used for filtering the completion items.
    pub filter_text: Option<String>,
    /// The text that should be inserted when the item is picked.
    pub insert_text: Option<String>,
    /// Indicates whether the insert text is plain text or a snippet.
    pub insert_text_format: Option<InsertTextFormat>,
    /// The text edit to be applied to the document on selecting this item.
    pub text_edit: Option<TextEdit>,
    /// Additional text edits to be applied, which must not overlap with the main edit.
    pub additional_text_edits: Option<Vec<TextEdit>>,
    /// Optional command to run after inserting the completion.
    pub command: Option<Command>,
    /// Characters that, when typed, will confirm the completion.
    pub commit_characters: Option<Vec<String>>,
    /// A field to hold any extra data useful for resolving the completion item.
    pub data: Option<Value>,
}

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::plugins::load_plugins;
use xenomorph_common::Plugin;

#[derive(Debug)]
struct Backend {
    client: Client,
    plugins: Vec<&'static Plugin<'static>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // Declare that your server supports completion
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        " ".to_string(),
                    ]),
                    ..Default::default()
                }),
                // Keep your existing capabilities
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn completion(&self, _: CompletionParams) -> Result<Option<CompletionResponse>> {
        Ok(Some(CompletionResponse::Array(
            self.plugins
                .iter()
                .map(|p| (p.provide)())
                .flatten()
                .map(|c| CompletionItem {
                    label: c.to_string(),
                    kind: Some(CompletionItemKind::PROPERTY),
                    detail: Some("More detail".to_string()),
                    ..Default::default()
                })
                .collect(),
        )))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let plugins = load_plugins(&vec!["test".to_string()]);

    let (service, socket) = LspService::new(|client| Backend { client, plugins });
    Server::new(stdin, stdout, socket).serve(service).await;
}
