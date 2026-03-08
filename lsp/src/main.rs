use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, Documentation, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, InsertTextFormat, Location,
    MarkupContent, MarkupKind, MessageType, OneOf, Position, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::config::Config;
use xenomorph_common::lexer::LexerLocation;
use xenomorph_common::parser::XenoParseResult;
use xenomorph_common::{
    lexer::{Lexer, Token, TokenData, TokenVariant},
    parser::{Declaration, ParseError, Parser},
    plugins::{load_plugins, XenoPlugin},
};
use xenomorph_lsp_common::types::{
    create_completion_item, BUILTIN_ANNOTATION_COMPLETIONS, BUILTIN_TYPE_COMPLETIONS,
};

type AstCache<'src> = (String, Tokens<'src>, XenoParseResult<'src>);

#[derive(Debug)]
struct Backend<'src> {
    client: Client,
    plugins: Vec<&'static XenoPlugin<'static>>,
    document_map: Mutex<HashMap<Url, Rc<String>>>,
    ast_cache: Mutex<HashMap<Url, AstCache<'src>>>,
}

// Helper function to convert parser location to LSP position
fn token_to_editor_location(location: &TokenData) -> Position {
    // Convert from 1-based to 0-based for LSP
    Position {
        line: (location.l - 1) as u32,
        character: (location.c - 1) as u32,
    }
}

impl<'src> Backend<'src> {
    fn parse_document(&self, uri: &Url) -> Option<(String, Tokens<'src>, XenoParseResult<'src>)> {
        let documents = self.document_map.lock().ok()?;
        let text = documents.get(uri)?.clone().to_string();
        let tokens = Lexer::tokenize(&text)?;
        let parse_result = Parser::parse(&tokens);

        Some((text, tokens, parse_result))
    }

    fn get_builtin_types(&self) -> Iter<CompletionItem> {
        self.plugins
            .iter()
            .filter_map(|p| p.provide_types.map(|p| p()))
            .flatten()
            .chain(BUILTIN_TYPE_COMPLETIONS.iter())
    }

    fn get_builtin_annotations(&self) -> Iter<CompletionItem> {
        BUILTIN_ANNOTATION_COMPLETIONS.iter().cloned()
    }

    // Validate document and send diagnostics
    async fn validate_document(&self, uri: &Url) {
        let res = self.parse_document(uri);
        let doc = match res {
            Some(doc) => doc,
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
        let text = doc.0;
        let tokens = doc.1;
        let parse_result = doc.2;
        let ast = parse_result.0;
        let errors = parse_result.1;

        if errors.len() > 0 {
            let diagnostics = errors
                .iter()
                .map(|err| {
                    let start_pos = token_to_editor_location(err.location);
                    // For end position, add the length of the token
                    let end_pos = Position {
                        line: start_pos.line,
                        character: start_pos.character + err.location.v.len() as u32,
                    };

                    Diagnostic {
                        range: Range {
                            start: start_pos,
                            end: end_pos,
                        },
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: err.message.clone(),
                        source: Some("xenomorph".to_string()),
                        ..Default::default()
                    }
                })
                .collect();

            self.client
                .publish_diagnostics(uri.clone(), diagnostics, None)
                .await;
        }
    }

    // Find the token at a given position
    fn find_token_at_position(&self, tokens: &[Token], position: Position) -> Option<&Token> {
        tokens.iter().find(|(_, data)| {
            let token_start = token_to_editor_location(data);
            let token_end = Position {
                line: token_start.line,
                character: token_start.character + data.v.len() as u32,
            };

            position >= token_start && position <= token_end
        })
    }

    // Get completions based on context
    fn get_context_completions(&self, tokens: &[Token], position: Position) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        // Find the current token
        if let Some(current_token) = self.find_token_at_position(tokens, position) {
            match current_token.0 {
                TokenVariant::At => {
                    // After @ suggest annotations
                    items.extend(self.get_builtin_annotations());
                }
                TokenVariant::Colon => {
                    // After colon, suggest types
                    items.extend(vec![
                        // create_completion_item(
                        //     "string",
                        //     Some("String type"),
                        //     CompletionItemKind::TYPE_PARAMETER,
                        // ),
                    ]);
                }
                TokenVariant::LCurly => {
                    // Inside object definition, suggest common property names
                    items.extend(vec![
                        create_completion_item(
                            "id",
                            Some("Identifier property"),
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "name",
                            Some("Name property"),
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "type",
                            Some("Type property"),
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "value",
                            Some("Value property"),
                            CompletionItemKind::PROPERTY,
                        ),
                    ]);
                }
                _ => {
                    // Default completions
                }
            }
        }

        items
    }

    // Provide hover information for a token
    fn get_hover_for_token(&self, token: &Token) -> Option<Hover> {
        let range = Range {
            start: token_to_editor_location(&token.1),
            end: Position {
                line: token_to_editor_location(&token.1).line,
                character: token_to_editor_location(&token.1).character + token.1.v.len() as u32,
            },
        };

        let contents = match token.0 {
            TokenVariant::Type => {
                format!("**type**\n\nDefines a new type in Xenomorph.")
            }
            TokenVariant::Set => {
                format!("**set**\n\nDefines a set type that can contain elements of other types.")
            }
            TokenVariant::Enum => {
                format!("**enum**\n\nDefines an enumeration of possible values.")
            }
            TokenVariant::At => {
                format!("**@**\n\nIndicates the start of an annotation that provides metadata or validation rules.")
            }
            TokenVariant::Identifier => {
                if token.1.v.starts_with('@') {
                    // This is an annotation
                    let annotation = &token.1.v[1..]; // Remove the @ prefix
                    match annotation {
                        "min" => format!("**@min**\n\nSpecifies a minimum value for numeric types."),
                        "max" => format!("**@max**\n\nSpecifies a maximum value for numeric types."),
                        "len" => format!("**@len**\n\nSpecifies a length or length range for strings or arrays."),
                        "if" => format!("**@if**\n\nConditional validation that applies only when the condition is met."),
                        "elseif" => format!("**@elesif**\n\nConditional validation that applies only when the preceeding conditions aren't met and this condition is met."),
                        "else" => format!("**@else**\n\nAlternative validation when preceeding conditions are not met."),
                        _ => format!("**@{}**\n\nAnnotation that provides metadata or validation rules.", annotation),
                    }
                } else {
                    format!("**{}**\n\nIdentifier", token.1.v)
                }
            }
            _ => return None, // No hover for other token types
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: contents,
            }),
            range: Some(range),
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
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
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
        let text = Rc::new(params.text_document.text);

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
                    if let Some(range) = change.range {
                        // Would need to implement proper incremental updates
                        // For simplicity, we're replacing the entire document
                        *doc = Rc::new(change.text);
                    } else {
                        // Full document update
                        *doc = Rc::new(change.text);
                    }
                }
            }
        }

        // Clear cache and revalidate
        {
            let mut cache = self.ast_cache.lock().unwrap();
            cache.remove(&uri);
        }

        self.validate_document(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut documents = self.document_map.lock().unwrap();
        documents.remove(&params.text_document.uri);

        let mut cache = self.ast_cache.lock().unwrap();
        cache.remove(&params.text_document.uri);

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
            CompletionItem {
                label: "object literal".to_string(),
                kind: Some(CompletionItemKind::SNIPPET),
                detail: Some("Create a new object type".to_string()),
                insert_text: Some("{\n\t${1:property}: ${2:type},\n\t$0\n}".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            },
        ];

        // Context-aware completions
        // if let Some((tokens, _)) = self.parse_document(&uri) {
        //     let context_completions = self.get_context_completions(&tokens, position);
        //     completions.extend(context_completions);
        // }

        // Plugin-provided completions
        completions.extend(self.plugins.iter().flat_map(|p| (p.provide)()).map(|c| {
            CompletionItem {
                label: c.to_string(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some("Plugin provided".to_string()),
                ..Default::default()
            }
        }));

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

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // if let Some((tokens, _)) = self.parse_document(&uri) {
        //     if let Some(token) = self.find_token_at_position(&tokens, position) {
        //         return Ok(self.get_hover_for_token(token));
        //     }
        // }

        Ok(None)
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        if let Some(text) = {
            let documents = self.document_map.lock().unwrap();
            documents.get(&uri).cloned()
        } {
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

        let (tokens, ast) = {
            let cache = self.ast_cache.lock().unwrap();
            if let Some((_, cached_tokens, (ast, _))) = cache.get(&uri) {
                (cached_tokens.clone(), ast.clone())
            } else {
                return Ok(None);
            }
        };

        if let Some(token) = self.find_token_at_position(&tokens, position) {
            if token.0 == TokenVariant::Identifier {
                let searched_name = token.1.v;

                for decl in &ast {
                    match decl {
                        Declaration::TypeDecl { name, t } => {
                            if name.v == searched_name {
                                let range = Range {
                                    start: Position {
                                        line: (name.l - 1) as u32,
                                        character: (name.c - 1) as u32,
                                    },
                                    end: Position {
                                        line: (name.l - 1) as u32,
                                        character: (name.c - 1 + name.v.len()) as u32,
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
            }
        }

        Ok(None)
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
        ast_cache: Mutex::new(HashMap::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
