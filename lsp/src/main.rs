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
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        " ".to_string(),
                    ]),
                    ..Default::default()
                }),
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

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        Ok(item)
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        plugins: load_plugins(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
