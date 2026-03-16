use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentSymbolParams,
    DocumentSymbolResponse, Documentation, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, InsertTextFormat, Location, MarkupContent, MarkupKind, MessageType, OneOf,
    Position, Range, ServerCapabilities, SymbolInformation, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::semantic::XenoDefNode;
use xenomorph_common::{
    lexer::{Lexer, Token, TokenVariant},
    parser::{Declaration, Parser},
    plugins::{load_plugins, XenoPlugin},
    ParseError, TokenData,
};
use xenomorph_lsp_common::types::{
    create_completion_item, BUILTIN_ANNOTATION_COMPLETIONS, BUILTIN_TYPE_COMPLETIONS,
};

#[derive(Debug)]
struct Backend {
    client: Client,
    plugins: Vec<&'static XenoPlugin<'static>>,
    document_map: Mutex<HashMap<Url, Arc<String>>>,
}

trait EditorPosition {
    fn to_editor_position(&self) -> Position;
    fn to_editor_range(&self) -> Range;
}
impl<'src> EditorPosition for TokenData<'src> {
    fn to_editor_position(&self) -> Position {
        Position {
            line: self.l,
            character: self.c,
        }
    }
    fn to_editor_range(&self) -> Range {
        let start = self.to_editor_position();
        Range {
            start,
            end: Position {
                line: start.line,
                character: start.character + self.v.len() as u32,
            },
        }
    }
}

impl Backend {
    /// Returns a clone of the stored document text for the given URI.
    fn get_document_text(&self, uri: &Url) -> Option<Arc<String>> {
        let documents = self.document_map.lock().ok()?;
        documents.get(uri).cloned()
    }

    fn ast_to_definition_completions<'src>(
        &self,
        ast: &'src Vec<Declaration>,
    ) -> impl Iterator<Item = CompletionItem> + 'src {
        ast.iter().filter_map(|decl| match decl {
            Declaration::TypeDecl { name, docs, .. } => Some(create_completion_item(
                name.v,
                *docs,
                CompletionItemKind::CLASS,
            )),
        })
    }

    fn get_builtin_types(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.plugins
            .iter()
            .filter_map(|p| p.provide_types.map(|f| f()))
            .flatten()
            .map(|pc| create_completion_item(pc.label, pc.detail, CompletionItemKind::CLASS))
            .chain(BUILTIN_TYPE_COMPLETIONS.iter().cloned())
    }

    fn get_builtin_annotations(&self) -> impl Iterator<Item = CompletionItem> {
        BUILTIN_ANNOTATION_COMPLETIONS.iter().cloned()
    }

    // Validate document and send diagnostics
    async fn validate_document(&self, uri: &Url) {
        let text = match self.get_document_text(uri) {
            Some(t) => t,
            None => {
                self.client
                    .show_message(
                        MessageType::ERROR,
                        "Failed to parse document for validation.",
                    )
                    .await;
                return;
            }
        };

        let make_diagnostics = |errors: &[ParseError<'_>]| -> Vec<Diagnostic> {
            errors
                .iter()
                .map(|err| Diagnostic {
                    range: err.location.to_editor_range(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: err.message.clone(),
                    source: Some("xenomorph".to_string()),
                    ..Default::default()
                })
                .collect()
        };

        let diagnostics = match Lexer::tokenize(&text) {
            Err(e) => make_diagnostics(&[e]),
            Ok(tokens) => {
                let (_ast, errors) = Parser::parse(&tokens);
                make_diagnostics(&errors)
            }
        };

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// Find the token at a given position
    fn find_token_at_position<'a>(
        &self,
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        tokens.iter().find(|(_, data)| {
            let token_range = data.to_editor_range();
            token_range.start <= position && position <= token_range.end
        })
    }

    fn find_declaration<'src>(
        &self,
        location: Position,
        tokens: &'src [Token<'src>],
        ast: &'src Vec<Declaration<'src>>,
    ) -> Option<&'src Declaration<'src>> {
        let token = self.find_token_at_position(tokens, location)?;

        if token.0 != TokenVariant::Identifier {
            return None;
        }

        let searched_name = token.1.v;
        for decl in ast {
            match decl {
                Declaration::TypeDecl { name, .. } => {
                    if name.v == searched_name {
                        return Some(decl);
                    }
                }
            }
        }

        None
    }

    fn find_definition<'src>(
        &self,
        location: Position,
        tokens: &'src [Token<'src>],
        ast: &'src Vec<Declaration<'src>>,
    ) -> Option<&'src TokenData<'src>> {
        let decl = self.find_declaration(location, tokens, ast)?;
        return Some(match decl {
            Declaration::TypeDecl { name, .. } => name,
        });
    }

    /// Get completions based on context
    fn get_context_completions(
        &self,
        tokens: &[Token],
        ast: &Vec<Declaration>,
        position: Position,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        if let Some(current_token) = self.find_token_at_position(tokens, position) {
            match current_token.0 {
                TokenVariant::At => items.extend(self.get_builtin_annotations()),
                TokenVariant::Colon => items.extend(
                    self.ast_to_definition_completions(&ast)
                        .chain(self.get_builtin_types()),
                ),
                TokenVariant::Eq => items.extend(vec![CompletionItem {
                    label: "struct".to_string(),
                    kind: Some(CompletionItemKind::SNIPPET),
                    detail: Some("Create a new struct type".to_string()),
                    insert_text: Some("{\n\t${1:property}: ${2:type},\n\t$0\n}".to_string()),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                }]),
                _ => {}
            }
        }

        items
    }

    // Provide hover information for a token
    fn get_hover_for_location(
        &self,
        location: Position,
        tokens: &[Token],
        ast: &Vec<Declaration>,
    ) -> Option<Hover> {
        let token = self.find_token_at_position(tokens, location)?;

        if token.0 != TokenVariant::Identifier {
            return None;
        }

        let searched_name = token.1.v;
        let def = self.find_declaration(location, tokens, ast)?;
        let contents = match def {
            Declaration::TypeDecl { name, docs, .. } if name.v == searched_name => {
                format!("**{}**\n\n{}", name.v, docs.unwrap_or(""))
            }
            _ => return None,
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: contents,
            }),
            range: Some(token.1.to_editor_range()),
        })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        "@".to_string(),
                        "{".to_string(),
                    ]),
                    all_commit_characters: None,
                    work_done_progress_options: Default::default(),
                    completion_item: None,
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Xenomorph Language Server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = Arc::new(params.text_document.text);

        {
            let mut documents = self.document_map.lock().unwrap();
            documents.insert(uri.clone(), text);
        }

        self.validate_document(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        {
            let mut documents = self.document_map.lock().unwrap();
            if let Some(doc) = documents.get_mut(&uri) {
                for change in params.content_changes {
                    if let Some(_range) = change.range {
                        // Would need to implement proper incremental updates
                        // For simplicity, we're replacing the entire document
                        *doc = Arc::new(change.text);
                    } else {
                        // Full document update
                        *doc = Arc::new(change.text);
                    }
                }
            }
        }

        self.validate_document(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        {
            let mut documents = self.document_map.lock().unwrap();
            documents.remove(&params.text_document.uri);
        }

        // Clear diagnostics when document is closed
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let mut completions: Vec<CompletionItem> = vec![
            create_completion_item(
                "type",
                Some("Define a new type"),
                CompletionItemKind::KEYWORD,
            ),
            create_completion_item(
                "validator",
                Some("Define a validator"),
                CompletionItemKind::KEYWORD,
            ),
            create_completion_item("set", Some("Define a set"), CompletionItemKind::KEYWORD),
            create_completion_item(
                "enum",
                Some("Define an enumeration"),
                CompletionItemKind::KEYWORD,
            ),
        ];

        // Context-aware completions
        // if let Some((tokens, _)) = self.parse_document(&uri) {
        //     let context_completions = self.get_context_completions(&tokens, position);
        //     completions.extend(context_completions);
        // }

        // Plugin-provided completions
        completions.extend(
            self.plugins
                .iter()
                .filter_map(|p| p.provide_types.map(|f| f()))
                .flatten()
                .map(|pc| create_completion_item(pc.label, pc.detail, CompletionItemKind::CLASS)),
        );

        Ok(Some(CompletionResponse::Array(completions)))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        let mut resolved = item.clone();

        if resolved.documentation.is_none() {
            let doc = match resolved.label.as_str() {
                "type" => "Defines a new type in Xenomorph.\n\nExample:\n``````",
                "string" => "String data type.\n\nCan be constrained with regex patterns or length annotations.",
                "u8" => "8-bit unsigned integer (0-255).\n\nCan be constrained with min/max annotations.",
                _ => "",
            };

            if !doc.is_empty() {
                resolved.documentation = Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.to_string(),
                }));
            }
        }

        Ok(resolved)
    }

    async fn hover(&self, _params: HoverParams) -> Result<Option<Hover>> {
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: "Hover information is not yet implemented.".to_string(),
            }),
            range: None,
        }))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        if let Some(text) = self.get_document_text(&uri) {
            let formatted = format_xenomorph(&text);

            let edit = TextEdit {
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: text.lines().count() as u32,
                        character: 0,
                    },
                },
                new_text: formatted,
            };

            return Ok(Some(vec![edit]));
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let text = match self.get_document_text(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let tokens = match Lexer::tokenize(&text) {
            Ok(tokens) => tokens,
            Err(_) => return Ok(None),
        };

        let (ast, _errors) = Parser::parse(&tokens);

        let def_tree = XenoDefNode::ast_to_def_tree(&ast);
        // let

        /*  Decommission
        old, unused

        if let Some(token) = self.find_token_at_position(&tokens, position) {
                    if token.0 == TokenVariant::Identifier {
                        let searched_name = token.1.v;
                        let (ast, _errors) = Parser::parse(&tokens);

                        for decl in &ast {
                            match decl {
                                Declaration::TypeDecl { name, .. } => {
                                    if name.v == searched_name {
                                        let range = Range {
                                            start: Position {
                                                line: name.l,
                                                character: name.c,
                                            },
                                            end: Position {
                                                line: name.l,
                                                character: name.c + name.v.len() as u32,
                                            },
                                        };

                                        return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                                            uri: uri.clone(),
                                            range,
                                        })));
                                    }
                                }
                            }
                        }
                    } /
                }*/

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        let text = match self.get_document_text(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let tokens = match Lexer::tokenize(&text) {
            Ok(tokens) => tokens,
            Err(_) => return Ok(None),
        };

        let (ast, _errors) = Parser::parse(&tokens);

        #[allow(deprecated)]
        let symbols: Vec<SymbolInformation> = ast
            .iter()
            .map(|decl| match decl {
                Declaration::TypeDecl { name, .. } => SymbolInformation {
                    name: name.v.to_string(),
                    kind: SymbolKind::STRUCT,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: Range {
                            start: Position {
                                line: name.l,
                                character: name.c,
                            },
                            end: Position {
                                line: name.l,
                                character: name.c + name.v.len() as u32,
                            },
                        },
                    },
                    container_name: None,
                },
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Flat(symbols)))
    }
}

// TODO: implement a more sophisticated formatter
fn format_xenomorph(text: &str) -> String {
    let mut result = String::new();
    let mut indent_level: u32 = 0;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        if trimmed.starts_with('}') || trimmed.starts_with(')') || trimmed.starts_with(']') {
            indent_level = indent_level.saturating_sub(1);
        }

        for _ in 0..indent_level {
            result.push_str("  "); // 2 spaces per indent level
        }

        result.push_str(trimmed);
        result.push('\n');

        if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            indent_level += 1;
        }
    }

    result
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        plugins: load_plugins(),
        document_map: Mutex::new(HashMap::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
