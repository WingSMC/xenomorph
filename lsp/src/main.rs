use crate::formatter::format_xenomorph;
use std::collections::HashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::{
    lexer::{Token, TokenVariant},
    module::{
        types::{DeclarationInfo, ErrorPhase},
        XenoRegistry,
    },
    parser::Declaration,
    TokenData,
};
use xenomorph_lsp_common::types::{
    create_completion_item, BUILTIN_ANNOTATION_COMPLETIONS, BUILTIN_TYPE_COMPLETIONS,
};

mod formatter;

struct Backend {
    client: Client,
    registry: XenoRegistry,
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
    // ── Path helpers ─────────────────────────────────────────────────

    /// Converts a file URI to a module path via the registry.
    fn uri_to_module_path(&self, uri: &Url) -> Option<String> {
        let file_path = uri.to_file_path().ok()?;
        self.registry.abs_path_to_module_path(&file_path)
    }

    // ── Completion helpers ──────────────────────────────────────────

    fn get_builtin_types(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.registry
            .plugins
            .iter()
            .filter_map(|p| p.provide_types.map(|f| f()))
            .flatten()
            .map(|pc| create_completion_item(pc.label, pc.detail, CompletionItemKind::CLASS))
            .chain(BUILTIN_TYPE_COMPLETIONS.iter().cloned())
    }

    fn get_builtin_annotations(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.registry
            .plugins
            .iter()
            .filter_map(|p| p.provide_annotations.map(|f| f()))
            .flatten()
            .map(|pc| create_completion_item(pc.label, pc.detail, CompletionItemKind::FUNCTION))
            .chain(BUILTIN_ANNOTATION_COMPLETIONS.iter().cloned())
    }

    /// Returns completion items for all declarations visible from the given module
    /// (its own declarations + declarations from imported modules).
    fn get_module_completions(&self, module_path: &str) -> Vec<CompletionItem> {
        self.registry
            .get_all_declarations_in_scope(module_path)
            .into_iter()
            .map(|info| {
                let mut item = create_completion_item(
                    &info.name,
                    info.docs.as_deref(),
                    CompletionItemKind::CLASS,
                );
                if info.module_path != module_path {
                    item.detail = Some(format!(
                        "{} (from {})",
                        item.detail.unwrap_or_default(),
                        info.module_path
                    ));
                }
                item
            })
            .collect()
    }

    /// Returns completion items for import path suggestions.
    fn get_import_completions(&self, path_so_far: &str) -> Vec<CompletionItem> {
        self.registry
            .suggest_import(path_so_far)
            .into_iter()
            .map(|(name, _, is_dir)| {
                let kind = if is_dir {
                    CompletionItemKind::FOLDER
                } else {
                    CompletionItemKind::MODULE
                };
                CompletionItem {
                    label: name.clone(),
                    kind: Some(kind),
                    detail: Some(if is_dir {
                        "directory".to_string()
                    } else {
                        "module".to_string()
                    }),
                    // For directories, append / to keep completing
                    insert_text: if is_dir {
                        Some(format!("{}/", name))
                    } else {
                        None
                    },
                    // Retrigger completion after inserting a directory
                    command: if is_dir {
                        Some(Command {
                            title: "Trigger completion".to_string(),
                            command: "editor.action.triggerSuggest".to_string(),
                            arguments: None,
                        })
                    } else {
                        None
                    },
                    ..Default::default()
                }
            })
            .collect()
    }

    /// Walks backward from a token to collect an import path like "foo/bar".
    /// Returns None if the token chain doesn't trace back to an Import token.
    fn collect_import_path(tokens: &[Token], current_token: &Token) -> Option<String> {
        let idx = tokens.iter().position(|t| {
            t.1.l == current_token.1.l && t.1.c == current_token.1.c && t.0 == current_token.0
        })?;

        // Walk backward collecting Identifier and Slash tokens
        let mut segments: Vec<&str> = Vec::new();
        let mut i = idx;
        loop {
            if i == 0 {
                return None;
            }
            i -= 1;
            match tokens[i].0 {
                TokenVariant::Identifier => segments.push(tokens[i].1.v),
                TokenVariant::Slash => continue,
                TokenVariant::Import => break,
                _ => return None,
            }
        }
        segments.reverse();
        Some(segments.join("/"))
    }

    // ── Token helpers ───────────────────────────────────────────────

    fn find_token_at_position<'a>(
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        tokens.iter().find(|(_, data)| {
            let token_range = data.to_editor_range();
            token_range.start <= position && position < token_range.end
        })
    }

    fn find_token_before_or_at_position<'a>(
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        Self::find_token_at_position(tokens, position).or_else(|| {
            tokens.iter().rev().find(|(_, data)| {
                let end = data.to_editor_range().end;
                end.line < position.line
                    || (end.line == position.line && end.character <= position.character)
            })
        })
    }

    // ── Document validation ─────────────────────────────────────────

    /// Reloads the module in the registry from the given source text,
    /// then publishes diagnostics.
    async fn validate_document(&self, uri: &Url, source: String) {
        let file_path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };

        let errors = self.registry.load_module_from_source(&file_path, source);

        let diagnostics: Vec<Diagnostic> = errors
            .iter()
            .filter_map(|err| {
                let (line, col, len) = err.location?;
                Some(Diagnostic {
                    range: Range {
                        start: Position {
                            line,
                            character: col,
                        },
                        end: Position {
                            line,
                            character: col + len,
                        },
                    },
                    severity: Some(match err.phase {
                        ErrorPhase::Lexer | ErrorPhase::Parser | ErrorPhase::Module => {
                            DiagnosticSeverity::ERROR
                        }
                        ErrorPhase::Analyzer => DiagnosticSeverity::ERROR,
                    }),
                    message: err.message.clone(),
                    source: Some("xenomorph".to_string()),
                    ..Default::default()
                })
            })
            .collect();

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    // ── Completions ─────────────────────────────────────────────────

    fn get_context_completions(
        &self,
        tokens: &[Token],
        _ast: &[Declaration],
        position: Position,
        module_path: Option<&str>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        let add_top_level_snippets = |items: &mut Vec<CompletionItem>| {
            items.push(CompletionItem {
                label: "type".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Declare a new type".to_string()),
                insert_text: Some("type ${1:Name} = ${0};".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
            items.push(CompletionItem {
                label: "import".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Import a module".to_string()),
                insert_text: Some("import ${1:module};".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        };

        let all_types = || -> Vec<CompletionItem> {
            let mut types: Vec<CompletionItem> = self.get_builtin_types().collect();
            if let Some(mp) = module_path {
                types.extend(self.get_module_completions(mp));
            }
            let mut seen = std::collections::HashSet::new();
            types.retain(|item| seen.insert(item.label.clone()));
            types
        };

        if let Some(current_token) = Self::find_token_before_or_at_position(tokens, position) {
            match current_token.0 {
                TokenVariant::At => {
                    items.extend(self.get_builtin_annotations());
                }
                TokenVariant::Or | TokenVariant::Colon => {
                    items.extend(all_types());
                }
                TokenVariant::Eq => {
                    items.push(CompletionItem {
                        label: "struct".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Create a new struct type".to_string()),
                        insert_text: Some("{\n\t${1:property}: ${2:type},\n\t$0\n}".to_string()),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                    items.push(CompletionItem {
                        label: "enum".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Create a new enum type".to_string()),
                        insert_text: Some(
                            "enum {\n\t${1:variant}: ${2:type},\n\t$0\n}".to_string(),
                        ),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                    items.extend(all_types());
                }
                TokenVariant::Semicolon => {
                    add_top_level_snippets(&mut items);
                }
                TokenVariant::Import => {
                    items.extend(self.get_import_completions(""));
                }
                TokenVariant::Slash => {
                    // Check if we're in an import path: walk back to collect segments
                    if let Some(path) = Self::collect_import_path(tokens, current_token) {
                        items.extend(self.get_import_completions(&format!("{}/", path)));
                    }
                }
                TokenVariant::Identifier => {
                    let token_idx = tokens.iter().position(|t| {
                        t.1.l == current_token.1.l
                            && t.1.c == current_token.1.c
                            && t.1.v == current_token.1.v
                            && t.0 == current_token.0
                    });

                    let prev_variant = token_idx
                        .and_then(|idx| idx.checked_sub(1))
                        .and_then(|idx| tokens.get(idx))
                        .map(|t| t.0);

                    match prev_variant {
                        Some(TokenVariant::Import) => {
                            // Typing first segment of import path
                            items.extend(self.get_import_completions(""));
                        }
                        Some(TokenVariant::Slash) => {
                            // Typing a segment after slash in import path
                            if let Some(path) = Self::collect_import_path(tokens, current_token) {
                                // path includes current identifier; use parent path
                                let parent = path.rsplitn(2, '/').last().unwrap_or("");
                                items.extend(self.get_import_completions(&format!("{}/", parent)));
                            }
                        }
                        Some(TokenVariant::Colon) | Some(TokenVariant::Or) => {
                            items.extend(all_types());
                        }
                        _ => {
                            items.extend(self.get_builtin_annotations());
                        }
                    }
                }
                TokenVariant::RParen => {
                    items.extend(self.get_builtin_annotations());
                }
                TokenVariant::LCurly | TokenVariant::Comma => {
                    items.push(CompletionItem {
                        label: "property".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Add a property".to_string()),
                        insert_text: Some("${1:name}: ${2:type},".to_string()),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                }
                _ => {
                    items.extend(all_types());
                    items.extend(self.get_builtin_annotations());
                }
            }
        } else {
            add_top_level_snippets(&mut items);
        }

        items
    }

    // ── Hover ───────────────────────────────────────────────────────

    fn get_hover_for_location(
        &self,
        tokens: &[Token],
        ast: &[Declaration],
        position: Position,
        module_path: Option<&str>,
    ) -> Option<Hover> {
        let token = Self::find_token_at_position(tokens, position)?;

        if token.0 != TokenVariant::Identifier {
            return None;
        }

        let searched_name = token.1.v;

        // 1. Check local AST declarations
        for decl in ast {
            if let Declaration::TypeDecl { name, docs, .. } = decl {
                if name.v == searched_name {
                    let contents = format!("**{}**\n\n{}", name.v, docs.unwrap_or(""));
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: contents,
                        }),
                        range: Some(token.1.to_editor_range()),
                    });
                }
            }
        }

        // 2. Check builtins
        let builtin_info = self
            .get_builtin_types()
            .find(|item| item.label == searched_name)
            .or_else(|| {
                self.get_builtin_annotations()
                    .find(|item| item.label == searched_name)
            });

        if let Some(i) = builtin_info {
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("**{}**\n\n{}", i.label, i.detail.unwrap_or_default()),
                }),
                range: Some(token.1.to_editor_range()),
            });
        }

        // 3. Check cross-module declarations via the registry
        let current_module = module_path.unwrap_or("");
        let info = self
            .registry
            .find_declaration(current_module, searched_name)?;
        let docs = info.docs.as_deref().unwrap_or("");
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "**{}** *(from {})*\n\n{}",
                    info.name, info.module_path, docs
                ),
            }),
            range: Some(token.1.to_editor_range()),
        })
    }

    // ── Goto Definition helpers ─────────────────────────────────────

    fn declaration_info_to_location(info: &DeclarationInfo) -> Option<Location> {
        let target_uri = Url::from_file_path(&info.abs_path).ok()?;
        Some(Location {
            uri: target_uri,
            range: Range {
                start: Position {
                    line: info.line,
                    character: info.column,
                },
                end: Position {
                    line: info.line,
                    character: info.column + info.name_len,
                },
            },
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
                        "|".to_string(),
                        ".".to_string(),
                        ":".to_string(),
                        "@".to_string(),
                        "{".to_string(),
                        " ".to_string(),
                    ]),
                    all_commit_characters: None,
                    work_done_progress_options: Default::default(),
                    completion_item: None,
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // Load the entry module and its transitive imports on startup
        let errors = self
            .registry
            .load_module(&[&self.registry.entry], true, None);
        for e in &errors {
            self.client
                .log_message(MessageType::WARNING, format!("Module error: {}", e))
                .await;
        }

        self.client
            .log_message(MessageType::INFO, "Xenomorph Language Server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.validate_document(&uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        // With TextDocumentSyncKind::FULL, last change contains the full text
        if let Some(change) = params.content_changes.into_iter().last() {
            self.validate_document(&uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);

        let completions =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    self.get_context_completions(tokens, ast, position, module_path.as_deref())
                });

        Ok(Some(CompletionResponse::Array(
            completions.unwrap_or_default(),
        )))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        Ok(item)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let module_path = self.uri_to_module_path(&uri);

        let hover =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    self.get_hover_for_location(tokens, ast, position, module_path.as_deref())
                });

        Ok(hover.flatten())
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |_, _, module| {
                    let source = module.borrow_source();
                    let formatted = format_xenomorph(source);

                    vec![TextEdit {
                        range: Range {
                            start: Position {
                                line: 0,
                                character: 0,
                            },
                            end: Position {
                                line: source.lines().count() as u32,
                                character: 0,
                            },
                        },
                        new_text: formatted,
                    }]
                });

        Ok(result)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let module_path = self.uri_to_module_path(&uri);
        let mp = module_path.as_deref().unwrap_or("");

        // First try: local definition or import navigation
        let local_result = self.registry.with_module(mp, |tokens, ast, _| {
            let token = Self::find_token_at_position(tokens, position)?;

            // If cursor is on an import line, navigate to the imported file
            if token.0 == TokenVariant::Identifier {
                for decl in ast.iter() {
                    if let Declaration::Import { path, location } = decl {
                        if token.1.l == location.l {
                            let segments: Vec<&str> = path.iter().copied().collect();
                            if let Ok((_, abs_path)) = self.registry.resolve_import(&segments, None)
                            {
                                if abs_path.exists() {
                                    if let Ok(target_uri) = Url::from_file_path(&abs_path) {
                                        return Some(GotoDefinitionResponse::Scalar(Location {
                                            uri: target_uri,
                                            range: Range::default(),
                                        }));
                                    }
                                }
                            }
                            return None;
                        }
                    }
                }
            }

            // Try local declaration
            if token.0 == TokenVariant::Identifier {
                for decl in ast {
                    if let Declaration::TypeDecl { name, .. } = decl {
                        if name.v == token.1.v {
                            return Some(GotoDefinitionResponse::Scalar(Location {
                                uri: uri.clone(),
                                range: name.to_editor_range(),
                            }));
                        }
                    }
                }
            }

            None
        });

        if let Some(Some(response)) = local_result {
            return Ok(Some(response));
        }

        // Second try: cross-module declaration via the registry
        let cross_result = self.registry.with_module(mp, |tokens, _, _| {
            let token = Self::find_token_at_position(tokens, position)?;
            if token.0 != TokenVariant::Identifier {
                return None;
            }
            let info = self.registry.find_declaration(mp, token.1.v)?;
            Self::declaration_info_to_location(&info).map(GotoDefinitionResponse::Scalar)
        });

        Ok(cross_result.flatten())
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, _, _| {
                    let token = Self::find_token_at_position(tokens, position)?;
                    Some(
                        tokens
                            .iter()
                            .filter_map(|t| {
                                if t.0 == TokenVariant::Identifier && t.1.v == token.1.v {
                                    Some(Location {
                                        uri: uri.clone(),
                                        range: t.1.to_editor_range(),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<Location>>(),
                    )
                });

        Ok(result.flatten())
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let module_path = self.uri_to_module_path(&uri);

        let symbols =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |_, ast, _| {
                    #[allow(deprecated)]
                    ast.iter()
                        .filter_map(|decl| match decl {
                            Declaration::Import { .. } => None,
                            Declaration::TypeDecl { name, .. } => Some(SymbolInformation {
                                name: name.v.to_string(),
                                kind: SymbolKind::STRUCT,
                                tags: None,
                                deprecated: None,
                                location: Location {
                                    uri: uri.clone(),
                                    range: name.to_editor_range(),
                                },
                                container_name: None,
                            }),
                        })
                        .collect::<Vec<SymbolInformation>>()
                });

        Ok(symbols.map(DocumentSymbolResponse::Flat))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    let token = Self::find_token_at_position(tokens, position)?;
                    if token.0 != TokenVariant::Identifier {
                        return None;
                    }

                    // Only allow renaming user-defined declarations
                    let is_user_defined = ast.iter().any(|decl| match decl {
                        Declaration::Import { .. } => false,
                        Declaration::TypeDecl { name, .. } => name.v == token.1.v,
                    });

                    if !is_user_defined {
                        return None;
                    }

                    Some(PrepareRenameResponse::RangeWithPlaceholder {
                        range: token.1.to_editor_range(),
                        placeholder: token.1.v.to_string(),
                    })
                });

        Ok(result.flatten())
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    let token = Self::find_token_at_position(tokens, position)?;
                    if token.0 != TokenVariant::Identifier {
                        return None;
                    }

                    let old_name = token.1.v;

                    let is_user_defined = ast.iter().any(|decl| match decl {
                        Declaration::Import { .. } => false,
                        Declaration::TypeDecl { name, .. } => name.v == old_name,
                    });

                    if !is_user_defined {
                        return None;
                    }

                    let edits: Vec<TextEdit> = tokens
                        .iter()
                        .filter(|t| t.0 == TokenVariant::Identifier && t.1.v == old_name)
                        .map(|t| TextEdit {
                            range: t.1.to_editor_range(),
                            new_text: new_name.clone(),
                        })
                        .collect();

                    if edits.is_empty() {
                        return None;
                    }

                    let mut changes = HashMap::new();
                    changes.insert(uri.clone(), edits);

                    Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    })
                });

        Ok(result.flatten())
    }
}

#[tokio::main]
async fn main() {
    let reg = match XenoRegistry::new(false) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    let (service, socket) = LspService::new(move |client| Backend {
        client,
        registry: reg,
    });

    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}
