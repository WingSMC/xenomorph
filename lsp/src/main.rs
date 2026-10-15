use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::{
    config::WorkspaceConfigWatcher,
    formatter::{format_xenomorph, format_xenomorph_with_syntax},
    lexer::{Token, TokenVariant},
    module::{
        types::{DeclarationInfo, ModuleDiagnostic},
        XenoRegistry,
    },
    parser::Declaration,
    semantic::{
        collect_source_type_occurrences, collect_static_type_references, collect_type_references,
        OwnedAnnotation, OwnedConstraint, OwnedSemanticType, OwnedTrait, OwnedTraitKind, OwnedType,
        TypeHierarchy,
    },
    TokenData,
};

mod completions;
mod hover;
mod semantic_tokens;

use completions::{
    create_completion_item, create_owned_annotation_completion_item,
    create_owned_type_completion_item,
};

struct HoverTarget {
    name: String,
    type_arguments: Vec<String>,
    range: Range,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceTokenSpan {
    line: u32,
    scalar_column: u32,
    utf16_length: u32,
}

impl SourceTokenSpan {
    fn from_token(token: &TokenData, source: &str) -> Self {
        Self {
            line: token.line(source),
            scalar_column: token.column(source),
            utf16_length: token.text(source).encode_utf16().count() as u32,
        }
    }

    fn to_editor_range(self, source: &str) -> Range {
        let start = source_position_to_editor(source, self.line, self.scalar_column);
        Range {
            start,
            end: Position::new(start.line, start.character + self.utf16_length),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GenericParameterTarget {
    name: String,
    declaration: SourceTokenSpan,
    occurrences: Vec<SourceTokenSpan>,
}

fn source_position_to_editor(source: &str, line: u32, scalar_column: u32) -> Position {
    let character = source
        .split('\n')
        .nth(line as usize)
        .map(|source_line| {
            source_line
                .chars()
                .take(scalar_column as usize)
                .map(char::len_utf16)
                .sum::<usize>() as u32
        })
        .unwrap_or(scalar_column);
    Position::new(line, character)
}

fn token_to_editor_range(source: &str, token: &TokenData) -> Range {
    let start = source_position_to_editor(source, token.line(source), token.column(source));
    let mut segments = token.text(source).split('\n');
    let first = segments.next().unwrap_or_default();
    let remaining: Vec<_> = segments.collect();
    let end = if let Some(last) = remaining.last() {
        Position::new(
            token.line(source) + remaining.len() as u32,
            last.strip_suffix('\r')
                .unwrap_or(last)
                .encode_utf16()
                .count() as u32,
        )
    } else {
        Position::new(
            start.line,
            start.character + first.encode_utf16().count() as u32,
        )
    };
    Range { start, end }
}

#[inline]
fn same_token(left: &TokenData, right: &TokenData) -> bool {
    left == right
}

fn generic_parameter_target_at(
    ast: &[Declaration],
    selected: &TokenData,
    source: &str,
) -> Option<GenericParameterTarget> {
    for declaration in ast {
        let Declaration::Type { generics, ty, .. } = declaration else {
            continue;
        };
        for (parameter, _) in generics.as_deref().unwrap_or_default() {
            let mut references = Vec::new();
            collect_type_references(ty, parameter.text(source), &mut references, source);
            if !same_token(parameter, selected)
                && !references
                    .iter()
                    .any(|reference| same_token(reference, selected))
            {
                continue;
            }

            let declaration = SourceTokenSpan::from_token(parameter, source);
            let mut occurrences = Vec::with_capacity(references.len() + 1);
            occurrences.push(declaration);
            occurrences.extend(
                references
                    .into_iter()
                    .map(|token| SourceTokenSpan::from_token(&token, source)),
            );
            return Some(GenericParameterTarget {
                name: parameter.text(source).to_string(),
                declaration,
                occurrences,
            });
        }
    }
    None
}

#[derive(Clone, Copy)]
enum CompletionFrame<'src> {
    Annotation {
        name: &'src str,
        parameter_index: usize,
    },
    Parenthesis,
    Bracket,
    Curly,
    Angle,
}

fn token_starts_before(source: &str, token: &Token, position: Position) -> bool {
    token_to_editor_range(source, &token.1).start < position
}

fn annotation_argument_context<'src>(
    tokens: &'src [Token],
    source: &'src str,
    position: Position,
) -> Option<(&'src str, usize)> {
    let mut frames = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !token_starts_before(source, token, position) {
            break;
        }

        match token.0 {
            TokenVariant::LParen => {
                let annotation_name = index.checked_sub(1).and_then(|local_index| {
                    let local = tokens.get(local_index)?;
                    if local.0 != TokenVariant::Identifier {
                        return None;
                    }
                    let (name_index, start) = if local_index >= 2
                        && tokens[local_index - 1].0 == TokenVariant::PathSeparator
                        && tokens[local_index - 2].0 == TokenVariant::Identifier
                        && tokens[local_index - 2].1.end + 2 == local.1.start
                    {
                        (local_index - 2, tokens[local_index - 2].1.start)
                    } else {
                        (local_index, local.1.start)
                    };
                    let at = tokens.get(name_index.checked_sub(1)?)?;
                    (at.0 == TokenVariant::At)
                        .then(|| &source[start as usize..local.1.end as usize])
                });
                frames.push(match annotation_name {
                    Some(name) => CompletionFrame::Annotation {
                        name,
                        parameter_index: 0,
                    },
                    None => CompletionFrame::Parenthesis,
                });
            }
            TokenVariant::LBracket => frames.push(CompletionFrame::Bracket),
            TokenVariant::LCurly => frames.push(CompletionFrame::Curly),
            TokenVariant::Lt => frames.push(CompletionFrame::Angle),
            TokenVariant::RParen => pop_frame(&mut frames, |frame| {
                matches!(
                    frame,
                    CompletionFrame::Annotation { .. } | CompletionFrame::Parenthesis
                )
            }),
            TokenVariant::RBracket => pop_frame(&mut frames, |frame| {
                matches!(frame, CompletionFrame::Bracket)
            }),
            TokenVariant::RCurly => {
                pop_frame(&mut frames, |frame| matches!(frame, CompletionFrame::Curly))
            }
            TokenVariant::Gt => {
                pop_frame(&mut frames, |frame| matches!(frame, CompletionFrame::Angle))
            }
            TokenVariant::Comma => {
                if let Some(CompletionFrame::Annotation {
                    parameter_index, ..
                }) = frames.last_mut()
                {
                    *parameter_index += 1;
                }
            }
            _ => {}
        }
    }

    frames.iter().rev().find_map(|frame| match frame {
        CompletionFrame::Annotation {
            name,
            parameter_index,
        } => Some((*name, *parameter_index)),
        _ => None,
    })
}

fn generic_constraint_context(tokens: &[Token], source: &str, position: Position) -> bool {
    let mut angle_depth = 0usize;
    let mut constraint_at_depth = None;
    for token in tokens {
        if !token_starts_before(source, token, position) {
            break;
        }
        match token.0 {
            TokenVariant::Lt => angle_depth += 1,
            TokenVariant::Gt => {
                if constraint_at_depth == Some(angle_depth) {
                    constraint_at_depth = None;
                }
                angle_depth = angle_depth.saturating_sub(1);
            }
            TokenVariant::Colon if angle_depth > 0 => constraint_at_depth = Some(angle_depth),
            TokenVariant::Comma if constraint_at_depth == Some(angle_depth) => {
                constraint_at_depth = None
            }
            _ => {}
        }
    }
    constraint_at_depth == Some(angle_depth) && angle_depth > 0
}

fn pop_frame(frames: &mut Vec<CompletionFrame<'_>>, matches: impl Fn(CompletionFrame<'_>) -> bool) {
    if let Some(index) = frames.iter().rposition(|frame| matches(*frame)) {
        frames.truncate(index);
    }
}

fn deduplicate_completions(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.label.clone()));
    items
}

struct Backend {
    client: Client,
    registry: Arc<XenoRegistry>,
    runtime: Arc<RuntimeState>,
}

struct RuntimeState {
    registry: RwLock<Arc<XenoRegistry>>,
    documents: Mutex<HashMap<Url, OpenDocument>>,
    updates: tokio::sync::Mutex<()>,
    coordinator: Mutex<()>,
    config_watcher: Mutex<Option<WorkspaceConfigWatcher>>,
    stopped: std::sync::atomic::AtomicBool,
}

struct OpenDocument {
    text: String,
    version: i32,
}

impl RuntimeState {
    fn snapshot(&self) -> Arc<XenoRegistry> {
        Arc::clone(&self.registry.read().unwrap())
    }

    fn reconcile(
        self: &Arc<Self>,
        client: Client,
    ) -> std::result::Result<Arc<XenoRegistry>, String> {
        let _coordinator = self
            .coordinator
            .lock()
            .map_err(|_| "Runtime coordinator is poisoned")?;
        if self.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Language server is shutting down".into());
        }
        let active = self.snapshot();
        let config = xenomorph_common::config::Config::discover(&active.config.workdir)?;
        let candidate =
            XenoRegistry::with_config(config, false).map_err(|error| error.to_string())?;
        candidate.load_module(&[&candidate.entry], true, None);
        {
            let documents = self
                .documents
                .lock()
                .map_err(|_| "Document storage is poisoned")?;
            synchronize_documents(&candidate, &documents);
        }
        candidate.refresh_name_collision_diagnostics();
        let watcher = config_watcher_paths(
            candidate.config.workspace_config_paths(),
            Arc::clone(self),
            client,
        )?;
        let candidate = Arc::new(candidate);
        *self
            .config_watcher
            .lock()
            .map_err(|_| "Config watcher storage is poisoned")? = Some(watcher);
        *self
            .registry
            .write()
            .map_err(|_| "Runtime storage is poisoned")? = Arc::clone(&candidate);
        Ok(candidate)
    }
}

struct LiveBackend {
    client: Client,
    runtime: Arc<RuntimeState>,
}

impl LiveBackend {
    fn backend(&self) -> Backend {
        Backend {
            client: self.client.clone(),
            registry: self.runtime.snapshot(),
            runtime: Arc::clone(&self.runtime),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LiveBackend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.backend().initialize(params).await
    }
    async fn initialized(&self, params: InitializedParams) {
        self.backend().initialized(params).await;
    }
    async fn shutdown(&self) -> Result<()> {
        self.backend().shutdown().await
    }
    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        self.backend().execute_command(params).await
    }
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.backend().did_open(params).await;
    }
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.backend().did_change(params).await;
    }
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.backend().did_close(params).await;
    }
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.backend().completion(params).await
    }
    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        self.backend().completion_resolve(item).await
    }
    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.backend().hover(params).await
    }
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        self.backend().semantic_tokens_full(params).await
    }
    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        self.backend().formatting(params).await
    }
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.backend().goto_definition(params).await
    }
    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.backend().references(params).await
    }
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        self.backend().document_symbol(params).await
    }
    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        self.backend().prepare_rename(params).await
    }
    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.backend().rename(params).await
    }
}

fn config_watcher(
    runtime: Arc<RuntimeState>,
    client: Client,
) -> std::result::Result<WorkspaceConfigWatcher, String> {
    let paths = runtime.snapshot().config.workspace_config_paths();
    config_watcher_paths(paths, runtime, client)
}

fn config_watcher_paths(
    paths: Vec<std::path::PathBuf>,
    runtime: Arc<RuntimeState>,
    client: Client,
) -> std::result::Result<WorkspaceConfigWatcher, String> {
    let handle = tokio::runtime::Handle::current();
    WorkspaceConfigWatcher::watch_paths(paths, move |event| {
        let runtime = Arc::clone(&runtime);
        let client = client.clone();
        handle.spawn(async move {
            match event {
                Ok(()) => {
                    if let Err(error) = reload_runtime(runtime, client.clone()).await {
                        client.log_message(MessageType::ERROR, format!("Workspace reload failed; keeping the last valid runtime: {error}")).await;
                    }
                }
                Err(error) => client.log_message(MessageType::ERROR, format!("Workspace config watcher error: {error}")).await,
            }
        });
    }).map_err(|error| error.to_string())
}

async fn reload_runtime(
    runtime: Arc<RuntimeState>,
    client: Client,
) -> std::result::Result<(), String> {
    let _update = runtime.updates.lock().await;
    let previous = runtime.snapshot();
    let state = Arc::clone(&runtime);
    let reload_client = client.clone();
    let candidate = tokio::task::spawn_blocking(move || state.reconcile(reload_client))
        .await
        .map_err(|error| format!("Runtime reload task failed: {error}"))??;
    let publications = {
        let documents = runtime.documents.lock().unwrap();
        let graph = candidate.module_graph();
        let mut uris: HashSet<Url> = documents.keys().cloned().collect();
        for registry in [&previous, &candidate] {
            for module in registry.module_cache.read().unwrap().values() {
                let module = module.read().unwrap();
                if let Ok(uri) = Url::from_file_path(module.borrow_abs_path()) {
                    uris.insert(uri);
                }
            }
        }
        uris.into_iter()
            .map(|uri| {
                let document = documents.get(&uri);
                let diagnostics = diagnostics_for_document(
                    &candidate,
                    &graph,
                    &uri,
                    document.map(|document| document.text.as_str()),
                );
                (uri, diagnostics, document.map(|document| document.version))
            })
            .collect::<Vec<_>>()
    };
    for (uri, diagnostics, version) in publications {
        client.publish_diagnostics(uri, diagnostics, version).await;
    }
    client
        .log_message(
            MessageType::INFO,
            "Workspace configuration and plugins reloaded without restarting the LSP.",
        )
        .await;
    Ok(())
}

/// Apply editor buffers in import order so newly imported open documents use
/// their unsaved text, and unrelated documents do not become graph members.
fn synchronize_documents(registry: &XenoRegistry, documents: &HashMap<Url, OpenDocument>) {
    let sources: HashMap<_, _> = documents
        .iter()
        .filter_map(|(uri, document)| {
            let path = uri.to_file_path().ok()?;
            let module = registry.abs_path_to_module_path(&path)?;
            Some((module, (path, &document.text)))
        })
        .collect();
    let mut visited = HashSet::new();
    let mut pending = VecDeque::from([registry.entry.clone()]);
    while let Some(module) = pending.pop_front() {
        if !visited.insert(module.clone()) {
            continue;
        }
        if let Some((path, source)) = sources.get(&module) {
            let unchanged = registry
                .with_module(&module, |_, _, data| {
                    data.borrow_source() == source.as_str()
                })
                .unwrap_or(false);
            if !unchanged {
                registry.load_module_from_source(path, (*source).clone());
            }
        }
        if let Some(imports) = registry.with_module(&module, |_, _, data| data.imports.clone()) {
            pending.extend(imports);
        }
    }
    registry.retain_modules(&registry.module_graph());
}

fn outside_module_graph_diagnostic(source: &str) -> Diagnostic {
    Diagnostic {
        range: document_range(source),
        severity: Some(DiagnosticSeverity::INFORMATION),
        message: "Not part of the current module graph.".to_string(),
        source: Some("xenomorph".to_string()),
        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
        ..Default::default()
    }
}

fn document_range(source: &str) -> Range {
    let source = xenomorph_common::SourceDocument::new(source.to_string());
    let (line, character) = source.position(source.text().len() as u32, true);
    Range::new(Position::new(0, 0), Position::new(line, character))
}

fn apply_document_changes(
    source: &str,
    changes: Vec<TextDocumentContentChangeEvent>,
) -> Option<String> {
    let mut text = source.to_string();
    for change in changes {
        if let Some(range) = change.range {
            let document = xenomorph_common::SourceDocument::new(text.clone());
            let start = document.offset(range.start.line, range.start.character, true)? as usize;
            let end = document.offset(range.end.line, range.end.character, true)? as usize;
            if start > end {
                return None;
            }
            text.replace_range(start..end, &change.text);
        } else {
            text = change.text;
        }
    }
    Some(text)
}

fn diagnostics_for_document(
    registry: &XenoRegistry,
    graph: &HashSet<String>,
    uri: &Url,
    open_source: Option<&str>,
) -> Vec<Diagnostic> {
    let module = uri
        .to_file_path()
        .ok()
        .and_then(|path| registry.abs_path_to_module_path(&path))
        .filter(|path| graph.contains(path));
    if let Some(module) = module {
        let errors = registry.get_all_errors_for(&module);
        registry
            .with_module(&module, |_, _, data| {
                diagnostics_for_source(&errors, &module, &data.source)
            })
            .unwrap_or_default()
    } else {
        open_source
            .map(|source| vec![outside_module_graph_diagnostic(source)])
            .unwrap_or_default()
    }
}

fn diagnostics_for_source(
    errors: &[ModuleDiagnostic],
    path: &str,
    source: &xenomorph_common::SourceDocument,
) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics_for_module(errors, path);
    for (diagnostic, error) in diagnostics.iter_mut().zip(
        errors
            .iter()
            .filter(|e| e.module_path == path && e.location.is_some()),
    ) {
        let (line, column, len) = error.location.unwrap();
        if let Some(start) = source.offset(line, column, false) {
            let end = start.saturating_add(len);
            if source.text().is_char_boundary(end as usize) && end as usize <= source.text().len() {
                let (a, b) = source.position(start, true);
                let (c, d) = source.position(end, true);
                diagnostic.range = Range::new(Position::new(a, b), Position::new(c, d));
            }
        }
    }
    diagnostics
}

fn diagnostics_for_module(errors: &[ModuleDiagnostic], module_path: &str) -> Vec<Diagnostic> {
    errors
        .iter()
        .filter(|error| error.module_path == module_path)
        .filter_map(|error| {
            let (line, col, len) = error.location?;
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
                severity: Some(match error.severity {
                    xenomorph_common::XenoDiagSeverity::Err => DiagnosticSeverity::ERROR,
                    xenomorph_common::XenoDiagSeverity::Warn => DiagnosticSeverity::WARNING,
                    xenomorph_common::XenoDiagSeverity::Info => DiagnosticSeverity::INFORMATION,
                }),
                message: error.message.clone(),
                source: Some("xenomorph".to_string()),
                tags: (!error.tags.is_empty()).then(|| {
                    error
                        .tags
                        .iter()
                        .map(|tag| match tag {
                            xenomorph_common::XenoDiagnosticTag::Unnecessary => {
                                DiagnosticTag::UNNECESSARY
                            }
                        })
                        .collect()
                }),
                ..Default::default()
            })
        })
        .collect()
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
            .metadata
            .types
            .values()
            .map(create_owned_type_completion_item)
    }

    fn get_builtin_annotations(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.registry
            .metadata
            .annotations
            .values()
            .map(create_owned_annotation_completion_item)
    }

    fn find_annotation(&self, name: &str) -> Option<&OwnedAnnotation> {
        self.registry.metadata.annotations.get(name)
    }

    fn semantic_types(&self) -> impl Iterator<Item = &OwnedSemanticType> + '_ {
        self.registry.metadata.types.values()
    }

    fn semantic_traits(&self) -> Vec<&OwnedTrait> {
        self.registry.metadata.traits.values().collect()
    }

    fn literal_completions(&self, required: &OwnedTrait) -> Vec<CompletionItem> {
        let mut hierarchy = TypeHierarchy::default();
        hierarchy
            .insert_metadata(&self.registry.metadata)
            .expect("validated registry metadata");
        let accepts = |name: &str| {
            matches!(required.kind, OwnedTraitKind::Literal)
                || self
                    .semantic_types()
                    .find(|candidate| candidate.name == name)
                    .is_some_and(|candidate| {
                        hierarchy
                            .implements_trait(&OwnedType::named(&candidate.name), &required.name)
                    })
        };
        let mut items = Vec::new();
        if accepts("integer") || accepts("number") {
            items.push(create_completion_item(
                "0",
                Some("Integer literal"),
                CompletionItemKind::VALUE,
            ));
        }
        if accepts("number") {
            items.push(create_completion_item(
                "0.0",
                Some("Number literal"),
                CompletionItemKind::VALUE,
            ));
        }
        if accepts("string") {
            let mut item = create_completion_item(
                "string literal",
                Some("String literal"),
                CompletionItemKind::VALUE,
            );
            item.insert_text = Some("\"${1:value}\"".to_string());
            item.insert_text_format = Some(InsertTextFormat::SNIPPET);
            items.push(item);
        }
        if accepts("bool") {
            items.push(create_completion_item(
                "true",
                Some("Boolean literal"),
                CompletionItemKind::VALUE,
            ));
            items.push(create_completion_item(
                "false",
                Some("Boolean literal"),
                CompletionItemKind::VALUE,
            ));
        }
        items
    }

    fn regex_literal_completion() -> CompletionItem {
        let mut item = create_completion_item(
            "regex literal",
            Some("Regular-expression literal"),
            CompletionItemKind::VALUE,
        );
        item.insert_text = Some("/${1:pattern}/".to_string());
        item.insert_text_format = Some(InsertTextFormat::SNIPPET);
        item
    }

    fn completions_for_constraint(
        &self,
        required: &OwnedConstraint,
        module_path: Option<&str>,
    ) -> Vec<CompletionItem> {
        let all_types = || {
            let mut types: Vec<_> = self.get_builtin_types().collect();
            if let Some(module_path) = module_path {
                types.extend(self.get_module_completions(module_path));
            }
            deduplicate_completions(types)
        };

        let OwnedConstraint::Trait(required) = required else {
            return all_types();
        };
        let Some(required) = self.registry.metadata.traits.get(required) else {
            return Vec::new();
        };
        let mut hierarchy = TypeHierarchy::default();
        hierarchy
            .insert_metadata(&self.registry.metadata)
            .expect("validated registry metadata");

        match &required.kind {
            OwnedTraitKind::Semantic => {
                let allowed: std::collections::HashSet<_> = self
                    .semantic_types()
                    .filter(|semantic_type| {
                        hierarchy.implements_trait(
                            &OwnedType::named(&semantic_type.name),
                            &required.name,
                        )
                    })
                    .map(|semantic_type| semantic_type.name.as_str())
                    .collect();
                all_types()
                    .into_iter()
                    .filter(|item| allowed.contains(item.label.as_str()))
                    .collect()
            }
            OwnedTraitKind::Literal | OwnedTraitKind::LiteralType => {
                self.literal_completions(required)
            }
            OwnedTraitKind::RegexLiteral => vec![Self::regex_literal_completion()],
            OwnedTraitKind::Identifier | OwnedTraitKind::Type => all_types(),
            OwnedTraitKind::Annotation => self.get_builtin_annotations().collect(),
            OwnedTraitKind::Expression => {
                let mut items = all_types();
                items.extend(self.get_builtin_annotations());
                if let Some(literal) = self.registry.metadata.traits.get("Literal") {
                    items.extend(self.literal_completions(literal));
                }
                deduplicate_completions(items)
            }
            OwnedTraitKind::Struct => all_types(),
            OwnedTraitKind::Sum(member) => self
                .registry
                .metadata
                .traits
                .get(member)
                .map(|member| self.literal_completions(member))
                .unwrap_or_default(),
        }
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
                        Some(format!("{}::", name))
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

    // ── Token helpers ───────────────────────────────────────────────

    fn find_token_at_position<'a>(
        source: &str,
        tokens: &'a [Token],
        position: Position,
    ) -> Option<&'a Token> {
        tokens.iter().find(|(_, data)| {
            let token_range = token_to_editor_range(source, data);
            token_range.start <= position && position < token_range.end
        })
    }

    fn find_token_before_or_at_position<'a>(
        source: &str,
        tokens: &'a [Token],
        position: Position,
    ) -> Option<&'a Token> {
        tokens
            .iter()
            .find(|(_, data)| {
                let range = token_to_editor_range(source, data);
                range.start < position && position < range.end
            })
            .or_else(|| {
                tokens.iter().rev().find(|(_, data)| {
                    let end = token_to_editor_range(source, data).end;
                    end.line < position.line
                        || (end.line == position.line && end.character <= position.character)
                })
            })
    }

    // ── Document validation ─────────────────────────────────────────

    /// Synchronizes graph members and updates diagnostics for cached and open files.
    async fn validate_document(&self, uri: &Url, source: String, version: i32) {
        // Keep mutation and publication in one ordered operation. LSP handlers
        // run concurrently, so the version check also rejects late older edits.
        let _update = self.runtime.updates.lock().await;
        self.validate_document_locked(uri, source, version).await;
    }

    async fn validate_document_locked(&self, uri: &Url, source: String, version: i32) {
        let publications = {
            let _coordinator = self.runtime.coordinator.lock().unwrap();
            let mut documents = self.runtime.documents.lock().unwrap();
            if documents
                .get(uri)
                .is_some_and(|document| document.version >= version)
            {
                return;
            }
            documents.insert(
                uri.clone(),
                OpenDocument {
                    text: source,
                    version,
                },
            );
            let registry = self.runtime.snapshot();
            let mut affected_paths: Vec<_> = registry
                .module_cache
                .read()
                .unwrap()
                .values()
                .map(|module| module.read().unwrap().borrow_abs_path().to_path_buf())
                .collect();
            synchronize_documents(&registry, &documents);
            let graph = registry.module_graph();
            affected_paths.extend(
                registry
                    .module_cache
                    .read()
                    .unwrap()
                    .values()
                    .map(|module| module.read().unwrap().borrow_abs_path().to_path_buf()),
            );
            let mut uris: HashSet<_> = documents.keys().cloned().collect();
            uris.extend(
                affected_paths
                    .into_iter()
                    .filter_map(|path| Url::from_file_path(path).ok()),
            );
            uris.into_iter()
                .map(|uri| {
                    let document = documents.get(&uri);
                    let diagnostics = diagnostics_for_document(
                        &registry,
                        &graph,
                        &uri,
                        document.map(|document| document.text.as_str()),
                    );
                    (uri, diagnostics, document.map(|document| document.version))
                })
                .collect::<Vec<_>>()
        };
        for (uri, diagnostics, version) in publications {
            self.client
                .publish_diagnostics(uri, diagnostics, version)
                .await;
        }
    }

    // ── Completions ─────────────────────────────────────────────────

    fn get_context_completions(
        &self,
        tokens: &[Token],
        ast: &[Declaration],
        source: &str,
        position: Position,
        module_path: Option<&str>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        let document = xenomorph_common::SourceDocument::new(source.to_string());
        if let Some(offset) = document.offset(position.line, position.character, true) {
            let visible: Vec<_> = tokens
                .iter()
                .filter(|(_, token)| token.start < offset)
                .collect();
            let start = visible
                .iter()
                .rposition(|(kind, _)| *kind == TokenVariant::Semicolon)
                .map_or(0, |i| i + 1);
            if let Some((_, import)) = visible[start..]
                .iter()
                .find(|(kind, _)| *kind == TokenVariant::Import)
            {
                let tail = &source[import.end as usize..offset as usize];
                let tail = tail.trim();
                let (module, partial, in_group) =
                    if let Some((module, names)) = tail.split_once('{') {
                        (
                            module.trim().trim_end_matches("::").trim().to_string(),
                            names.rsplit(',').next().unwrap_or("").trim().to_string(),
                            true,
                        )
                    } else if let Some((module, partial)) = tail.rsplit_once("::") {
                        (module.to_string(), partial.to_string(), false)
                    } else {
                        (String::new(), tail.to_string(), false)
                    };
                if !in_group {
                    items.extend(self.get_import_completions(tail));
                }
                if !module.is_empty() {
                    let is_selected =
                        |token: &TokenData| !(token.start <= offset && offset <= token.end);
                    let mut imported: HashSet<_> = ast
                        .iter()
                        .filter_map(|declaration| match declaration {
                            Declaration::Import { names, .. } => Some(names),
                            _ => None,
                        })
                        .flatten()
                        .filter(|token| is_selected(token))
                        .map(|token| token.text(source))
                        .collect();
                    // An unfinished import is absent from the AST. Retain its
                    // completed selections on either side of the cursor.
                    let active: Vec<_> = tokens
                        .iter()
                        .skip_while(|(_, token)| token.start < import.start)
                        .take_while(|(kind, _)| *kind != TokenVariant::Semicolon)
                        .collect();
                    if let Some(open) = active
                        .iter()
                        .position(|(kind, _)| *kind == TokenVariant::LCurly)
                    {
                        for pair in active[open + 1..].windows(2) {
                            if pair[0].0 == TokenVariant::Identifier
                                && matches!(pair[1].0, TokenVariant::Comma | TokenVariant::RCurly)
                                && is_selected(&pair[0].1)
                            {
                                imported.insert(pair[0].1.text(source));
                            }
                        }
                    }
                    items.extend(
                        self.registry
                            .get_exported_declarations(&module.replace("::", "/"))
                            .into_iter()
                            .filter(|d| {
                                d.name.starts_with(&partial) && !imported.contains(d.name.as_str())
                            })
                            .map(|d| {
                                create_completion_item(
                                    &d.name,
                                    d.docs.as_deref(),
                                    CompletionItemKind::CLASS,
                                )
                            }),
                    );
                }
                return deduplicate_completions(items);
            }
        }

        if let Some((annotation_name, parameter_index)) =
            annotation_argument_context(tokens, source, position)
        {
            if let Some(parameter) = self
                .find_annotation(annotation_name)
                .and_then(|annotation| annotation.parameter_at(parameter_index))
            {
                return self.completions_for_constraint(&parameter.constraint, module_path);
            }
            return items;
        }

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
                insert_text: Some("import ${1:module}::{${2:Declaration}};".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
            items.push(CompletionItem {
                label: "pub".into(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Export a declaration or import".into()),
                ..Default::default()
            });
        };

        let all_types = || -> Vec<CompletionItem> {
            let mut types: Vec<CompletionItem> = self.get_builtin_types().collect();
            if let Some(mp) = module_path {
                types.extend(self.get_module_completions(mp));
            }
            deduplicate_completions(types)
        };

        if generic_constraint_context(tokens, source, position) {
            items.extend(all_types());
            items.extend(self.semantic_traits().into_iter().map(|xeno_trait| {
                create_completion_item(
                    &xeno_trait.name,
                    xeno_trait.documentation.as_deref(),
                    CompletionItemKind::INTERFACE,
                )
            }));
            return deduplicate_completions(items);
        }

        if let Some(current_token) =
            Self::find_token_before_or_at_position(source, tokens, position)
        {
            let client = self.client.clone();
            let msg = format!(
                "Current token: {:?} at line {}, col {}, value '{}'",
                current_token.0,
                current_token.1.line(source),
                current_token.1.column(source),
                current_token.1.text(source)
            );
            tokio::spawn(async move {
                let _ = client.log_message(MessageType::INFO, msg).await;
            });

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
                TokenVariant::Path => {
                    items.extend(self.get_import_completions(current_token.1.text(source)));
                }
                TokenVariant::Identifier => {
                    let token_idx = tokens.iter().position(|t| {
                        t.1.line(source) == current_token.1.line(source)
                            && t.1.column(source) == current_token.1.column(source)
                            && t.1.text(source) == current_token.1.text(source)
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

    fn hover_target_at_location(
        &self,
        tokens: &[Token],
        ast: &[Declaration],
        source: &str,
        position: Position,
    ) -> Option<HoverTarget> {
        let token = Self::find_token_at_position(source, tokens, position)?;

        if !matches!(
            token.0,
            TokenVariant::Identifier | TokenVariant::PathSeparator
        ) {
            return None;
        }

        let range = xenomorph_common::plugins::syntax::source_tokens(ast)
            .into_iter()
            .filter(|range| range.start <= token.1.start && range.end >= token.1.end)
            .min_by_key(|range| range.end - range.start)
            .unwrap_or(token.1);

        Some(HoverTarget {
            name: range.text(source).to_string(),
            type_arguments: hover::type_arguments_at_token(ast, &range, source).unwrap_or_default(),
            range: token_to_editor_range(source, &range),
        })
    }

    fn type_declaration_preview(
        &self,
        info: &DeclarationInfo,
        type_arguments: &[String],
    ) -> Option<String> {
        self.registry
            .with_module(&info.module_path, |_, ast, module| {
                let source = module.borrow_source();
                ast.iter().find_map(|declaration| match declaration {
                    Declaration::Type { name, .. } if name.text(source) == info.name => {
                        hover::format_type_declaration(declaration, type_arguments, source)
                    }
                    _ => None,
                })
            })
            .flatten()
    }

    fn user_type_hover(&self, target: &HoverTarget, current_module: &str) -> Option<Hover> {
        let info = self
            .registry
            .find_declaration(current_module, &target.name)?;
        let preview = self.type_declaration_preview(&info, &target.type_arguments)?;
        let mut contents = format!("**{}**", target.name);
        if info.module_path != current_module {
            contents.push_str(&format!(" *(from {})*", info.module_path));
        }
        if let Some(docs) = info.docs.as_deref().filter(|docs| !docs.is_empty()) {
            contents.push_str("\n\n");
            contents.push_str(docs);
        }
        contents.push_str("\n\n```xenomorph\n");
        contents.push_str(&preview);
        contents.push_str("\n```");

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: contents,
            }),
            range: Some(target.range),
        })
    }

    fn builtin_hover(&self, target: &HoverTarget) -> Option<Hover> {
        let builtin_info = self
            .get_builtin_types()
            .find(|item| item.label == target.name)
            .or_else(|| {
                self.get_builtin_annotations()
                    .find(|item| item.label == target.name)
            })?;

        let value = match builtin_info.documentation {
            Some(Documentation::MarkupContent(content)) => content.value,
            Some(Documentation::String(value)) => value,
            None => format!(
                "**{}**\n\n{}",
                builtin_info.label,
                builtin_info.detail.unwrap_or_default()
            ),
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(target.range),
        })
    }

    // ── Goto Definition helpers ─────────────────────────────────────

    fn declaration_info_to_location(&self, info: &DeclarationInfo) -> Option<Location> {
        let target_uri = Url::from_file_path(&info.abs_path).ok()?;
        let range = self
            .registry
            .with_module(&info.module_path, |_, ast, module| {
                let source = module.borrow_source();
                ast.iter().find_map(|declaration| match declaration {
                    Declaration::Type { name, .. }
                        if name.line(source) == info.line
                            && name.column(source) == info.column
                            && name.text(source) == info.name =>
                    {
                        Some(token_to_editor_range(module.borrow_source(), name))
                    }
                    _ => None,
                })
            })
            .flatten()?;
        Some(Location {
            uri: target_uri,
            range,
        })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["xenomorph.reloadPlugins".to_string()],
                    work_done_progress_options: Default::default(),
                }),
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
                        "(".to_string(),
                        ",".to_string(),
                        "{".to_string(),
                        "/".to_string(),
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
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: Default::default(),
                            legend: semantic_tokens::legend(),
                            range: None,
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
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

        match config_watcher(Arc::clone(&self.runtime), self.client.clone()) {
            Ok(watcher) => {
                let config_paths = watcher
                    .config_paths()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                *self.runtime.config_watcher.lock().unwrap() = Some(watcher);
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Watching workspace config(s): {config_paths}"),
                    )
                    .await;
            }
            Err(error) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Unable to watch workspace config: {error}"),
                    )
                    .await;
            }
        }

        self.client
            .log_message(MessageType::INFO, "Xenomorph Language Server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        {
            let _coordinator = self.runtime.coordinator.lock().unwrap();
            self.runtime
                .stopped
                .store(true, std::sync::atomic::Ordering::Release);
            let empty = XenoRegistry {
                module_cache: RwLock::new(HashMap::new()),
                index: RwLock::new(HashMap::new()),
                hierarchy: RwLock::new(TypeHierarchy::default()),
                coordinator: Mutex::new(()),
                root: self.registry.root.clone(),
                entry: self.registry.entry.clone(),
                config: Arc::clone(&self.registry.config),
                metadata: self.registry.metadata.clone(),
                plugins: Vec::new(),
                analyzer: xenomorph_common::semantic::Analyzer::new(false, &[]).unwrap(),
            };
            *self.runtime.registry.write().unwrap() = Arc::new(empty);
        }
        self.runtime.config_watcher.lock().unwrap().take();
        Ok(())
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        if params.command != "xenomorph.reloadPlugins" {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Unknown Xenomorph command",
            ));
        }
        reload_runtime(Arc::clone(&self.runtime), self.client.clone())
            .await
            .map_err(|error| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("Plugin reload failed: {error}"))
            })?;
        Ok(Some(serde_json::json!({ "reloaded": true })))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.validate_document(&uri, text, params.text_document.version)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let _update = self.runtime.updates.lock().await;
        let uri = params.text_document.uri;
        let source = {
            let documents = self.runtime.documents.lock().unwrap();
            let Some(document) = documents.get(&uri) else {
                return;
            };
            if document.version >= params.text_document.version {
                return;
            }
            apply_document_changes(&document.text, params.content_changes)
        };
        if let Some(source) = source {
            self.validate_document_locked(&uri, source, params.text_document.version)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let _update = self.runtime.updates.lock().await;
        self.runtime
            .documents
            .lock()
            .unwrap()
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);

        let completions = self.registry.with_module(
            module_path.as_deref().unwrap_or(""),
            |tokens, ast, module| {
                self.get_context_completions(
                    tokens,
                    ast,
                    module.borrow_source(),
                    position,
                    module_path.as_deref(),
                )
            },
        );

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

        let current_module = module_path.as_deref().unwrap_or("");
        let target = self
            .registry
            .with_module(current_module, |tokens, ast, module| {
                self.hover_target_at_location(tokens, ast, module.borrow_source(), position)
            })
            .flatten();
        let Some(target) = target else {
            return Ok(None);
        };

        Ok(self
            .user_type_hover(&target, current_module)
            .or_else(|| self.builtin_hover(&target)))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let module_path = self.uri_to_module_path(&params.text_document.uri);
        let tokens = self.registry.with_module(
            module_path.as_deref().unwrap_or(""),
            |tokens, ast, module| SemanticTokens {
                result_id: None,
                data: semantic_tokens::encode(module.borrow_source(), tokens, ast),
            },
        );

        Ok(tokens.map(SemanticTokensResult::Tokens))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let _update = self.runtime.updates.lock().await;
        let registry = self.runtime.snapshot();
        let uri = params.text_document.uri;
        let documents = self.runtime.documents.lock().unwrap();
        let Some(document) = documents.get(&uri) else {
            return Ok(None);
        };
        let source = document.text.as_str();
        let module_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| registry.abs_path_to_module_path(&path));

        let formatted = registry
            .with_module(
                module_path.as_deref().unwrap_or(""),
                |tokens, ast, module| {
                    (module.borrow_source() == source).then(|| {
                        format_xenomorph_with_syntax(
                            source,
                            tokens,
                            ast,
                            &registry.config.formatter,
                        )
                    })
                },
            )
            .flatten()
            .unwrap_or_else(|| format_xenomorph(source, &registry.config.formatter));

        Ok(Some(vec![TextEdit {
            range: document_range(source),
            new_text: formatted,
        }]))
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
        let local_result = self.registry.with_module(mp, |tokens, ast, module| {
            let source = module.borrow_source();
            let token = Self::find_token_at_position(source, tokens, position)?;

            // If cursor is on an import line, navigate to the imported file
            if token.0 == TokenVariant::Identifier {
                for decl in ast.iter() {
                    if let Declaration::Import {
                        path, path_tokens, ..
                    } = decl
                    {
                        if path_tokens.contains(&token.1) {
                            let segments = path.to_vec();
                            if let Ok((_, abs_path)) = self.registry.resolve_import(
                                &segments.iter().map(String::as_str).collect::<Vec<_>>(),
                                None,
                            ) {
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
                if let Some(generic) = generic_parameter_target_at(ast, &token.1, source) {
                    return Some(GotoDefinitionResponse::Scalar(Location {
                        uri: uri.clone(),
                        range: generic.declaration.to_editor_range(source),
                    }));
                }
                for decl in ast {
                    if let Declaration::Type { name, .. } = decl {
                        if name.text(source) == token.1.text(source) {
                            return Some(GotoDefinitionResponse::Scalar(Location {
                                uri: uri.clone(),
                                range: token_to_editor_range(source, name),
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
        let selected = self
            .registry
            .with_module(mp, |tokens, _, module| {
                let token = Self::find_token_at_position(module.borrow_source(), tokens, position)?;
                if token.0 != TokenVariant::Identifier {
                    return None;
                }
                Some(token.1.text(module.borrow_source()).to_string())
            })
            .flatten();
        let cross_result = selected
            .and_then(|name| self.registry.find_declaration(mp, &name))
            .and_then(|info| self.declaration_info_to_location(&info))
            .map(GotoDefinitionResponse::Scalar);
        Ok(cross_result)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);
        let current_module = module_path.as_deref().unwrap_or("");
        let include_declaration = params.context.include_declaration;

        let selected = self
            .registry
            .with_module(current_module, |tokens, ast, module| {
                let source = module.borrow_source();
                let token = Self::find_token_at_position(source, tokens, position)?;
                if token.0 != TokenVariant::Identifier {
                    return None;
                }
                if generic_parameter_target_at(ast, &token.1, source).is_none()
                    && !collect_source_type_occurrences(ast, token.1.text(source), source)
                        .contains(&token.1)
                {
                    return None;
                }
                Some((
                    token.1.text(source).to_string(),
                    generic_parameter_target_at(ast, &token.1, source).map(|generic| {
                        generic
                            .occurrences
                            .into_iter()
                            .filter(|occurrence| {
                                include_declaration || *occurrence != generic.declaration
                            })
                            .map(|occurrence| Location {
                                uri: uri.clone(),
                                range: occurrence.to_editor_range(source),
                            })
                            .collect::<Vec<_>>()
                    }),
                ))
            })
            .flatten();

        let Some((searched_name, generic_locations)) = selected else {
            return Ok(None);
        };
        if let Some(locations) = generic_locations {
            return Ok(Some(locations));
        }

        let target = self
            .registry
            .find_declaration(current_module, &searched_name);
        if target.is_none() && !self.registry.metadata.types.contains_key(&searched_name) {
            return Ok(None);
        }

        let module_paths: Vec<String> = self
            .registry
            .module_cache
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect();

        let mut locations = Vec::new();
        for module_path in module_paths {
            let visible_decl = self.registry.find_declaration(&module_path, &searched_name);
            if let Some(target) = &target {
                let Some(visible_decl) = visible_decl else {
                    continue;
                };
                if visible_decl.name != target.name
                    || visible_decl.module_path != target.module_path
                {
                    continue;
                }
            } else if visible_decl.is_some() {
                continue;
            }

            if let Some(mut module_locations) = self
                .registry
                .with_module(&module_path, |_, ast, module| {
                    let uri = Url::from_file_path(module.borrow_abs_path()).ok()?;
                    let source = module.borrow_source();
                    if target.is_none() {
                        return Some(
                            collect_static_type_references(ast, &searched_name, source)
                                .iter()
                                .map(|token| Location {
                                    uri: uri.clone(),
                                    range: token_to_editor_range(source, token),
                                })
                                .collect(),
                        );
                    }
                    Some(
                        collect_source_type_occurrences(ast, &searched_name, source)
                            .into_iter()
                            .filter_map(|token| {
                                if target.as_ref().is_some_and(|target| {
                                    !include_declaration
                                        && module_path == target.module_path
                                        && token.line(source) == target.line
                                        && token.column(source) == target.column
                                }) {
                                    return None;
                                }

                                Some(Location {
                                    uri: uri.clone(),
                                    range: token_to_editor_range(source, &token),
                                })
                            })
                            .collect::<Vec<Location>>(),
                    )
                })
                .flatten()
            {
                locations.append(&mut module_locations);
            }
        }

        locations.sort_by(|left, right| {
            left.uri
                .as_str()
                .cmp(right.uri.as_str())
                .then_with(|| left.range.start.line.cmp(&right.range.start.line))
                .then_with(|| left.range.start.character.cmp(&right.range.start.character))
        });

        Ok(Some(locations))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let module_path = self.uri_to_module_path(&uri);

        let symbols =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |_, ast, module| {
                    let source = module.borrow_source();
                    #[allow(deprecated)]
                    ast.iter()
                        .filter_map(|decl| match decl {
                            Declaration::Import { .. } | Declaration::Custom { .. } => None,
                            Declaration::Type { name, .. } => Some(SymbolInformation {
                                name: name.text(source).to_string(),
                                kind: SymbolKind::STRUCT,
                                tags: None,
                                deprecated: None,
                                location: Location {
                                    uri: uri.clone(),
                                    range: token_to_editor_range(module.borrow_source(), name),
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

        let result = self.registry.with_module(
            module_path.as_deref().unwrap_or(""),
            |tokens, ast, module| {
                let source = module.borrow_source();
                let token = Self::find_token_at_position(source, tokens, position)?;
                if token.0 != TokenVariant::Identifier {
                    return None;
                }

                // Only allow declaration-local generic parameters and
                // user-defined type declarations to be renamed.
                let is_user_defined = generic_parameter_target_at(ast, &token.1, source).is_some()
                    || (self
                        .registry
                        .find_declaration(
                            module_path.as_deref().unwrap_or(""),
                            token.1.text(source),
                        )
                        .is_some()
                        && collect_source_type_occurrences(ast, token.1.text(source), source)
                            .contains(&token.1));

                if !is_user_defined {
                    return None;
                }

                Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: token_to_editor_range(source, &token.1),
                    placeholder: token.1.text(source).to_string(),
                })
            },
        );

        Ok(result.flatten())
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let module_path = self.uri_to_module_path(&uri);

        let result = self.registry.with_module(
            module_path.as_deref().unwrap_or(""),
            |tokens, ast, module| {
                let source = module.borrow_source();
                let token = Self::find_token_at_position(source, tokens, position)?;
                if token.0 != TokenVariant::Identifier {
                    return None;
                }

                let old_name = token.1.text(source);

                if let Some(generic) = generic_parameter_target_at(ast, &token.1, source) {
                    let edits = generic
                        .occurrences
                        .into_iter()
                        .map(|occurrence| TextEdit {
                            range: occurrence.to_editor_range(source),
                            new_text: new_name.clone(),
                        })
                        .collect();
                    let mut changes = HashMap::new();
                    changes.insert(uri.clone(), edits);
                    return Some((
                        old_name.to_string(),
                        Some(WorkspaceEdit {
                            changes: Some(changes),
                            ..Default::default()
                        }),
                    ));
                }

                let is_user_defined = self
                    .registry
                    .find_declaration(module_path.as_deref().unwrap_or(""), old_name)
                    .is_some()
                    && collect_source_type_occurrences(ast, old_name, source).contains(&token.1);

                if !is_user_defined {
                    return None;
                }

                Some((old_name.to_string(), None))
            },
        );

        let Some((_, generic)) = result.flatten() else {
            return Ok(None);
        };
        if generic.is_some() {
            return Ok(generic);
        }
        let locations = self
            .references(ReferenceParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position,
                },
                context: ReferenceContext {
                    include_declaration: true,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .await?
            .unwrap_or_default();
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for location in locations {
            changes.entry(location.uri).or_default().push(TextEdit {
                range: location.range,
                new_text: new_name.clone(),
            });
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }))
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
    let runtime = Arc::new(RuntimeState {
        registry: RwLock::new(Arc::new(reg)),
        documents: Mutex::new(HashMap::new()),
        updates: tokio::sync::Mutex::new(()),
        coordinator: Mutex::new(()),
        config_watcher: Mutex::new(None),
        stopped: std::sync::atomic::AtomicBool::new(false),
    });
    let service_runtime = Arc::clone(&runtime);

    let (service, socket) = LspService::new(move |client| LiveBackend {
        client,
        runtime: Arc::clone(&service_runtime),
    });

    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
    runtime.config_watcher.lock().unwrap().take();
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::lexer::Lexer;
    use xenomorph_common::module::types::ErrorPhase;
    use xenomorph_common::XenoDiagSeverity;

    struct ReferenceWorkspace {
        directory: std::path::PathBuf,
        registry: Arc<XenoRegistry>,
    }

    impl ReferenceWorkspace {
        fn new(sources: &[(&str, &str)]) -> Self {
            Self::with_types(sources, &[])
        }

        fn with_types(sources: &[(&str, &str)], types: &[OwnedSemanticType]) -> Self {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "xenomorph-lsp-references-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir(&directory).unwrap();
            for (name, source) in sources {
                std::fs::write(directory.join(format!("{name}.xen")), source).unwrap();
            }
            let mut config = xenomorph_common::config::Config::default();
            config.workdir = directory.clone();
            config.parser.entry = sources[0].0.into();
            let mut registry = XenoRegistry::with_config(config, false).unwrap();
            registry
                .metadata
                .types
                .extend(types.iter().map(|ty| (ty.name.clone(), ty.clone())));
            registry
                .hierarchy
                .write()
                .unwrap()
                .insert_metadata(&registry.metadata)
                .unwrap();
            for (name, _) in sources {
                registry.load_module(&[name], true, None);
                registry.with_module(name, |_, _, module| {
                    assert!(module.borrow_parser_errors().is_empty());
                    assert!(module.borrow_lexer_errors().is_empty());
                });
            }
            Self {
                directory,
                registry: Arc::new(registry),
            }
        }

        fn location(&self, module: &str, line: u32, start: u32, end: u32) -> Location {
            Location {
                uri: Url::from_file_path(self.directory.join(format!("{module}.xen"))).unwrap(),
                range: Range::new(Position::new(line, start), Position::new(line, end)),
            }
        }

        async fn references(
            &self,
            module: &str,
            position: Position,
            include_declaration: bool,
        ) -> Option<Vec<Location>> {
            let registry = Arc::clone(&self.registry);
            let (service, _socket) = LspService::new(|client| Backend {
                client,
                registry: Arc::clone(&registry),
                runtime: Arc::new(RuntimeState {
                    registry: RwLock::new(registry),
                    documents: Mutex::new(HashMap::new()),
                    updates: tokio::sync::Mutex::new(()),
                    coordinator: Mutex::new(()),
                    config_watcher: Mutex::new(None),
                    stopped: std::sync::atomic::AtomicBool::new(false),
                }),
            });
            service
                .inner()
                .references(ReferenceParams {
                    text_document_position: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier {
                            uri: self.location(module, 0, 0, 0).uri,
                        },
                        position,
                    },
                    context: ReferenceContext {
                        include_declaration,
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                })
                .await
                .unwrap()
        }
    }

    impl Drop for ReferenceWorkspace {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn document_changes_apply_utf16_ranges_sequentially() {
        let source = "// \u{1f600}\r\ntype A = string;";
        let changes = serde_json::from_value(serde_json::json!([
            { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 5 } }, "text": "header" },
            { "range": { "start": { "line": 1, "character": 5 }, "end": { "line": 1, "character": 6 } }, "text": "Renamed" },
            { "range": { "start": { "line": 1, "character": 15 }, "end": { "line": 1, "character": 21 } }, "text": "number" }
        ])).unwrap();
        assert_eq!(
            apply_document_changes(source, changes).unwrap(),
            "// header\r\ntype Renamed = number;"
        );
        let invalid = serde_json::from_value(serde_json::json!([
            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "text": "x" }
        ])).unwrap();
        assert!(apply_document_changes(source, invalid).is_none());
    }

    #[test]
    fn current_graph_follows_transitive_imports_and_cycles_and_excludes_cached_orphans() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "import middle::Middle; type Root = string;"),
            ("middle", "import leaf::Leaf; type Middle = string;"),
            ("leaf", "import middle::Middle; type Leaf = string;"),
            ("orphan", "type Orphan = string;"),
        ]);
        assert_eq!(
            workspace.registry.module_graph(),
            HashSet::from(["index".into(), "middle".into(), "leaf".into()]),
        );
    }

    #[test]
    fn unused_diagnostics_follow_entry_exports_and_private_dependencies() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "pub import other::Api; type Unused = string;"),
            (
                "other",
                "type Helper = string; pub type Api = Helper; pub type Spare = string;",
            ),
        ]);
        let graph = workspace.registry.module_graph();
        let uri = workspace.location("other", 0, 0, 0).uri;
        let diagnostics = diagnostics_for_document(&workspace.registry, &graph, &uri, None);
        let core_errors = workspace.registry.get_all_errors_for("other");
        assert_eq!(diagnostics, diagnostics_for_module(&core_errors, "other"));
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d
                    .tags
                    .as_ref()
                    .is_some_and(|t| t.contains(&DiagnosticTag::UNNECESSARY)))
                .count(),
            1
        );
        assert_eq!(diagnostics[0].range.start.character, 45);
        workspace.registry.load_module_from_source(
            &workspace.directory.join("index.xen"),
            "import other::Api; type Unused = string;".into(),
        );
        let diagnostics = diagnostics_for_document(&workspace.registry, &graph, &uri, None);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d
                    .tags
                    .as_ref()
                    .is_some_and(|t| t.contains(&DiagnosticTag::UNNECESSARY)))
                .count(),
            3
        );
        let index_uri = workspace.location("index", 0, 0, 0).uri;
        let diagnostics = diagnostics_for_document(&workspace.registry, &graph, &index_uri, None);
        let import = diagnostics
            .iter()
            .find(|d| d.message == "Unused import 'Api'.")
            .unwrap();
        assert_eq!(
            import.range,
            Range::new(Position::new(0, 14), Position::new(0, 17))
        );
        assert_eq!(import.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(import.tags, Some(vec![DiagnosticTag::UNNECESSARY]));
    }

    #[tokio::test]
    async fn import_completions_offer_only_public_declarations_in_groups_and_single_paths() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "pub import other::Open;"),
            (
                "other",
                "pub type Open = string; pub type Fresh = number; type Private = string;",
            ),
            ("barrel", "pub import other::Open;"),
        ]);
        let registry = Arc::clone(&workspace.registry);
        let (service, _socket) = LspService::new(|client| Backend {
            client,
            registry: Arc::clone(&registry),
            runtime: Arc::new(RuntimeState {
                registry: RwLock::new(registry),
                documents: Mutex::new(HashMap::new()),
                updates: tokio::sync::Mutex::new(()),
                coordinator: Mutex::new(()),
                config_watcher: Mutex::new(None),
                stopped: std::sync::atomic::AtomicBool::new(false),
            }),
        });
        std::fs::write(
            workspace.directory.join("uncached.xen"),
            "pub type Open = string; type Private = string;",
        )
        .unwrap();
        for source in [
            "pub import other::{",
            "import other::",
            "import other::{O",
            "// imports\npub import other::{",
            "import uncached::{O",
        ] {
            let tokens = Lexer::tokenize(source).unwrap();
            let doc = xenomorph_common::SourceDocument::new(source.to_string());
            let (line, character) = doc.position(source.len() as u32, true);
            let items = service.inner().get_context_completions(
                &tokens,
                &[],
                source,
                Position::new(line, character),
                Some("index"),
            );
            assert!(
                items.iter().any(|item| item.label == "Open"),
                "{source}: {items:?}"
            );
            assert!(!items.iter().any(|item| item.label == "Private"));
        }
        for (template, expected) in [
            ("import other::Open; import other::{|", vec!["Fresh"]),
            ("pub import barrel::Open; import other::{|", vec!["Fresh"]),
            ("import other::{Open, |", vec!["Fresh"]),
            ("import other::{|, Open};", vec!["Fresh"]),
            ("import other::{Fresh, |Open};", vec!["Open"]),
            ("import other::{Open}; import other::|", vec!["Fresh"]),
            ("import other::{|}; import other::Open;", vec!["Fresh"]),
            ("import other::{O|pen};", vec!["Open"]),
            ("import other::{// \u{1f600}\nOpen, |", vec!["Fresh"]),
        ] {
            let offset = template.find('|').unwrap() as u32;
            let source = template.replace('|', "");
            let tokens = Lexer::tokenize(&source).unwrap();
            let (ast, _) = xenomorph_common::parser::Parser::parse(&tokens, &source);
            let document = xenomorph_common::SourceDocument::new(source.clone());
            let (line, character) = document.position(offset, true);
            let items = service.inner().get_context_completions(
                &tokens,
                &ast,
                &source,
                Position::new(line, character),
                Some("index"),
            );
            let mut names: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, expected, "{template}");
        }
        assert!(!workspace
            .registry
            .index
            .read()
            .unwrap()
            .contains_key("uncached"));
    }

    #[tokio::test]
    async fn rename_updates_imported_names_and_declarations_without_changing_paths_or_fields() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "pub import Named::Named;"),
            ("Named", "pub type Named = { Named: string };"),
        ]);
        let registry = Arc::clone(&workspace.registry);
        let (service, _socket) = LspService::new(|client| Backend {
            client,
            registry: Arc::clone(&registry),
            runtime: Arc::new(RuntimeState {
                registry: RwLock::new(registry),
                documents: Mutex::new(HashMap::new()),
                updates: tokio::sync::Mutex::new(()),
                coordinator: Mutex::new(()),
                config_watcher: Mutex::new(None),
                stopped: std::sync::atomic::AtomicBool::new(false),
            }),
        });
        let result = service
            .inner()
            .rename(RenameParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: workspace.location("index", 0, 0, 0).uri,
                    },
                    position: Position::new(0, 19),
                },
                new_name: "Renamed".into(),
                work_done_progress_params: Default::default(),
            })
            .await
            .unwrap()
            .unwrap();
        let changes = result.changes.unwrap();
        assert_eq!(
            changes[&workspace.location("index", 0, 0, 0).uri],
            vec![TextEdit {
                range: workspace.location("index", 0, 18, 23).range,
                new_text: "Renamed".into()
            }]
        );
        assert_eq!(
            changes[&workspace.location("Named", 0, 0, 0).uri],
            vec![TextEdit {
                range: workspace.location("Named", 0, 9, 14).range,
                new_text: "Renamed".into()
            }]
        );
    }

    #[test]
    fn synchronizing_documents_follows_unsaved_transitive_imports() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "type Root = string;"),
            ("middle", "type Middle = string;"),
            ("leaf", "type Saved = string;"),
        ]);
        let documents = HashMap::from([
            (
                workspace.location("index", 0, 0, 0).uri,
                OpenDocument {
                    text: "import middle::Middle; type Root = string;".into(),
                    version: 1,
                },
            ),
            (
                workspace.location("middle", 0, 0, 0).uri,
                OpenDocument {
                    text: "import leaf::Leaf; type Middle = string;".into(),
                    version: 1,
                },
            ),
            (
                workspace.location("leaf", 0, 0, 0).uri,
                OpenDocument {
                    text: "type Draft = string;".into(),
                    version: 1,
                },
            ),
        ]);
        synchronize_documents(&workspace.registry, &documents);
        assert_eq!(
            workspace.registry.module_graph(),
            HashSet::from(["index".into(), "middle".into(), "leaf".into()]),
        );
        assert!(workspace
            .registry
            .with_module("leaf", |_, _, data| data.declarations.contains_key("Draft"))
            .unwrap());
    }

    #[tokio::test]
    async fn older_document_versions_cannot_restore_removed_imports() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "import other::Other; type Root = string;"),
            ("other", "type Other = string; type Root = string;"),
        ]);
        assert!(!workspace.registry.get_all_errors_for("index").is_empty());
        let registry = Arc::clone(&workspace.registry);
        let (service, _socket) = LspService::new(|client| Backend {
            client,
            registry: Arc::clone(&registry),
            runtime: Arc::new(RuntimeState {
                registry: RwLock::new(registry),
                documents: Mutex::new(HashMap::new()),
                updates: tokio::sync::Mutex::new(()),
                coordinator: Mutex::new(()),
                config_watcher: Mutex::new(None),
                stopped: std::sync::atomic::AtomicBool::new(false),
            }),
        });
        let uri = workspace.location("index", 0, 0, 0).uri;
        let backend = service.inner();
        backend
            .validate_document(
                &uri,
                "// import other::Other;\ntype Root = string;".into(),
                3,
            )
            .await;
        backend
            .validate_document(&uri, "import other::Other;\ntype Root = string;".into(), 2)
            .await;
        assert_eq!(
            workspace.registry.module_graph(),
            HashSet::from(["index".into()])
        );
        assert!(workspace
            .registry
            .with_module("other", |_, _, _| ())
            .is_none());
        assert!(workspace
            .registry
            .hierarchy
            .read()
            .unwrap()
            .get_type_in(Some("other"), "Other")
            .is_none());
        assert_eq!(backend.runtime.documents.lock().unwrap()[&uri].version, 3);
        let diagnostics = workspace.registry.get_all_errors_for("index");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, XenoDiagSeverity::Info);
        assert_eq!(
            diagnostics[0].message,
            "Not used by the entry module's exports."
        );
        assert_eq!(
            diagnostics[0].tags,
            [xenomorph_common::XenoDiagnosticTag::Unnecessary]
        );
    }

    #[tokio::test]
    async fn references_find_builtin_types_across_indexed_modules() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "type Text = string;"),
            ("other", "type Other = string[];"),
        ]);
        for include_declaration in [false, true] {
            let references = workspace
                .references("index", Position::new(0, 13), include_declaration)
                .await;
            assert_eq!(
                references,
                Some(vec![
                    workspace.location("index", 0, 12, 18),
                    workspace.location("other", 0, 13, 19),
                ])
            );
        }
    }

    #[tokio::test]
    async fn references_find_plugin_types_across_indexed_modules() {
        let workspace = ReferenceWorkspace::with_types(
            &[
                ("index", "type Symbol = symbol;"),
                ("other", "type Symbols = Dict<symbol, symbol[]>;"),
            ],
            &[OwnedSemanticType {
                name: "symbol".into(),
                documentation: None,
                generic_params: Vec::new(),
                parents: vec![OwnedConstraint::Trait("KeyTrait".into())],
            }],
        );
        assert_eq!(
            workspace
                .references("other", Position::new(0, 21), true)
                .await,
            Some(vec![
                workspace.location("index", 0, 14, 20),
                workspace.location("other", 0, 20, 26),
                workspace.location("other", 0, 28, 34),
            ])
        );
    }

    #[tokio::test]
    async fn references_find_static_types_in_all_type_expression_forms() {
        let sources = [
            "type Example = ?u64;",
            "type Example = [u64, u64[]];",
            "type Example = | u64 | string;",
            "type Example = & u64 & number;",
            "type Example = set<u64> [1 as u64];",
            "type Example = { value: Dict<string, u64[]> };",
            "type Example = enum { value: u64 };",
            "type Example = u64 @min(1 as u64);",
            "type Example = u64 @example(@example(u64));",
            "type Example<T: u64> = T;",
        ];
        for source in sources {
            let workspace = ReferenceWorkspace::new(&[("index", source)]);
            let expected: Vec<_> = source
                .match_indices("u64")
                .map(|(start, _)| workspace.location("index", 0, start as u32, start as u32 + 3))
                .collect();
            assert_eq!(
                workspace
                    .references("index", expected[0].range.start, false)
                    .await,
                Some(expected),
                "{source}"
            );
        }
    }

    #[tokio::test]
    async fn references_exclude_non_type_identifiers_comments_and_literals() {
        let source =
            "// string\ntype Text = { string: string, literal: \"string\", value: ?string[] };";
        let workspace = ReferenceWorkspace::new(&[("index", source)]);
        assert_eq!(
            workspace
                .references("index", Position::new(1, 24), false)
                .await,
            Some(vec![
                workspace.location("index", 1, 22, 28),
                workspace.location("index", 1, 57, 63),
            ])
        );
    }

    #[tokio::test]
    async fn references_keep_source_declarations_scoped_and_honor_include_declaration() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "pub type Named = string; type Alias = Named;"),
            ("other", "import index::Named; type Other = Named;"),
            ("unrelated", "type Named = u64; type Separate = Named;"),
        ]);
        let expected = vec![
            workspace.location("index", 0, 38, 43),
            workspace.location("other", 0, 14, 19),
            workspace.location("other", 0, 34, 39),
        ];
        assert_eq!(
            workspace
                .references("other", Position::new(0, 35), false)
                .await,
            Some(expected.clone())
        );
        assert_eq!(
            workspace
                .references("index", Position::new(0, 10), true)
                .await,
            Some([vec![workspace.location("index", 0, 9, 14)], expected].concat())
        );
    }

    #[tokio::test]
    async fn references_keep_generic_parameters_scoped_to_their_declaration() {
        let workspace = ReferenceWorkspace::new(&[
            ("index", "type First<T> = T; type Second<T> = T;"),
            ("other", "type Third<T> = T;"),
        ]);
        assert_eq!(
            workspace
                .references("index", Position::new(0, 16), false)
                .await,
            Some(vec![workspace.location("index", 0, 16, 17)])
        );
        assert_eq!(
            workspace
                .references("index", Position::new(0, 16), true)
                .await,
            Some(vec![
                workspace.location("index", 0, 11, 12),
                workspace.location("index", 0, 16, 17),
            ])
        );
    }

    #[tokio::test]
    async fn references_do_not_resolve_unknown_types() {
        let workspace = ReferenceWorkspace::new(&[("index", "type Alias = Unknown;")]);
        assert_eq!(
            workspace
                .references("index", Position::new(0, 14), false)
                .await,
            None
        );
    }

    #[test]
    fn diagnostic_ending_inside_crlf_stops_at_the_content_column() {
        let source = xenomorph_common::SourceDocument::new("a\u{e9}\u{1f600}\r\nnext".into());
        let errors = [ModuleDiagnostic {
            tags: vec![xenomorph_common::XenoDiagnosticTag::Unnecessary],
            module_path: "test".into(),
            message: "boundary diagnostic".into(),
            location: Some((0, 1, 7)),
            phase: ErrorPhase::Parser,
            severity: XenoDiagSeverity::Err,
        }];
        let diagnostics = diagnostics_for_source(&errors, "test", &source);
        assert_eq!(
            diagnostics[0].range,
            Range::new(Position::new(0, 1), Position::new(0, 4))
        );
        assert_eq!(diagnostics[0].tags, Some(vec![DiagnosticTag::UNNECESSARY]));
    }

    fn context_at_end(source: &str) -> Option<(String, usize)> {
        let tokens = Lexer::tokenize(source).expect("completion fixture should lex");
        annotation_argument_context(
            &tokens,
            source,
            Position {
                line: 0,
                character: source.len() as u32,
            },
        )
        .map(|(name, index)| (name.to_string(), index))
    }

    #[test]
    fn annotation_context_tracks_variadic_parameter_index() {
        assert_eq!(
            context_at_end("@Lombok(Data, "),
            Some(("Lombok".to_string(), 1))
        );
    }

    #[test]
    fn qualified_annotation_context_tracks_names_and_nested_parameters() {
        assert_eq!(
            context_at_end("@java::Lombok(java::Data, "),
            Some(("java::Lombok".into(), 1))
        );
        assert_eq!(
            context_at_end("@plugin::Outer(@plugin::Inner(string, "),
            Some(("plugin::Inner".into(), 1))
        );
        assert_eq!(
            context_at_end("@plugin::Outer(@plugin::Inner(string), "),
            Some(("plugin::Outer".into(), 1))
        );
    }

    #[test]
    fn annotation_context_ignores_nested_expression_commas() {
        assert_eq!(
            context_at_end("@Outer(@Inner(first, second), "),
            Some(("Outer".to_string(), 1))
        );
        assert_eq!(
            context_at_end("@Outer(@Inner(first, "),
            Some(("Inner".to_string(), 1))
        );
    }

    #[test]
    fn generic_constraint_context_only_matches_colons_inside_angle_parameters() {
        let generic = "type Box<T: Number";
        let generic_tokens = Lexer::tokenize(generic).expect("generic fixture should lex");
        assert!(generic_constraint_context(
            &generic_tokens,
            generic,
            Position::new(0, generic.len() as u32)
        ));

        let field = "type Box = { value: Number";
        let field_tokens = Lexer::tokenize(field).expect("field fixture should lex");
        assert!(!generic_constraint_context(
            &field_tokens,
            field,
            Position::new(0, field.len() as u32)
        ));
    }

    #[test]
    fn completion_context_uses_type_content_before_terminating_semicolon() {
        let source = "type A = string @minlen(10);";
        let tokens = Lexer::tokenize(source).expect("completion fixture should lex");
        let semicolon = source
            .find(';')
            .expect("fixture should contain a semicolon") as u32;

        let token =
            Backend::find_token_before_or_at_position(source, &tokens, Position::new(0, semicolon))
                .expect("type content should precede the cursor");

        assert_eq!(token.0, TokenVariant::RParen);
    }

    #[test]
    fn completion_context_uses_terminator_after_semicolon() {
        let source = "type A = string @minlen(10);";
        let tokens = Lexer::tokenize(source).expect("completion fixture should lex");

        let token = Backend::find_token_before_or_at_position(
            source,
            &tokens,
            Position::new(0, source.len() as u32),
        )
        .expect("declaration terminator should precede the cursor");

        assert_eq!(token.0, TokenVariant::Semicolon);
    }

    #[test]
    fn generic_parameter_target_is_scoped_to_its_declaration() {
        let source = "type First<T> = T; type Second<T> = Box<T>;";
        let tokens = Lexer::tokenize(source).expect("generic fixture should lex");
        let (ast, diagnostics) = xenomorph_common::parser::Parser::parse(&tokens, source);
        assert!(diagnostics.is_empty());
        let selected = tokens
            .iter()
            .filter(|token| token.0 == TokenVariant::Identifier && token.1.text(source) == "T")
            .nth(3)
            .expect("second generic reference should exist");

        let target = generic_parameter_target_at(&ast, &selected.1, source)
            .expect("generic reference should resolve");

        let second_start = source.find("Second").unwrap() as u32;
        assert_eq!(target.name, "T");
        assert_eq!(target.occurrences.len(), 2);
        assert_eq!(target.declaration.scalar_column, second_start + 7);
        assert!(target
            .occurrences
            .iter()
            .all(|occurrence| occurrence.scalar_column >= second_start));
    }

    #[test]
    fn generic_parameter_target_collects_nested_and_annotation_references() {
        let source = "type Result<T> = { value: Box<T>, items: T[] } @example(T);";
        let tokens = Lexer::tokenize(source).expect("generic fixture should lex");
        let (ast, diagnostics) = xenomorph_common::parser::Parser::parse(&tokens, source);
        assert!(diagnostics.is_empty());
        let declaration = tokens
            .iter()
            .find(|token| token.0 == TokenVariant::Identifier && token.1.text(source) == "T")
            .expect("generic declaration should exist");

        let target = generic_parameter_target_at(&ast, &declaration.1, source)
            .expect("generic declaration should resolve");

        assert_eq!(target.occurrences.len(), 4);
    }

    #[test]
    fn generic_parameter_target_does_not_treat_constraints_as_parameter_uses() {
        let source = "type Result<T: T> = T;";
        let tokens = Lexer::tokenize(source).expect("generic fixture should lex");
        let (ast, diagnostics) = xenomorph_common::parser::Parser::parse(&tokens, source);
        assert!(diagnostics.is_empty());
        let declaration = tokens
            .iter()
            .find(|token| token.0 == TokenVariant::Identifier && token.1.text(source) == "T")
            .expect("generic declaration should exist");

        let target = generic_parameter_target_at(&ast, &declaration.1, source)
            .expect("generic declaration should resolve");

        assert_eq!(target.occurrences.len(), 2);
    }

    #[test]
    fn editor_ranges_convert_scalar_columns_and_lengths_to_utf16() {
        let source = "type Emoji = [\"😀\", string];";
        let tokens = Lexer::tokenize(source).expect("UTF-16 fixture should lex");
        let string = tokens
            .iter()
            .find(|token| token.0 == TokenVariant::Identifier && token.1.text(source) == "string")
            .expect("type reference should exist");

        let range = token_to_editor_range(source, &string.1);

        assert_eq!(range.start.character, 20);
        assert_eq!(range.end.character, 26);
    }

    #[test]
    fn match_argument_completion_inserts_regex_literal() {
        let completion = Backend::regex_literal_completion();

        assert_eq!(completion.label, "regex literal");
        assert_eq!(completion.insert_text.as_deref(), Some("/${1:pattern}/"));
        assert_eq!(
            completion.insert_text_format,
            Some(InsertTextFormat::SNIPPET)
        );
    }

    #[test]
    fn diagnostics_are_only_published_for_their_own_module() {
        let errors = vec![
            ModuleDiagnostic {
                tags: vec![],
                module_path: "parser/p1".to_string(),
                message: "p1 error".to_string(),
                location: Some((1, 2, 3)),
                phase: ErrorPhase::Parser,
                severity: XenoDiagSeverity::Err,
            },
            ModuleDiagnostic {
                tags: vec![],
                module_path: "parser/p2".to_string(),
                message: "p2 error".to_string(),
                location: Some((4, 5, 6)),
                phase: ErrorPhase::Analyzer,
                severity: XenoDiagSeverity::Err,
            },
        ];

        let diagnostics = diagnostics_for_module(&errors, "parser/p2");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "p2 error");
        assert_eq!(diagnostics[0].range.start, Position::new(4, 5));
        assert_eq!(diagnostics[0].range.end, Position::new(4, 11));
    }
}
