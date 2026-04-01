use crate::formatter::format_xenomorph;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::{
    lexer::{Lexer, Token, TokenVariant},
    module::{
        build_declaration_cache, load_module, new_registry, resolve_import, DeclarationInfo,
        SharedModuleRegistry,
    },
    parser::{Declaration, Parser},
    plugins::{load_plugins, XenoPlugin},
    TokenData, XenoError,
};
use xenomorph_lsp_common::types::{
    create_completion_item, BUILTIN_ANNOTATION_COMPLETIONS, BUILTIN_TYPE_COMPLETIONS,
};

mod formatter;

#[derive(Debug)]
struct Backend {
    client: Client,
    plugins: Vec<&'static XenoPlugin<'static>>,
    document_map: Mutex<HashMap<Url, Arc<String>>>,
    module_registry: SharedModuleRegistry,
    /// Cached declaration info from all loaded modules.
    declaration_cache: Mutex<HashMap<String, DeclarationInfo>>,
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
            Declaration::Import { .. } => None,
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

    fn get_builtin_annotations(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.plugins
            .iter()
            .filter_map(|p| p.provide_annotations.map(|f| f()))
            .flatten()
            .map(|pc| create_completion_item(pc.label, pc.detail, CompletionItemKind::FUNCTION))
            .chain(BUILTIN_ANNOTATION_COMPLETIONS.iter().cloned())
    }

    fn with_tokens<T, F>(&self, uri: &Url, f: F) -> Option<T>
    where
        F: for<'src> FnOnce(&Vec<Token<'src>>) -> T,
    {
        let text = self.get_document_text(uri)?;
        let tokens = Lexer::tokenize(text.as_str()).ok()?;
        Some(f(&tokens))
    }
    fn with_parsed_document<T, F>(&self, uri: &Url, f: F) -> Option<T>
    where
        F: for<'src> FnOnce(&Vec<Token<'src>>, &Vec<Declaration<'src>>) -> T,
    {
        let text = self.get_document_text(uri)?;
        let tokens = Lexer::tokenize(text.as_str()).ok()?;
        let (ast, _errors) = Parser::parse(&tokens);
        Some(f(&tokens, &ast))
    }

    fn position_to_byte_offset(text: &str, position: Position) -> Option<usize> {
        let mut line: u32 = 0;
        let mut col_utf16: u32 = 0;

        if position.line == 0 && position.character == 0 {
            return Some(0);
        }

        for (idx, ch) in text.char_indices() {
            if line == position.line && col_utf16 == position.character {
                return Some(idx);
            }

            if ch == '\n' {
                if line == position.line {
                    return Some(idx);
                }
                line += 1;
                col_utf16 = 0;
                continue;
            }

            if line == position.line {
                col_utf16 += ch.len_utf16() as u32;
                if col_utf16 > position.character {
                    return Some(idx + ch.len_utf8());
                }
            }
        }

        if line == position.line && col_utf16 == position.character {
            return Some(text.len());
        }

        None
    }

    fn apply_content_change(text: &mut String, change: &TextDocumentContentChangeEvent) {
        match change.range {
            None => {
                *text = change.text.clone();
            }
            Some(range) => {
                let start = Self::position_to_byte_offset(text, range.start);
                let end = Self::position_to_byte_offset(text, range.end);

                match (start, end) {
                    (Some(start), Some(end)) if start <= end && end <= text.len() => {
                        text.replace_range(start..end, &change.text);
                    }
                    _ => {
                        // Fallback to full replacement when range mapping fails.
                        *text = change.text.clone();
                    }
                }
            }
        }
    }

    async fn validate_document(&self, uri: &Url) {
        let text = match self.get_document_text(uri) {
            Some(t) => t,
            None => return,
        };

        let make_diagnostics = |errors: &[XenoError<'_>]| -> Vec<Diagnostic> {
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
                let (ast, errors) = Parser::parse(&tokens);
                let semantic_errors = xenomorph_common::semantic::analyze(&ast);

                // Validate imports: check if imported modules exist on disk
                let mut import_errors: Vec<XenoError> = Vec::new();
                if let Some(file_path) = uri.to_file_path().ok() {
                    let workspace_root = file_path.parent().unwrap_or(Path::new("."));
                    for decl in &ast {
                        if let Declaration::Import { path, location } = decl {
                            let segments: Vec<&str> = path.iter().map(|s| s.as_ref()).collect();
                            match resolve_import(&segments, &file_path, workspace_root) {
                                Ok((_, abs_path)) => {
                                    if !abs_path.exists() {
                                        import_errors.push(XenoError {
                                            location: (*location).clone(),
                                            message: format!(
                                                "Module '{}' not found (expected at '{}')",
                                                path.join("/"),
                                                abs_path.display()
                                            ),
                                        });
                                    }
                                }
                                Err(_) => {
                                    import_errors.push(XenoError {
                                        location: (*location).clone(),
                                        message: format!(
                                            "Cannot resolve module '{}'",
                                            path.join("/")
                                        ),
                                    });
                                }
                            }
                        }
                    }

                    // Load the module graph and update the declaration cache
                    self.reload_modules(&file_path);
                }

                make_diagnostics(&[errors, semantic_errors, import_errors].concat())
            }
        };

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// Load/reload the module graph starting from the given file and rebuild the
    /// declaration cache.
    fn reload_modules(&self, file_path: &Path) {
        let workspace_root = file_path.parent().unwrap_or(Path::new("."));

        // Clear and reload the registry
        {
            let mut reg = self.module_registry.write().unwrap();
            reg.modules.clear();
        }

        let _errors = load_module(file_path, workspace_root, &self.module_registry);

        // Rebuild declaration cache
        let new_cache = build_declaration_cache(&self.module_registry);
        {
            let mut cache = self.declaration_cache.lock().unwrap();
            *cache = new_cache;
        }
    }

    /// Get completion items from the cross-module declaration cache.
    fn get_module_completions(&self) -> Vec<CompletionItem> {
        let cache = self.declaration_cache.lock().unwrap();
        cache
            .values()
            .map(|info| {
                create_completion_item(&info.name, info.docs.as_deref(), CompletionItemKind::CLASS)
            })
            .collect()
    }

    fn find_token_at_position<'a>(
        &self,
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        tokens.iter().find(|(_, data)| {
            let token_range = data.to_editor_range();
            token_range.start <= position && position < token_range.end
        })
    }

    fn find_token_before_or_at_position<'a>(
        &self,
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        self.find_token_at_position(tokens, position).or_else(|| {
            tokens.iter().rev().find(|(_, data)| {
                let end = data.to_editor_range().end;
                end.line < position.line
                    || (end.line == position.line && end.character <= position.character)
            })
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
                Declaration::Import { .. } => {}
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
            Declaration::Import { .. } => return None,
            Declaration::TypeDecl { name, .. } => name,
        });
    }

    fn get_context_completions(
        &self,
        tokens: &[Token],
        ast: &Vec<Declaration>,
        position: Position,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        let add_top_level_snippets = |items: &mut Vec<CompletionItem>| {
            items.push(CompletionItem {
                label: "type".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Declare a new type".to_string()),
                insert_text: Some("type ${1:Name} = ${0}".to_string()),
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

        // Get all type completions (local + imported modules + builtins)
        let all_types = || -> Vec<CompletionItem> {
            let mut types: Vec<CompletionItem> = self
                .ast_to_definition_completions(ast)
                .chain(self.get_builtin_types())
                .collect();
            types.extend(self.get_module_completions());
            // Deduplicate by label (local declarations take precedence)
            let mut seen = std::collections::HashSet::new();
            types.retain(|item| seen.insert(item.label.clone()));
            types
        };

        if let Some(current_token) = self.find_token_before_or_at_position(tokens, position) {
            match current_token.0 {
                // After @, suggest annotations
                TokenVariant::At => {
                    items.extend(self.get_builtin_annotations());
                }
                // After | or :, suggest types (local + imported + builtins)
                TokenVariant::Or | TokenVariant::Colon => {
                    items.extend(all_types());
                }
                // After =, suggest struct snippet + types
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
                // Between declarations (top-level), only suggest declaration snippets
                TokenVariant::Semicolon => {
                    add_top_level_snippets(&mut items);
                }
                // After import keyword, no completions (path-based)
                TokenVariant::Import => {}
                // After/inside an identifier: use previous token context
                // so Ctrl+Space after ':' still suggests types.
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
                        Some(TokenVariant::Colon) | Some(TokenVariant::Or) => {
                            items.extend(all_types());
                        }
                        _ => {
                            items.extend(self.get_builtin_annotations());
                        }
                    }
                }
                // After ), suggest annotations (e.g. after @annotation())
                TokenVariant::RParen => {
                    items.extend(self.get_builtin_annotations());
                }
                // After {, suggest property snippets
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
                    // Default: show types + annotations
                    items.extend(all_types());
                    items.extend(self.get_builtin_annotations());
                }
            }
        } else {
            // No previous token (beginning of file) — only declaration snippets
            add_top_level_snippets(&mut items);
        }

        items
    }

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
        let def_opt = self.find_declaration(location, tokens, ast);

        let contents = match def_opt {
            Some(d) => match d {
                Declaration::TypeDecl { name, docs, .. } if name.v == searched_name => {
                    format!("**{}**\n\n{}", name.v, docs.unwrap_or(""))
                }
                _ => return None,
            },
            None => {
                // Check builtins first
                let builtin_info = self
                    .get_builtin_types()
                    .find(|item| item.label == searched_name)
                    .or_else(|| {
                        self.get_builtin_annotations()
                            .find(|item| item.label == searched_name)
                    });

                match builtin_info {
                    Some(i) => format!("**{}**\n\n{}", i.label, i.detail.unwrap_or_default()),
                    None => {
                        // Check the cross-module declaration cache
                        let cache = self.declaration_cache.lock().unwrap();
                        match cache.get(searched_name) {
                            Some(info) => {
                                let docs = info.docs.as_deref().unwrap_or("");
                                format!(
                                    "**{}** *(from {})*\n\n{}",
                                    info.name, info.module_path, docs
                                )
                            }
                            None => return None,
                        }
                    }
                }
            }
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
            let mut updated = documents
                .get(&uri)
                .map(|d| d.as_ref().clone())
                .unwrap_or_default();

            for change in &params.content_changes {
                Self::apply_content_change(&mut updated, change);
            }

            documents.insert(uri.clone(), Arc::new(updated));
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

        let mut completions = Vec::new();

        if let Some(context_completions) = self.with_parsed_document(&uri, |tokens, ast| {
            self.get_context_completions(tokens, ast, position)
        }) {
            completions.extend(context_completions);
        }

        Ok(Some(CompletionResponse::Array(completions)))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        Ok(item)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let hover = self.with_parsed_document(&uri, |tokens, ast| {
            self.get_hover_for_location(position, tokens, ast)
        });

        Ok(hover.flatten())
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

        // First try: local definition in the same file
        let local_result = self.with_parsed_document(&uri, |tokens, ast| {
            let token = self.find_token_at_position(tokens, position)?;

            // If cursor is on the import path (identifier after `import` keyword),
            // resolve the import and jump to that file.
            if token.0 == TokenVariant::Identifier {
                for decl in ast.iter() {
                    if let Declaration::Import { path, location } = decl {
                        // Check if the cursor token is part of this import declaration.
                        // The import path identifiers are on the same line as the import keyword.
                        let import_line = location.l;
                        if token.1.l == import_line {
                            // Resolve the import to a file path
                            if let Some(file_path) = uri.to_file_path().ok() {
                                let workspace_root = file_path.parent().unwrap_or(Path::new("."));
                                let segments: Vec<&str> = path.iter().map(|s| s.as_ref()).collect();
                                if let Ok((_, abs_path)) =
                                    resolve_import(&segments, &file_path, workspace_root)
                                {
                                    if abs_path.exists() {
                                        if let Ok(target_uri) = Url::from_file_path(&abs_path) {
                                            return Some(GotoDefinitionResponse::Scalar(
                                                Location {
                                                    uri: target_uri,
                                                    range: Range {
                                                        start: Position {
                                                            line: 0,
                                                            character: 0,
                                                        },
                                                        end: Position {
                                                            line: 0,
                                                            character: 0,
                                                        },
                                                    },
                                                },
                                            ));
                                        }
                                    }
                                }
                            }
                            return None;
                        }
                    }
                }
            }

            // Try local declaration in the same file
            let definition = self.find_definition(position, tokens, ast)?;
            Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: definition.to_editor_range(),
            }))
        });

        if let Some(Some(response)) = local_result {
            return Ok(Some(response));
        }

        // Second try: check the cross-module declaration cache for imported types
        let cross_module_result = self.with_tokens(&uri, |tokens| {
            let token = self.find_token_at_position(tokens, position)?;
            if token.0 != TokenVariant::Identifier {
                return None;
            }

            let searched_name = token.1.v;
            let cache = self.declaration_cache.lock().unwrap();
            let info = cache.get(searched_name)?;

            let target_uri = Url::from_file_path(&info.abs_path).ok()?;
            let target_range = Range {
                start: Position {
                    line: info.line,
                    character: info.column,
                },
                end: Position {
                    line: info.line,
                    character: info.column + info.name_len,
                },
            };

            Some(GotoDefinitionResponse::Scalar(Location {
                uri: target_uri,
                range: target_range,
            }))
        });

        Ok(cross_module_result.flatten())
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let result = self
            .with_tokens(&uri, |tokens| {
                let token = self.find_token_at_position(tokens, position)?;

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
            })
            .flatten();

        Ok(result)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        let symbols = self.with_parsed_document(&uri, |_, ast| {
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

        let result = self.with_parsed_document(&uri, |tokens, ast| {
            let token = self.find_token_at_position(tokens, position)?;

            if token.0 != TokenVariant::Identifier {
                return None;
            }

            // Only allow renaming user-defined declarations, not built-in types/annotations
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

        let result = self.with_parsed_document(&uri, |tokens, ast| {
            let token = self.find_token_at_position(tokens, position)?;

            if token.0 != TokenVariant::Identifier {
                return None;
            }

            let old_name = token.1.v;

            // Only allow renaming user-defined declarations, not built-in types/annotations
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
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        plugins: load_plugins(),
        document_map: Mutex::new(HashMap::new()),
        module_registry: new_registry(),
        declaration_cache: Mutex::new(HashMap::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
