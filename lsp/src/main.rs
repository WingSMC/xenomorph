use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::parser::parser::parse;
use xenomorph_common::{
    lexer::tokens::{Token, TokenData, TokenVariant},
    parser::{parser::ParseError, parser_expr::Declaration},
    plugins::{load_plugins, Plugin},
};

type AstCache<'src> = (
    Arc<String>,
    Vec<Declaration<'src>>,
    Vec<ParseError<'src>>,
    Box<Vec<Token<'src>>>,
);

#[derive(Debug)]
struct Backend<'src> {
    client: Client,
    plugins: Vec<&'static Plugin<'static>>,
    document_map: Mutex<HashMap<Url, Arc<String>>>,
    ast_cache: Mutex<HashMap<Url, AstCache<'src>>>,
}

// Helper function to convert parser location to LSP position
fn token_to_position(token: &TokenData) -> Position {
    // Convert from 1-based to 0-based for LSP
    Position {
        line: (token.l - 1) as u32,
        character: (token.c - 1) as u32,
    }
}

impl<'src> Backend<'src> {
    fn parse_document(&'src self, uri: &Url) -> Option<()> {
        let text_arc = {
            let documents = self.document_map.lock().unwrap();
            documents.get(uri)?.clone() // Assuming this returns Arc<String>
        };

        // Store Arc first with placeholder parse results
        {
            let mut cache = self.ast_cache.lock().unwrap();
            cache.insert(
                uri.clone(),
                (
                    text_arc.clone(),
                    Vec::new(),
                    Vec::new(),
                    Box::new(Vec::new()),
                ),
            );
        }

        let parse_result = {
            let cache = self.ast_cache.lock().unwrap();
            let (stored_text, _, _, _) = cache.get(uri)?;
            parse(stored_text.as_str()) // Parse from the cached Arc<String>
        };

        {
            let mut cache = self.ast_cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(uri) {
                entry.1 = parse_result.0; // declarations
                entry.2 = parse_result.1; // errors
                entry.3 = parse_result.2; // tokens
            }
        }

        Some(())
    }

    // Validate document and send diagnostics
    async fn validate_document(&self, uri: &Url) {
        if let Some((_, parse_result)) = self.parse_document(uri) {
            let diagnostics = match parse_result {
                Err(errors) => {
                    // Convert parser errors to LSP diagnostics
                    errors
                        .iter()
                        .map(|err| {
                            let start_pos = if let Some(token) = err.token {
                                token_to_position(&token.1)
                            } else {
                                Position {
                                    line: 0,
                                    character: 0,
                                }
                            };

                            // For end position, add the length of the token
                            let end_pos = if let Some(token) = err.token {
                                Position {
                                    line: start_pos.line,
                                    character: start_pos.character + token.1.v.len() as u32,
                                }
                            } else {
                                Position {
                                    line: 0,
                                    character: 1,
                                }
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
                        .collect()
                }
                Ok(_) => vec![], // No errors
            };

            // Send diagnostics to client
            self.client
                .publish_diagnostics(uri.clone(), diagnostics, None)
                .await;
        }
    }

    fn find_token_at_position(
        &'src self,
        tokens: &'src [Token<'src>],
        position: Position,
    ) -> Option<&Token<'src>> {
        tokens.iter().find(|(_, data)| {
            let token_start = token_to_position(data);
            let token_end = Position {
                line: token_start.line,
                character: token_start.character + data.v.len() as u32,
            };

            position >= token_start && position <= token_end
        })
    }

    fn get_context_completions(
        &'src self,
        tokens: &'src [Token<'src>],
        position: Position,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        // Find the current token
        if let Some(current_token) = self.find_token_at_position(tokens, position) {
            match current_token.0 {
                TokenVariant::At => {
                    // After @ suggest annotations
                    items.extend(vec![
                        create_completion_item(
                            "min",
                            "Minimum value constraint",
                            CompletionItemKind::FUNCTION,
                        ),
                        create_completion_item(
                            "max",
                            "Maximum value constraint",
                            CompletionItemKind::FUNCTION,
                        ),
                        create_completion_item(
                            "len",
                            "Length constraint",
                            CompletionItemKind::FUNCTION,
                        ),
                        create_completion_item(
                            "minlen",
                            "Minimum length constraint",
                            CompletionItemKind::FUNCTION,
                        ),
                        create_completion_item(
                            "maxlen",
                            "Maximum length constraint",
                            CompletionItemKind::FUNCTION,
                        ),
                        create_completion_item(
                            "if",
                            "Conditional validation",
                            CompletionItemKind::KEYWORD,
                        ),
                        create_completion_item(
                            "else",
                            "Alternative validation",
                            CompletionItemKind::KEYWORD,
                        ),
                    ]);
                }
                TokenVariant::Colon => {
                    // After colon, suggest types
                    items.extend(vec![
                        create_completion_item(
                            "string",
                            "String type",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                        create_completion_item(
                            "u8",
                            "8-bit unsigned integer",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                        create_completion_item(
                            "u64",
                            "64-bit unsigned integer",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                        create_completion_item(
                            "bool",
                            "Boolean type",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                        create_completion_item(
                            "float",
                            "Floating point number",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                        create_completion_item(
                            "Date",
                            "Date type",
                            CompletionItemKind::TYPE_PARAMETER,
                        ),
                    ]);
                }
                TokenVariant::LCurly => {
                    // Inside object definition, suggest common property names
                    items.extend(vec![
                        create_completion_item(
                            "id",
                            "Identifier property",
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "name",
                            "Name property",
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "type",
                            "Type property",
                            CompletionItemKind::PROPERTY,
                        ),
                        create_completion_item(
                            "value",
                            "Value property",
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
            start: token_to_position(&token.1).clone(),
            end: Position {
                line: token_to_position(&token.1).line.clone(),
                character: token_to_position(&token.1).character + token.1.v.len() as u32,
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

fn create_completion_item(label: &str, detail: &str, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend<'static> {
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
            create_completion_item("type", "Define a new type", CompletionItemKind::KEYWORD),
            create_completion_item(
                "validator",
                "Define a validator",
                CompletionItemKind::KEYWORD,
            ),
            create_completion_item("set", "Define a set", CompletionItemKind::KEYWORD),
            create_completion_item("enum", "Define an enumeration", CompletionItemKind::KEYWORD),
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
        if let Some((tokens, _)) = self.parse_document(&uri) {
            let context_completions = self.get_context_completions(&tokens, position);
            completions.extend(context_completions);
        }

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

        if self.parse_document(&uri).is_some() {
            let cache = self.ast_cache.lock().unwrap();
            let cached = cache.get(&uri).unwrap();
            if let Some(token) = self.find_token_at_position(&cached.3, position) {
                return Ok(self.get_hover_for_token(token));
            }
        }

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

        if let Some((tokens, Ok(ast))) = self.parse_document(&uri) {
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
