use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

mod bindings;
pub mod types;
pub(crate) use bindings::in_scope;

use crate::config::Config;
use crate::lexer::{Lexed, Lexer, Token, XenoTokens};
use crate::module::types::{DeclarationInfo, ErrorPhase, ModuleDiagnostic, ModulePath};
use crate::parser::{Declaration, Parsed, Parser, XenoAst};
use crate::plugins::{load_plugins, LoadedPlugin, PluginMode};
use crate::semantic::{
    Analyzer, MetadataGraph, NameCollisionValidator, TypeDeclarationInfo, BUILTIN_ANNOTATIONS,
    BUILTIN_TRAITS, BUILTIN_TYPES,
};
use crate::utils::calculate_hash;
use crate::XenoDiagSeverity;

/// Information about a single module (one .xen file).
/// Owns the current source and every source-indexed artifact.
pub struct ModuleData {
    /// The absolute filesystem path.
    pub abs_path: PathBuf,
    /// Module path relative to workspace root, using '/' separators (e.g. "a/b").
    pub module_path: ModulePath,
    /// Owned source text — tokens and AST resolve ranges against this document.
    pub source: crate::SourceDocument,
    /// Hash of the source text, used for change detection.
    pub hash: u64,
    /// Lexer errors.
    pub lexer_errors: Vec<ModuleDiagnostic>,
    /// Parser errors.
    pub parser_errors: Vec<ModuleDiagnostic>,
    /// Semantic analyzer errors.
    pub analyzer_errors: Vec<ModuleDiagnostic>,
    /// Cache-wide type/member/generic name collision errors.
    pub collision_errors: Vec<ModuleDiagnostic>,
    /// Module-level errors (file not found, import resolution, etc.)
    pub module_errors: Vec<ModuleDiagnostic>,
    /// Modules that this module imports
    pub imports: Vec<ModulePath>,
    /// Changed flag
    pub changed: bool,
    /// Tokens of the module
    pub lexed: Lexed,
    /// AST of the module
    pub parsed: Parsed,
    pub custom_diagnostics: Vec<crate::XenoDiagnostic>,
    pub declarations: HashMap<String, DeclarationInfo>,
}

impl ModuleData {
    pub fn borrow_abs_path(&self) -> &PathBuf {
        &self.abs_path
    }
    pub fn borrow_module_path(&self) -> &ModulePath {
        &self.module_path
    }
    pub fn borrow_source(&self) -> &str {
        self.source.text()
    }
    pub fn borrow_hash(&self) -> &u64 {
        &self.hash
    }
    pub fn borrow_lexer_errors(&self) -> &Vec<ModuleDiagnostic> {
        &self.lexer_errors
    }
    pub fn borrow_parser_errors(&self) -> &Vec<ModuleDiagnostic> {
        &self.parser_errors
    }
    pub fn borrow_analyzer_errors(&self) -> &Vec<ModuleDiagnostic> {
        &self.analyzer_errors
    }
    pub fn borrow_collision_errors(&self) -> &Vec<ModuleDiagnostic> {
        &self.collision_errors
    }
    pub fn borrow_module_errors(&self) -> &Vec<ModuleDiagnostic> {
        &self.module_errors
    }
    pub fn borrow_imports(&self) -> &Vec<ModulePath> {
        &self.imports
    }
    pub fn borrow_tokens(&self) -> &XenoTokens {
        &self.lexed.tokens
    }
    pub fn borrow_ast(&self) -> &XenoAst {
        &self.parsed.ast
    }
    pub fn borrow_declarations(&self) -> &HashMap<String, DeclarationInfo> {
        &self.declarations
    }
    fn diagnostic(&self, error: crate::XenoDiagnostic, phase: ErrorPhase) -> ModuleDiagnostic {
        let (line, column) = self.source.position(error.location.start, false);
        ModuleDiagnostic {
            module_path: self.module_path.clone(),
            message: error.message,
            location: Some((line, column, error.location.len())),
            phase,
            severity: error.severity,
            tags: error.tags,
        }
    }
    fn update(
        &mut self,
        source: String,
        hash: u64,
        plugins: &[LoadedPlugin],
        matchers: &HashMap<String, usize>,
    ) {
        let lexed = Lexer::relex(self.source.text(), &self.lexed, &source);
        let parsed = if lexed.error.is_none() {
            Parser::parse_incremental(
                &lexed.tokens,
                &source,
                Some((&self.lexed.tokens, self.source.text(), &self.parsed)),
            )
        } else {
            Parsed::default()
        };
        // A name index selects one of several duplicate declarations. Its
        // semantic value cannot be reused for a surviving sibling by name.
        let mut seen = HashSet::new();
        for declaration in &self.parsed.ast {
            if let Declaration::Type { name, .. } = declaration {
                let name = name.text(self.source.text());
                if !seen.insert(name) {
                    self.declarations.remove(name);
                }
            }
        }
        self.source = crate::SourceDocument::new(source);
        self.hash = hash;
        self.lexed = lexed;
        self.parsed = parsed;
        self.parse_custom_blocks(plugins, matchers);
        self.index();
    }

    fn parse_custom_blocks(&mut self, plugins: &[LoadedPlugin], matchers: &HashMap<String, usize>) {
        self.custom_diagnostics.clear();
        for declaration in &self.parsed.ast {
            let Declaration::Custom {
                decl_id,
                location,
                content,
                public,
                ..
            } = declaration
            else {
                continue;
            };
            let Some(plugin) = matchers.get(decl_id).map(|&index| &plugins[index]) else {
                self.custom_diagnostics.push(crate::XenoDiagnostic {
                    tags: vec![],
                    location: *location,
                    severity: XenoDiagSeverity::Warn,
                    message: format!("Unknown custom declaration identifier '{decl_id}'"),
                });
                continue;
            };
            let request = crate::plugins::protocol::CustomDeclarationRequest {
                source: std::borrow::Cow::Borrowed(self.source.text()),
                module_path: self.module_path.clone(),
                abs_path: self.abs_path.clone(),
                identifier: decl_id.clone(),
                public: public.is_some(),
                location: *location,
                content: *content,
            };
            if let Err(diagnostic) = plugin.parse_custom(&request) {
                self.custom_diagnostics.push(diagnostic);
            }
        }
    }
    fn index(&mut self) {
        self.lexer_errors = self
            .lexed
            .error
            .clone()
            .into_iter()
            .map(|e| self.diagnostic(e, ErrorPhase::Lexer))
            .collect();
        self.parser_errors = self
            .parsed
            .diagnostics
            .iter()
            .chain(self.custom_diagnostics.iter())
            .cloned()
            .map(|e| self.diagnostic(e, ErrorPhase::Parser))
            .collect();
        self.imports = self
            .borrow_ast()
            .iter()
            .filter_map(|d| match d {
                Declaration::Import { path, .. } => Some(path.join("/")),
                _ => None,
            })
            .collect();
        let old_declarations = std::mem::take(&mut self.declarations);
        let reused: HashSet<_> = self.parsed.reused_indices.iter().copied().collect();
        self.declarations = self
            .borrow_ast()
            .iter()
            .enumerate()
            .filter_map(|(index, d)| match d {
                Declaration::Type {
                    docs,
                    name,
                    generics,
                    ty,
                    ..
                } => {
                    let text = name.text(self.source.text()).to_string();
                    let (line, column) = self.source.position(name.start, false);
                    Some((
                        text.clone(),
                        DeclarationInfo {
                            public: d.public().is_some(),
                            name: text.clone(),
                            module_path: self.module_path.clone(),
                            abs_path: self.abs_path.clone(),
                            docs: docs.clone(),
                            line,
                            column,
                            name_len: name.len(),
                            semantic: if reused.contains(&index) {
                                old_declarations.get(&text).map(|d| Arc::clone(&d.semantic))
                            } else {
                                None
                            }
                            .unwrap_or_else(|| {
                                Arc::new(TypeDeclarationInfo::from_ast(
                                    generics.as_deref(),
                                    &ty.0,
                                    self.source.text(),
                                ))
                            }),
                        },
                    ))
                }
                _ => None,
            })
            .collect();
        self.analyzer_errors.clear();
        self.collision_errors.clear();
        self.module_errors.clear();
        self.changed = true;
    }
}

#[derive(Clone)]
pub struct ModuleSummary {
    pub bindings: Vec<bindings::ImportBinding>,
    pub dependencies: HashMap<String, HashSet<String>>,
    pub abs_path: PathBuf,
    pub imports: Vec<ModulePath>,
    pub declarations: HashMap<String, DeclarationInfo>,
    pub references: HashSet<String>,
}

impl ModuleSummary {
    pub(crate) fn from_module(module: &ModuleData) -> Self {
        Self {
            abs_path: module.abs_path.clone(),
            imports: module.imports.clone(),
            declarations: module.declarations.clone(),
            bindings: bindings::import_bindings(module.borrow_ast(), module.borrow_source()),
            dependencies: module
                .borrow_ast()
                .iter()
                .filter_map(|d| match d {
                    Declaration::Type { name, .. } => Some((
                        name.text(module.borrow_source()).into(),
                        crate::semantic::declaration_dependencies(d, module.borrow_source()),
                    )),
                    _ => None,
                })
                .collect(),
            references: module
                .borrow_tokens()
                .iter()
                .filter(|(kind, _)| *kind == crate::lexer::TokenVariant::Identifier)
                .map(|(_, token)| token.text(module.borrow_source()).into())
                .collect(),
        }
    }
}

struct Change {
    path: String,
    names: HashSet<String>,
}

/// Determines the workspace root and entry module path from the config.
fn get_root(config: &Config) -> Result<(PathBuf, String), ModuleDiagnostic> {
    let mut joined = config.workdir.join(Path::new(&config.parser.entry));
    joined.add_extension("xen");
    let entry_file = joined.canonicalize().map_err(|e| ModuleDiagnostic {
        tags: vec![],
        module_path: config.parser.entry.clone(),
        message: format!("Cannot resolve entry file '{:?}': {}", joined, e),
        location: None,
        phase: ErrorPhase::Module,
        severity: XenoDiagSeverity::Err,
    })?;

    let root_err = || ModuleDiagnostic {
        tags: vec![],
        module_path: config.parser.entry.clone(),
        message: format!(
            "Cannot determine workspace root from entry file '{}'",
            entry_file.display()
        ),
        location: None,
        phase: ErrorPhase::Module,
        severity: XenoDiagSeverity::Err,
    };

    let root = entry_file
        .parent()
        .ok_or_else(root_err)?
        .canonicalize()
        .map_err(|_| root_err())?;

    Ok((
        root,
        entry_file
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ModuleDiagnostic {
                tags: vec![],
                module_path: config.parser.entry.clone(),
                message: format!(
                    "Entry file '{}' does not have a valid stem",
                    entry_file.display()
                ),
                location: None,
                phase: ErrorPhase::Module,
                severity: XenoDiagSeverity::Err,
            })?,
    ))
}

/// Thread-safe module registry. Single source of truth for all module data.
pub struct XenoRegistry {
    pub module_cache: RwLock<HashMap<ModulePath, Arc<RwLock<ModuleData>>>>,
    pub index: RwLock<HashMap<ModulePath, ModuleSummary>>,
    pub hierarchy: RwLock<crate::semantic::TypeHierarchy>,
    pub coordinator: Mutex<()>,
    pub root: PathBuf,
    pub entry: String,
    pub config: Arc<Config>,
    pub metadata: MetadataGraph,
    pub plugins: Vec<LoadedPlugin>,
    pub custom_matchers: HashMap<String, usize>,
    pub analyzer: Analyzer,
}

impl XenoRegistry {
    pub fn new(generation_mode: bool) -> Result<XenoRegistry, ModuleDiagnostic> {
        let config = std::env::current_dir()
            .map_err(|error| error.to_string())
            .and_then(|directory| Config::discover(&directory))
            .map_err(Self::runtime_error)?;
        Self::with_config(config, generation_mode)
    }

    pub fn with_config(
        config: Config,
        generation_mode: bool,
    ) -> Result<XenoRegistry, ModuleDiagnostic> {
        let (root, entry) = get_root(&config)?;
        let mode = if generation_mode {
            PluginMode::Generation
        } else {
            PluginMode::Analysis
        };
        let plugins = load_plugins(&config, mode).map_err(Self::runtime_error)?;
        let custom_matchers = plugins
            .iter()
            .enumerate()
            .flat_map(|(index, plugin)| {
                plugin
                    .description
                    .custom_declarations
                    .iter()
                    .cloned()
                    .map(move |identifier| (identifier, index))
            })
            .collect();
        let mut metadata =
            MetadataGraph::from_all_descriptors(BUILTIN_TYPES, BUILTIN_TRAITS, BUILTIN_ANNOTATIONS);
        for plugin in &plugins {
            metadata
                .merge(&plugin.description.metadata)
                .map_err(Self::runtime_error)?;
        }
        let analyzer = Analyzer::with_metadata(generation_mode, metadata.clone())
            .map_err(Self::runtime_error)?;
        let mut hierarchy = crate::semantic::TypeHierarchy::default();
        hierarchy
            .insert_metadata(&metadata)
            .map_err(Self::runtime_error)?;
        Ok(XenoRegistry {
            module_cache: RwLock::new(HashMap::default()),
            index: RwLock::new(HashMap::new()),
            hierarchy: RwLock::new(hierarchy),
            coordinator: Mutex::new(()),
            root,
            entry,
            analyzer,
            config: Arc::new(config),
            metadata,
            plugins,
            custom_matchers,
        })
    }

    fn runtime_error(message: String) -> ModuleDiagnostic {
        ModuleDiagnostic {
            tags: vec![],
            module_path: String::new(),
            message,
            location: None,
            phase: ErrorPhase::Module,
            severity: XenoDiagSeverity::Err,
        }
    }

    /// Initializes a new `XenoRegistry` and loads the entire workspace starting from the entry module.
    pub fn load_workspace(generation_mode: bool) -> Result<XenoRegistry, Vec<ModuleDiagnostic>> {
        let reg = XenoRegistry::new(generation_mode).map_err(|e| vec![e])?;
        Self::finish_workspace(reg)
    }

    pub fn load_workspace_with_config(
        config: Config,
        generation_mode: bool,
    ) -> Result<XenoRegistry, Vec<ModuleDiagnostic>> {
        Self::with_config(config, generation_mode)
            .map_err(|error| vec![error])?
            .finish_workspace()
    }

    fn finish_workspace(self) -> Result<XenoRegistry, Vec<ModuleDiagnostic>> {
        let errs = self.load_module(&[&self.entry], true, None);
        if Self::has_fatal_diagnostics(&errs) {
            return Err(errs);
        }
        Ok(self)
    }

    fn has_fatal_diagnostics(diagnostics: &[ModuleDiagnostic]) -> bool {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err)
    }

    // ── Path utilities ──────────────────────────────────────────────

    /// Converts an absolute file path to a ModulePath relative to the workspace root.
    /// e.g. "C:/workspace/api/user.xen" → "api/user"
    pub fn abs_path_to_module_path(&self, abs_path: &Path) -> Option<ModulePath> {
        let canonical = abs_path.canonicalize().ok()?;
        let relative = canonical.strip_prefix(&self.root).ok()?;
        Some(relative.with_extension("").to_str()?.replace('\\', "/"))
    }

    // ── Module loading ──────────────────────────────────────────────

    /// Loads a module from a given absolute file path string.
    pub fn load_module_from_uri(&self, uri: &str) -> Vec<ModuleDiagnostic> {
        let path_res = PathBuf::from(uri)
            .canonicalize()
            .map_err(|e| ModuleDiagnostic {
                tags: vec![],
                module_path: uri.to_string(),
                message: format!("Cannot resolve URI '{}': {}", uri, e),
                location: None,
                phase: ErrorPhase::Module,
                severity: XenoDiagSeverity::Err,
            });
        let path = match path_res {
            Ok(p) => p,
            Err(e) => return vec![e],
        };

        let relative = match path.strip_prefix(&self.root) {
            Ok(r) => r,
            Err(e) => {
                return vec![ModuleDiagnostic {
                    tags: vec![],
                    module_path: uri.to_string(),
                    message: format!(
                        "URI '{}' is not within workspace root '{}': {}",
                        uri,
                        self.root.display(),
                        e
                    ),
                    location: None,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                }]
            }
        };

        let segments: Vec<&str> = relative
            .iter()
            .filter_map(|s| s.to_str())
            .map(|s| s.trim_end_matches(".xen"))
            .collect();

        self.load_module(&segments, true, None)
    }

    /// Loads a module from in-memory source text (e.g. unsaved editor buffer).
    /// Returns all errors for this module (lexer + parser + analyzer + module).
    pub fn load_module_from_source(
        &self,
        abs_path: &Path,
        source: String,
    ) -> Vec<ModuleDiagnostic> {
        let module_path = match self.abs_path_to_module_path(abs_path) {
            Some(mp) => mp,
            None => {
                return vec![ModuleDiagnostic {
                    tags: vec![],
                    module_path: abs_path.to_string_lossy().to_string(),
                    message: format!(
                        "Path '{}' is not within workspace root '{}'",
                        abs_path.display(),
                        self.root.display()
                    ),
                    location: None,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                }]
            }
        };

        let canonical = match abs_path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                return vec![ModuleDiagnostic {
                    tags: vec![],
                    module_path,
                    message: format!("Cannot canonicalize '{}': {}", abs_path.display(), e),
                    location: None,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                }]
            }
        };

        let _update = self.coordinator.lock().unwrap();
        let changes = self
            .ingest(module_path, canonical, source)
            .into_iter()
            .collect();
        self.complete_update(changes);
        self.get_all_cached_errors()
    }

    pub fn load_module(
        &self,
        import_segments: &[&str],
        force: bool,
        import_str: Option<&str>,
    ) -> Vec<ModuleDiagnostic> {
        let (module_path, abs_path) = match self.resolve_import(import_segments, import_str) {
            Ok(value) => value,
            Err(error) => return vec![error],
        };
        let _update = self.coordinator.lock().unwrap();
        if !force && self.module_cache.read().unwrap().contains_key(&module_path) {
            return vec![];
        }
        let source = match fs::read_to_string(&abs_path) {
            Ok(source) => source,
            Err(error) => {
                return vec![Self::runtime_error(format!(
                    "Failed to read '{}': {error}",
                    abs_path.display()
                ))]
            }
        };
        let changes = self
            .ingest(module_path, abs_path, source)
            .into_iter()
            .collect();
        self.complete_update(changes);
        self.get_all_cached_errors()
    }

    fn ingest(&self, module_path: String, abs_path: PathBuf, source: String) -> Option<Change> {
        let hash = calculate_hash(&source);
        let handle = {
            let mut modules = self.module_cache.write().unwrap();
            modules
                .entry(module_path.clone())
                .or_insert_with(|| {
                    Arc::new(RwLock::new(
                        Self::_create_module_data(
                            &module_path,
                            abs_path,
                            String::new(),
                            calculate_hash(&""),
                            &self.config.debug,
                        )
                        .unwrap(),
                    ))
                })
                .clone()
        };
        // Source and all indexes are replaced while the same module writer is held.
        let mut module = handle.write().unwrap();
        if self.index.read().unwrap().contains_key(&module_path)
            && module.hash == hash
            && module.borrow_source() == source
        {
            return None;
        }
        let mut names: HashSet<_> = module.declarations.keys().cloned().collect();
        module.update(source, hash, &self.plugins, &self.custom_matchers);
        if self.config.debug.tokens {
            eprint!("{:?}", module.borrow_tokens());
        }
        if self.config.debug.ast {
            eprint!("{:?}", module.borrow_ast());
        }
        let summary = ModuleSummary::from_module(&module);
        names.extend(summary.declarations.keys().cloned());
        let mut hierarchy = self.hierarchy.write().unwrap();
        hierarchy.retain_module_declarations(
            &module_path,
            &summary.declarations.keys().cloned().collect(),
        );
        let index = self.index.read().unwrap();
        let previous = index.get(&module_path);
        hierarchy.register_module(module_path.clone(), summary.imports.clone());
        for d in summary.declarations.values() {
            if previous
                .and_then(|m| m.declarations.get(&d.name))
                .is_some_and(|old| Arc::ptr_eq(&old.semantic, &d.semantic))
            {
                continue;
            }
            hierarchy.insert_declaration(
                module_path.clone(),
                d.name.clone(),
                (*d.semantic).clone(),
            );
        }
        drop(index);
        self.index
            .write()
            .unwrap()
            .insert(module_path.clone(), summary);
        Some(Change {
            path: module_path,
            names,
        })
    }

    fn complete_update(&self, mut changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        // Ingest new imports before analyzing any module. No module lock is held
        // while another module is loaded, including malformed cyclic imports.
        loop {
            let pending: BTreeSet<_> = {
                let index = self.index.read().unwrap();
                index
                    .values()
                    .flat_map(|m| &m.imports)
                    .filter(|path| !index.contains_key(*path))
                    .cloned()
                    .collect()
            };
            let mut loaded = false;
            for path in pending {
                if let Ok((module, abs)) = self.resolve_import(&[], Some(&path)) {
                    if let Ok(source) = fs::read_to_string(&abs) {
                        if let Some(change) = self.ingest(module, abs, source) {
                            changes.push(change);
                        }
                        loaded = true;
                    }
                }
            }
            if !loaded {
                break;
            }
        }
        let affected = {
            let index = self.index.read().unwrap();
            let mut affected: HashSet<_> = changes.iter().map(|c| c.path.clone()).collect();
            let names: HashSet<_> = changes.iter().flat_map(|c| c.names.iter()).collect();
            for (path, module) in index.iter() {
                if module.references.iter().any(|name| names.contains(name)) {
                    affected.insert(path.clone());
                }
            }
            let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
            for (path, module) in index.iter() {
                for import in &module.imports {
                    reverse.entry(import).or_default().push(path);
                }
            }
            let mut pending: VecDeque<_> = affected.iter().cloned().collect();
            while let Some(path) = pending.pop_front() {
                for importer in reverse.get(path.as_str()).into_iter().flatten() {
                    if affected.insert((*importer).to_string()) {
                        pending.push_back((*importer).to_string());
                    }
                }
            }
            affected
        };
        let owners = self.collision_owners();
        self.refresh_bindings();
        let (order, blocked) = self.graph_order();
        // Kahn's remaining vertices are cycles and every importer of a cycle.
        for path in blocked {
            let handle = self.module_cache.read().unwrap().get(&path).cloned();
            if let Some(handle) = handle {
                let mut module = handle.write().unwrap();
                self.update_collisions(&mut module, &path, &owners);
                module.analyzer_errors.clear();
                module.module_errors = self.validate_imports(&module, &path);
                let location = module.borrow_ast().iter().find_map(|d| match d {
                    Declaration::Import { location, .. } => {
                        let (line, column) = module.source.position(location.start, false);
                        Some((line, column, location.len()))
                    }
                    _ => None,
                });
                module.module_errors.push(ModuleDiagnostic {
                    tags: vec![],
                    module_path: path,
                    message: "Module dependency cycle; semantic scans are deferred until the cycle is removed".into(),
                    location,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                });
            }
        }
        for path in order {
            let was_blocked = self
                .get_all_errors_for(&path)
                .iter()
                .any(|d| d.message.starts_with("Module dependency cycle"));
            if self.analyzer.generation_mode
                || self
                    .plugins
                    .iter()
                    .any(|p| p.capabilities & crate::plugins::abi::ANALYZER != 0)
                || affected.contains(&path)
                || was_blocked
            {
                self.revalidate_cached_module(&path, &owners);
            }
        }
        self.refresh_usage_diagnostics();
    }

    /// Recompute cheap usage hints across the graph even when semantic scans
    /// are reused. Changing entry exports can change usage in unchanged files.
    fn refresh_usage_diagnostics(&self) {
        let index = self.index.read().unwrap().clone();
        let reachable = bindings::reachable(&index, &self.entry);
        let handles = self.module_cache.read().unwrap().clone();
        for (path, handle) in handles {
            let mut module = handle.write().unwrap();
            module
                .analyzer_errors
                .retain(|d| !d.tags.contains(&crate::XenoDiagnosticTag::Unnecessary));
            let diagnostics = crate::semantic::analyze_usage(
                module.borrow_ast(),
                module.borrow_source(),
                &path,
                &self.imported_types(&index, &path),
                Some(&reachable),
            );
            let diagnostics: Vec<_> = diagnostics
                .into_iter()
                .map(|d| module.diagnostic(d, ErrorPhase::Analyzer))
                .collect();
            module.analyzer_errors.extend(diagnostics);
        }
    }

    fn graph_order(&self) -> (Vec<String>, Vec<String>) {
        let index = self.index.read().unwrap();
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut importers: HashMap<String, Vec<String>> = HashMap::new();
        for (path, module) in index.iter() {
            let imports: HashSet<_> = module
                .imports
                .iter()
                .filter(|p| index.contains_key(*p))
                .collect();
            counts.insert(path.clone(), imports.len());
            for import in imports {
                importers
                    .entry(import.clone())
                    .or_default()
                    .push(path.clone());
            }
        }
        let mut ready: BTreeSet<_> = counts
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(p, _)| p.clone())
            .collect();
        let mut order = vec![];
        while let Some(path) = ready.pop_first() {
            counts.remove(&path);
            if let Some(parents) = importers.get(&path) {
                for parent in parents {
                    let count = counts.get_mut(parent).unwrap();
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(parent.clone());
                    }
                }
            }
            order.push(path);
        }
        let mut blocked: Vec<_> = counts.into_keys().collect();
        blocked.sort();
        (order, blocked)
    }

    pub fn revalidate_importers(&self, changed_module: &str) -> Vec<ModulePath> {
        // Ingestion already performed the coordinated pass. This API reports the
        // affected modules for callers that publish editor diagnostics.
        let index = self.index.read().unwrap();
        Self::transitive_importers(&index, changed_module)
    }
    fn collision_owners(&self) -> BTreeMap<String, BTreeSet<String>> {
        let index = self.index.read().unwrap();
        let reachable = bindings::reachable(&index, &self.entry);
        let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (path, module) in index.iter() {
            for name in module.declarations.keys() {
                if self.analyzer.generation_mode
                    && !reachable.contains(&(path.clone(), name.clone()))
                {
                    continue;
                }
                owners.entry(name.clone()).or_default().insert(path.clone());
            }
        }
        owners
    }
    fn update_collisions(
        &self,
        module: &mut ModuleData,
        path: &str,
        owners: &BTreeMap<String, BTreeSet<String>>,
    ) {
        let static_types = self.metadata.types.keys().cloned().collect();
        let reachable = self.reachable_declarations();
        let ast: Vec<_> = module
            .borrow_ast()
            .iter()
            .filter(|d| {
                !self.analyzer.generation_mode
                    || match d {
                        Declaration::Type { name, .. } => reachable
                            .contains(&(path.into(), name.text(module.borrow_source()).into())),
                        _ => false,
                    }
            })
            .cloned()
            .collect();
        let diagnostics =
            NameCollisionValidator::new(path, module.borrow_source(), owners, &static_types)
                .validate(&ast);
        module.collision_errors = diagnostics
            .into_iter()
            .map(|d| module.diagnostic(d, ErrorPhase::Analyzer))
            .collect();
    }
    pub fn refresh_name_collision_diagnostics(&self) -> Vec<ModulePath> {
        let _update = self.coordinator.lock().unwrap();
        let owners = self.collision_owners();
        let handles = self.module_cache.read().unwrap().clone();
        let mut paths = vec![];
        for (path, handle) in handles {
            self.update_collisions(&mut handle.write().unwrap(), &path, &owners);
            paths.push(path);
        }
        paths.sort();
        paths
    }
    fn transitive_importers(
        cache: &HashMap<ModulePath, ModuleSummary>,
        changed_module: &str,
    ) -> Vec<ModulePath> {
        let mut visited = HashSet::from([changed_module.to_string()]);
        let mut pending = VecDeque::from([changed_module.to_string()]);
        let mut result = vec![];
        while let Some(path) = pending.pop_front() {
            let mut next: Vec<_> = cache
                .iter()
                .filter(|(_, m)| m.imports.contains(&path))
                .map(|(p, _)| p.clone())
                .collect();
            next.sort();
            for p in next {
                if visited.insert(p.clone()) {
                    pending.push_back(p.clone());
                    result.push(p);
                }
            }
        }
        result
    }
    #[cfg(test)]
    fn dependency_order(
        cache: &HashMap<ModulePath, ModuleSummary>,
        entry: &str,
    ) -> Vec<ModulePath> {
        fn visit(
            cache: &HashMap<ModulePath, ModuleSummary>,
            path: &str,
            seen: &mut HashSet<String>,
            out: &mut Vec<String>,
        ) {
            if !seen.insert(path.to_string()) {
                return;
            }
            if let Some(m) = cache.get(path) {
                for i in &m.imports {
                    visit(cache, i, seen, out)
                }
                out.push(path.to_string());
            }
        }
        let mut seen = HashSet::new();
        let mut out = vec![];
        visit(cache, entry, &mut seen, &mut out);
        let mut rest: Vec<_> = cache.keys().collect();
        rest.sort();
        for p in rest {
            visit(cache, p, &mut seen, &mut out)
        }
        out
    }
    fn revalidate_cached_module(
        &self,
        path: &str,
        owners: &BTreeMap<String, BTreeSet<String>>,
    ) -> bool {
        let handle = self.module_cache.read().unwrap().get(path).cloned();
        let Some(handle) = handle else {
            return false;
        };
        let mut module = handle.write().unwrap();
        self.update_collisions(&mut module, path, owners);
        let index = self.index.read().unwrap();
        let mut hierarchy = self.hierarchy.read().unwrap().clone();
        hierarchy.set_current_module(path);
        let scope = crate::semantic::ScopeInfo {
            module_path: path.into(),
            abs_path: module.abs_path.clone(),
            own_types: module.declarations.keys().cloned().collect(),
            imported_types: self.imported_types(&index, path),
            builtin_types: self.metadata.types.keys().cloned().collect(),
            known_annotations: self.metadata.annotations.keys().cloned().collect(),
            type_hierarchy: hierarchy,
            annotations: self.metadata.annotations.clone(),
        };
        drop(index);
        let import_errors = self.validate_imports(&module, path);
        let generation_allowed = !Self::has_fatal_diagnostics(&import_errors)
            && module.lexer_errors.is_empty()
            && !Self::has_fatal_diagnostics(&module.parser_errors)
            && !Self::has_fatal_diagnostics(&module.collision_errors)
            && module
                .imports
                .iter()
                .all(|p| !Self::has_fatal_diagnostics(&self.get_all_errors_for(p)));
        let analysis_only = Analyzer::with_metadata(false, self.metadata.clone())
            .expect("validated registry metadata");
        let analyzer = if generation_allowed {
            &self.analyzer
        } else {
            &analysis_only
        };
        let reachable = self.reachable_declarations();
        let ast: Vec<_> = module
            .borrow_ast()
            .iter()
            .filter(|d| {
                !self.analyzer.generation_mode
                    || match d {
                        Declaration::Type { name, .. } => reachable
                            .contains(&(path.into(), name.text(module.borrow_source()).into())),
                        Declaration::Custom { .. } => path == self.entry && d.public().is_some(),
                        Declaration::Import { .. } => true,
                    }
            })
            .cloned()
            .collect();
        let diagnostics = analyzer.run_scope_with_bundle(
            &ast,
            module.borrow_source(),
            &scope,
            &self.plugins,
            Some(&reachable),
        );
        module.analyzer_errors = diagnostics
            .into_iter()
            .map(|d| module.diagnostic(d, ErrorPhase::Analyzer))
            .collect();
        module.module_errors = import_errors;
        true
    }

    pub fn resolve_import(
        &self,
        import_array: &[&str],
        import_str: Option<&str>,
    ) -> Result<(ModulePath, PathBuf), ModuleDiagnostic> {
        let import_str = import_str
            .map(|s| s.to_string())
            .unwrap_or_else(|| import_array.join("/"));
        let mut pathbuf = self.root.join(&import_str);
        pathbuf.add_extension("xen");

        match pathbuf.canonicalize() {
            Ok(p) => Ok((import_str, p)),
            Err(e) => Err(ModuleDiagnostic {
                tags: vec![],
                module_path: import_str.clone(),
                message: format!("Cannot resolve import '{}': {}", import_str, e),
                location: None,
                phase: ErrorPhase::Module,
                severity: XenoDiagSeverity::Err,
            }),
        }
    }

    /// Validates all import declarations in a module.
    fn validate_imports(&self, module: &ModuleData, module_path: &str) -> Vec<ModuleDiagnostic> {
        let mut errors = Vec::new();
        for decl in module.borrow_ast().iter() {
            if let Declaration::Import {
                path,
                names,
                location,
                ..
            } = decl
            {
                let segments = path.to_vec();
                match self.resolve_import(
                    &segments.iter().map(String::as_str).collect::<Vec<_>>(),
                    None,
                ) {
                    Ok((target, abs_path)) => {
                        let index = self.index.read().unwrap();
                        let mut selected = HashSet::new();
                        for name in names {
                            let text = name.text(module.borrow_source());
                            let message = if !selected.insert(text) {
                                Some(format!("Duplicate import of '{text}'"))
                            } else if bindings::exported(&index, &target, text, &mut HashSet::new())
                                .is_none()
                            {
                                Some(format!(
                                    "Module '{}' does not export '{text}'",
                                    path.join("::")
                                ))
                            } else if module.declarations.contains_key(text)
                                || module
                                    .borrow_ast()
                                    .iter()
                                    .filter_map(|d| match d {
                                        Declaration::Import { names, .. } => Some(names),
                                        _ => None,
                                    })
                                    .flatten()
                                    .filter(|n| n.text(module.borrow_source()) == text)
                                    .count()
                                    > 1
                            {
                                Some(format!(
                                    "Import '{text}' conflicts with another declaration or import"
                                ))
                            } else {
                                None
                            };
                            if let Some(message) = message {
                                errors.push(module.diagnostic(
                                    crate::XenoDiagnostic {
                                        tags: vec![],
                                        location: *name,
                                        message,
                                        severity: XenoDiagSeverity::Err,
                                    },
                                    ErrorPhase::Module,
                                ));
                            }
                        }
                        if !abs_path.exists() {
                            errors.push(ModuleDiagnostic {
                                tags: vec![],
                                module_path: module_path.to_string(),
                                message: format!(
                                    "Module '{}' not found (expected at '{}')",
                                    path.join("/"),
                                    abs_path.display()
                                ),
                                location: Some((
                                    location.line(module.borrow_source()),
                                    location.column(module.borrow_source()),
                                    location.len(),
                                )),
                                phase: ErrorPhase::Analyzer,
                                severity: XenoDiagSeverity::Err,
                            });
                        }
                    }
                    Err(_) => {
                        errors.push(ModuleDiagnostic {
                            tags: vec![],
                            module_path: module_path.to_string(),
                            message: format!("Cannot resolve module '{}'", path.join("/")),
                            location: Some((
                                location.line(module.borrow_source()),
                                location.column(module.borrow_source()),
                                location.len(),
                            )),
                            phase: ErrorPhase::Analyzer,
                            severity: XenoDiagSeverity::Err,
                        });
                    }
                }
            }
        }
        errors
    }

    /// Suggest available module paths starting with the given partial path.
    /// Returns `(segment_name, abs_path, is_directory)` tuples.
    pub fn suggest_import(&self, path_so_far: &str) -> Vec<(String, PathBuf, bool)> {
        let segments: Vec<&str> = path_so_far.split("::").collect();
        let prefix = segments[..segments.len().saturating_sub(1)].join("/");
        let last_segment = *segments.last().unwrap_or(&"");
        let dir_to_search = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.root.join(&prefix)
        };

        if let Ok(entries) = fs::read_dir(dir_to_search) {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let path = e.path();
                    if path.is_dir() {
                        let name = path.file_name()?.to_str()?;
                        if name.starts_with(last_segment) {
                            return Some((name.to_string(), path, true));
                        }
                    } else if path.extension().and_then(|e| e.to_str()) == Some("xen") {
                        let stem = path.file_stem()?.to_str()?;
                        if stem.starts_with(last_segment) {
                            return Some((stem.to_string(), path, false));
                        }
                    }
                    None
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    // ── Cached data access ──────────────────────────────────────────

    /// Removes modules outside the caller's active graph from all indexes.
    pub fn retain_modules(&self, paths: &HashSet<ModulePath>) {
        let removed = {
            let _update = self.coordinator.lock().unwrap();
            let mut modules = self.module_cache.write().unwrap();
            let removed: Vec<_> = modules
                .keys()
                .filter(|path| !paths.contains(*path))
                .cloned()
                .collect();
            modules.retain(|path, _| paths.contains(path));
            self.index
                .write()
                .unwrap()
                .retain(|path, _| paths.contains(path));
            let mut hierarchy = self.hierarchy.write().unwrap();
            for path in &removed {
                hierarchy.remove_module(path);
            }
            !removed.is_empty()
        };
        if removed {
            self.refresh_name_collision_diagnostics();
            let _update = self.coordinator.lock().unwrap();
            self.refresh_usage_diagnostics();
        }
    }

    /// Purges every loaded module and returns the number removed.
    pub fn purge_module_cache(&self) -> usize {
        let _update = self.coordinator.lock().unwrap();
        let mut modules = self.module_cache.write().unwrap();
        let len = modules.len();
        modules.clear();
        self.index.write().unwrap().clear();
        let mut hierarchy = crate::semantic::TypeHierarchy::default();
        hierarchy
            .insert_metadata(&self.metadata)
            .expect("validated registry metadata");
        *self.hierarchy.write().unwrap() = hierarchy;
        len
    }
    pub fn with_module<T>(
        &self,
        path: &str,
        f: impl for<'a> FnOnce(&'a [Token], &'a [Declaration], &'a ModuleData) -> T,
    ) -> Option<T> {
        let handle = self.module_cache.read().unwrap().get(path).cloned()?;
        let module = handle.read().unwrap();
        Some(f(module.borrow_tokens(), module.borrow_ast(), &module))
    }
    pub fn get_all_errors_for(&self, path: &str) -> Vec<ModuleDiagnostic> {
        self.with_module(path, |_, _, m| {
            m.lexer_errors
                .iter()
                .chain(&m.parser_errors)
                .chain(&m.analyzer_errors)
                .chain(&m.collision_errors)
                .chain(&m.module_errors)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    }
    fn get_all_cached_errors(&self) -> Vec<ModuleDiagnostic> {
        let mut paths: Vec<_> = self.module_cache.read().unwrap().keys().cloned().collect();
        paths.sort();
        paths
            .into_iter()
            .flat_map(|p| self.get_all_errors_for(&p))
            .collect()
    }
    pub fn get_errors_by_phase(&self, path: &str, phase: ErrorPhase) -> Vec<ModuleDiagnostic> {
        self.get_all_errors_for(path)
            .into_iter()
            .filter(|d| d.phase == phase)
            .collect()
    }
    pub fn find_declaration(&self, current_module: &str, name: &str) -> Option<DeclarationInfo> {
        let index = self.index.read().unwrap();
        bindings::in_scope(&index, current_module)
            .into_iter()
            .find(|d| d.name == name)
    }
    pub fn get_all_declarations_in_scope(&self, current_module: &str) -> Vec<DeclarationInfo> {
        let index = self.index.read().unwrap();
        bindings::in_scope(&index, current_module)
    }

    pub fn reachable_declarations(&self) -> HashSet<(String, String)> {
        bindings::reachable(&self.index.read().unwrap(), &self.entry)
    }

    pub fn module_graph(&self) -> HashSet<ModulePath> {
        let index = self.index.read().unwrap();
        let mut graph = HashSet::new();
        let mut pending = vec![self.entry.clone()];
        while let Some(path) = pending.pop() {
            if graph.insert(path.clone()) {
                if let Some(module) = index.get(&path) {
                    pending.extend(module.imports.iter().cloned());
                }
            }
        }
        graph
    }

    pub fn get_exported_declarations(&self, path: &str) -> Vec<DeclarationInfo> {
        let mut index = self.index.read().unwrap().clone();
        let mut pending = vec![path.to_string()];
        let mut seen = HashSet::new();
        while let Some(path) = pending.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            if !index.contains_key(&path) {
                let Ok((_, abs)) = self.resolve_import(&[], Some(&path)) else {
                    continue;
                };
                let Ok(source) = fs::read_to_string(&abs) else {
                    continue;
                };
                let Ok(module) =
                    Self::_create_module_data(&path, abs, source, 0, &self.config.debug)
                else {
                    continue;
                };
                index.insert(path.clone(), ModuleSummary::from_module(&module));
            }
            if let Some(module) = index.get(&path) {
                pending.extend(
                    module
                        .bindings
                        .iter()
                        .filter(|b| b.public)
                        .map(|b| b.module.clone()),
                );
            }
        }
        let Some(module) = index.get(path) else {
            return vec![];
        };
        let names: HashSet<_> = module
            .declarations
            .keys()
            .chain(
                module
                    .bindings
                    .iter()
                    .filter(|b| b.public)
                    .flat_map(|b| b.names.iter()),
            )
            .collect();
        names
            .into_iter()
            .filter_map(|name| bindings::exported(&index, path, name, &mut HashSet::new()))
            .collect()
    }

    fn imported_types(
        &self,
        index: &HashMap<String, ModuleSummary>,
        path: &str,
    ) -> HashMap<String, Vec<String>> {
        let mut imported: HashMap<String, Vec<String>> = HashMap::new();
        for declaration in bindings::in_scope(index, path)
            .into_iter()
            .filter(|d| d.module_path != path)
        {
            let names = imported.entry(declaration.module_path).or_default();
            if !names.contains(&declaration.name) {
                names.push(declaration.name);
            }
        }
        imported
    }

    fn refresh_bindings(&self) {
        let index = self.index.read().unwrap();
        let mut hierarchy = self.hierarchy.write().unwrap();
        for path in index.keys() {
            hierarchy.register_bindings(path.clone(), self.imported_types(&index, path));
        }
    }

    fn _create_module_data(
        module_path: &ModulePath,
        abs_path: PathBuf,
        source: String,
        hash: u64,
        _debug: &crate::config::DebugConfig,
    ) -> Result<ModuleData, Vec<ModuleDiagnostic>> {
        let lexed = Lexer::lex(&source);
        let parsed = if lexed.error.is_none() {
            Parser::parse_incremental(&lexed.tokens, &source, None)
        } else {
            Parsed::default()
        };
        let mut module = ModuleData {
            abs_path,
            module_path: module_path.clone(),
            source: crate::SourceDocument::new(source),
            hash,
            lexed,
            parsed,
            custom_diagnostics: vec![],
            lexer_errors: vec![],
            parser_errors: vec![],
            analyzer_errors: vec![],
            collision_errors: vec![],
            module_errors: vec![],
            imports: vec![],
            changed: true,
            declarations: HashMap::new(),
        };
        module.index();
        Ok(module)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        path::PathBuf,
        sync::Arc,
    };

    use super::{ErrorPhase, ModuleData, ModuleDiagnostic};
    use crate::{
        config::{Config, PluginConfigs},
        module::XenoRegistry,
        semantic::{Analyzer, MetadataGraph, NameCollisionValidator, BUILTIN_TYPES},
        utils::calculate_hash,
        XenoDiagSeverity,
    };

    fn summaries(cache: &HashMap<String, ModuleData>) -> HashMap<String, super::ModuleSummary> {
        cache
            .iter()
            .map(|(p, m)| {
                (
                    p.clone(),
                    super::ModuleSummary {
                        bindings: super::bindings::import_bindings(
                            m.borrow_ast(),
                            m.borrow_source(),
                        ),
                        dependencies: HashMap::new(),
                        abs_path: m.abs_path.clone(),
                        imports: m.imports.clone(),
                        declarations: m.declarations.clone(),
                        references: HashSet::new(),
                    },
                )
            })
            .collect()
    }
    fn diagnostic(severity: XenoDiagSeverity) -> ModuleDiagnostic {
        ModuleDiagnostic {
            tags: vec![],
            module_path: "test".to_string(),
            message: "diagnostic".to_string(),
            location: None,
            phase: ErrorPhase::Parser,
            severity,
        }
    }

    #[test]
    fn warning_and_info_diagnostics_are_not_fatal() {
        let diagnostics = vec![
            diagnostic(XenoDiagSeverity::Warn),
            diagnostic(XenoDiagSeverity::Info),
        ];

        assert!(!XenoRegistry::has_fatal_diagnostics(&diagnostics));
    }

    #[test]
    fn error_diagnostics_are_fatal() {
        let diagnostics = vec![diagnostic(XenoDiagSeverity::Err)];

        assert!(XenoRegistry::has_fatal_diagnostics(&diagnostics));
    }

    fn parsed_module(module_path: &str, source: &str) -> ModuleData {
        XenoRegistry::_create_module_data(
            &module_path.to_string(),
            PathBuf::from(format!("{module_path}.xen")),
            source.to_string(),
            calculate_hash(&source),
            &crate::config::DebugConfig::default(),
        )
        .expect("semantic test source should parse")
    }

    #[test]
    fn transitive_importers_are_deduplicated_across_cycles_and_diamonds() {
        let mut cache = HashMap::new();
        cache.insert(
            "leaf".to_string(),
            parsed_module("leaf", "pub type Leaf = string;"),
        );
        cache.insert(
            "a".to_string(),
            parsed_module(
                "a",
                "import leaf::Leaf; import top::Top; pub type A = Leaf;",
            ),
        );
        cache.insert(
            "b".to_string(),
            parsed_module("b", "import leaf::Leaf; pub type B = Leaf;"),
        );
        cache.insert(
            "top".to_string(),
            parsed_module("top", "import a::A; import b::B; pub type Top = A | B;"),
        );

        assert_eq!(
            XenoRegistry::transitive_importers(&summaries(&cache), "leaf"),
            vec!["a".to_string(), "b".to_string(), "top".to_string()]
        );
    }

    #[test]
    fn purging_module_cache_removes_all_modules() {
        let empty_plugins = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "test".to_string(),
            parsed_module("test", "pub type Test = string;"),
        );
        let registry = XenoRegistry {
            index: std::sync::RwLock::new(summaries(&cache)),
            hierarchy: std::sync::RwLock::new(crate::semantic::TypeHierarchy::default()),
            coordinator: std::sync::Mutex::new(()),
            module_cache: std::sync::RwLock::new(
                cache
                    .into_iter()
                    .map(|(p, m)| (p, Arc::new(std::sync::RwLock::new(m))))
                    .collect(),
            ),
            root: PathBuf::new(),
            entry: "test".to_string(),
            analyzer: Analyzer::new(false, &empty_plugins).unwrap(),
            plugins: empty_plugins,
            custom_matchers: HashMap::new(),
            config: Arc::new(Config::default()),
            metadata: MetadataGraph::default(),
        };

        assert_eq!(registry.purge_module_cache(), 1);
        assert!(registry.module_cache.read().unwrap().is_empty());
    }

    fn analyze(cache: &HashMap<String, ModuleData>, module_path: &str) -> Vec<String> {
        let module = cache.get(module_path).expect("module should be cached");
        let imports = module.borrow_imports().to_vec();
        let mut errors = Analyzer::new(false, &[])
            .unwrap()
            .run(
                module.borrow_ast(),
                module,
                &imports,
                cache,
                &[],
                &PluginConfigs::new(),
            )
            .into_iter()
            .collect::<Vec<_>>();
        let mut type_owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (owner, cached_module) in cache {
            for declaration in cached_module.borrow_declarations().values() {
                type_owners
                    .entry(declaration.name.clone())
                    .or_default()
                    .insert(owner.clone());
            }
        }
        let static_types: HashSet<_> = BUILTIN_TYPES
            .iter()
            .map(|semantic_type| semantic_type.name.to_string())
            .collect();
        errors.extend(
            NameCollisionValidator::new(
                module_path,
                module.borrow_source(),
                &type_owners,
                &static_types,
            )
            .validate(module.borrow_ast()),
        );
        errors
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err)
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    #[test]
    fn semantic_pass_allows_local_forward_references() {
        let source = "pub type Before = Later; pub type Later = string;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn semantic_pass_exposes_never_as_a_source_type() {
        let source = "pub type Impossible = NEVER; pub type StillImpossible = Impossible;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn semantic_pass_rejects_disjoint_intersections_as_never() {
        let source = "pub type Impossible = &u8&string;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert_eq!(
            analyze(&cache, "test"),
            vec![
                "Types 'u8' and 'string' cannot be intersected because their intersection is NEVER."
            ]
        );
    }

    #[test]
    fn intersection_constraints_are_inherited_from_either_member() {
        let source = "pub type Text = &uuid&any @match(/^[a-f]+$/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn sum_constraints_must_be_guaranteed_by_every_member() {
        let source = "pub type Text = |uuid|string @match(/^[a-f]+$/); pub type Mixed = |uuid|u8 @match(/^[a-f]+$/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert_eq!(
            analyze(&cache, "test"),
            vec![
                "Annotation '@match' is not applicable to type 'uuid | u8'. Required type(s): string."
            ]
        );
    }

    #[test]
    fn standalone_optional_types_keep_present_value_constraints() {
        let source = "pub type MaybeCount = ?u8 @min(0);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn intersection_validation_resolves_imported_aliases() {
        let mut cache = HashMap::new();
        cache.insert(
            "base".to_string(),
            parsed_module("base", "pub type Text = string;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module("test", "import base::Text; pub type Impossible = &Text&u8;"),
        );

        assert_eq!(
            analyze(&cache, "test"),
            vec![
                "Types 'Text' and 'u8' cannot be intersected because their intersection is NEVER."
            ]
        );
    }

    #[test]
    fn intersection_validation_uses_generic_type_constraints() {
        let source =
            "pub type Impossible<T: string> = &T&u8; pub type Possible<T: number> = &T&u8;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert_eq!(
            analyze(&cache, "test"),
            vec!["Types 'T' and 'u8' cannot be intersected because their intersection is NEVER."]
        );
    }

    #[test]
    fn unknown_annotations_warn_while_unknown_types_error() {
        let mut cache = HashMap::new();
        cache.insert(
            "test".to_string(),
            parsed_module("test", "pub type Test = Missing @Lombok;"),
        );
        let module = cache.get("test").expect("module should be cached");

        let diagnostics = Analyzer::new(false, &[]).unwrap().run(
            module.borrow_ast(),
            module,
            module.borrow_imports(),
            &cache,
            &[],
            &PluginConfigs::new(),
        );

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "Unknown annotation '@Lombok'"
                && diagnostic.severity == XenoDiagSeverity::Warn
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "Unknown type 'Missing'"
                && diagnostic.severity == XenoDiagSeverity::Err
        }));
    }

    #[test]
    fn semantic_pass_rejects_duplicate_type_names_in_one_module() {
        let source = "pub type Duplicate = string; pub type Duplicate = u8;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec!["Duplicate type name 'Duplicate' in this module"]
        );
    }

    #[test]
    fn semantic_pass_rejects_duplicate_type_names_across_cached_modules() {
        let mut cache = HashMap::new();
        cache.insert(
            "first".to_string(),
            parsed_module("first", "pub type Shared = string;"),
        );
        cache.insert(
            "second".to_string(),
            parsed_module("second", "pub type Shared = u8;"),
        );

        let errors = analyze(&cache, "second");
        assert_eq!(
            errors,
            vec!["Duplicate type name 'Shared' (also declared in module 'first')"]
        );
    }

    #[test]
    fn semantic_pass_rejects_duplicate_struct_fields_and_enum_variants() {
        let source =
            "pub type User = { id: string, id: u8 }; pub type State = enum { ready: string, ready: u8 };";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec![
                "Duplicate struct field 'id'",
                "Duplicate enum variant 'ready'",
            ]
        );
    }

    #[test]
    fn semantic_pass_allows_member_names_in_separate_declarations() {
        let source = "pub type First = { id: string }; pub type Second = { id: u8 }; pub type Left = enum { ready: string }; pub type Right = enum { ready: u8 };";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn semantic_pass_rejects_duplicate_and_shadowing_generic_parameters() {
        let source = "pub type Existing = string; pub type Pair<T, T> = T; pub type UserGeneric<Existing> = Existing; pub type BuiltinGeneric<string> = string;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec![
                "Duplicate generic parameter 'T'",
                "Generic parameter 'Existing' shadows an existing type",
                "Generic parameter 'string' shadows an existing type",
            ]
        );
    }

    #[test]
    fn semantic_pass_rejects_generics_that_shadow_types_from_other_cached_modules() {
        let mut cache = HashMap::new();
        cache.insert(
            "shared".to_string(),
            parsed_module("shared", "pub type Existing = string;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module("test", "pub type UserGeneric<Existing> = Existing;"),
        );

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec!["Generic parameter 'Existing' shadows an existing type"]
        );
    }

    #[test]
    fn semantic_pass_allows_generic_names_to_repeat_in_separate_declarations() {
        let source = "pub type First<T> = T; pub type Second<T> = T;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn full_cache_revalidation_finds_types_added_after_generic_declarations() {
        let empty_plugins = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "early".to_string(),
            parsed_module("early", "pub type Wrapper<Late> = Late;"),
        );
        cache.insert(
            "late".to_string(),
            parsed_module("late", "pub type Late = string;"),
        );
        let registry = XenoRegistry {
            index: std::sync::RwLock::new(summaries(&cache)),
            hierarchy: std::sync::RwLock::new(crate::semantic::TypeHierarchy::default()),
            coordinator: std::sync::Mutex::new(()),
            module_cache: std::sync::RwLock::new(
                cache
                    .into_iter()
                    .map(|(p, m)| (p, Arc::new(std::sync::RwLock::new(m))))
                    .collect(),
            ),
            root: PathBuf::new(),
            entry: "early".to_string(),
            analyzer: Analyzer::new(false, &empty_plugins).unwrap(),
            plugins: empty_plugins,
            custom_matchers: HashMap::new(),
            config: Arc::new(Config::default()),
            metadata: MetadataGraph::default(),
        };

        registry.refresh_name_collision_diagnostics();

        let errors = registry.get_all_errors_for("early");
        assert!(errors.iter().any(|diagnostic| {
            diagnostic.message == "Generic parameter 'Late' shadows an existing type"
        }));
    }

    #[test]
    fn collision_refresh_reports_duplicate_types_on_both_declarations() {
        let empty_plugins = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "first".to_string(),
            parsed_module("first", "pub type Shared = string;"),
        );
        cache.insert(
            "second".to_string(),
            parsed_module("second", "pub type Shared = u8;"),
        );
        let registry = XenoRegistry {
            index: std::sync::RwLock::new(summaries(&cache)),
            hierarchy: std::sync::RwLock::new(crate::semantic::TypeHierarchy::default()),
            coordinator: std::sync::Mutex::new(()),
            module_cache: std::sync::RwLock::new(
                cache
                    .into_iter()
                    .map(|(p, m)| (p, Arc::new(std::sync::RwLock::new(m))))
                    .collect(),
            ),
            root: PathBuf::new(),
            entry: "first".to_string(),
            analyzer: Analyzer::new(false, &empty_plugins).unwrap(),
            plugins: empty_plugins,
            custom_matchers: HashMap::new(),
            config: Arc::new(Config::default()),
            metadata: MetadataGraph::default(),
        };

        registry.refresh_name_collision_diagnostics();

        for module_path in ["first", "second"] {
            let errors = registry.get_all_errors_for(module_path);
            assert!(errors.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Duplicate type name 'Shared'")
            }));
        }
    }

    #[test]
    fn dependency_order_places_imports_before_importers() {
        let mut cache = HashMap::new();
        cache.insert(
            "leaf".to_string(),
            parsed_module("leaf", "pub type Leaf = string;"),
        );
        cache.insert(
            "middle".to_string(),
            parsed_module("middle", "import leaf::Leaf; pub type Middle = Leaf;"),
        );
        cache.insert(
            "entry".to_string(),
            parsed_module("entry", "import middle::Middle; pub type Entry = Middle;"),
        );

        assert_eq!(
            XenoRegistry::dependency_order(&summaries(&cache), "entry"),
            vec!["leaf", "middle", "entry"]
        );
    }

    #[test]
    fn semantic_pass_rejects_type_names_that_shadow_static_types() {
        let mut cache = HashMap::new();
        cache.insert(
            "test".to_string(),
            parsed_module("test", "type string = u8;"),
        );

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec!["Type name 'string' conflicts with a built-in or plugin type"]
        );
    }

    #[test]
    fn semantic_pass_embeds_imported_types_in_trait_hierarchy() {
        let mut cache = HashMap::new();
        cache.insert(
            "base".to_string(),
            parsed_module("base", "pub type Imported = string;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import base::Imported; pub type UsesImported = Imported @minlen(1);",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn generic_constraints_accept_trait_names_and_follow_parentage() {
        let source = "pub type Before = Box<u8>; pub type Box<T: NumberLiteral> = T; pub type Identity<T> = T; pub type Forward<T: NumberLiteral> = Box<T>; pub type Nested<T: NumberLiteral> = Box<Identity<T>>; pub type Float = Box<f32>; pub type Bad = Box<string>;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 1, "unexpected errors: {errors:#?}");
        assert!(errors[0].contains("does not satisfy constraint 'NumberLiteral'"));
        assert!(errors[0].contains("string"));
    }

    #[test]
    fn dictionary_keys_must_implement_key_trait() {
        let source = "pub type TextKey = Dict<string, bool>; pub type NumberKey = Dict<u8, string>; pub type Alias = string; pub type AliasKey = Dict<Alias, u8>; pub type Bad = Dict<bool, string>;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 1, "unexpected errors: {errors:#?}");
        assert!(errors[0].contains("bool"));
        assert!(errors[0].contains("Dict.K"));
        assert!(errors[0].contains("constraint 'KeyTrait'"));
    }

    #[test]
    fn hierarchy_preserves_duplicate_names_from_loaded_modules() {
        let mut cache = HashMap::new();
        cache.insert(
            "text".to_string(),
            parsed_module("text", "pub type Shared = string;"),
        );
        cache.insert(
            "numeric".to_string(),
            parsed_module("numeric", "pub type Shared = u8;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import text::Shared; pub type Local = Shared @minlen(1);",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn imported_types_resolve_parents_through_their_own_imports() {
        let mut cache = HashMap::new();
        cache.insert(
            "base".to_string(),
            parsed_module("base", "pub type Text = string;"),
        );
        cache.insert(
            "middle".to_string(),
            parsed_module("middle", "import base::Text; pub type Imported = Text;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import middle::Imported; pub type UsesImported = Imported @minlen(1);",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn imported_generics_resolve_arguments_in_the_calling_module() {
        let mut cache = HashMap::new();
        cache.insert(
            "containers".to_string(),
            parsed_module("containers", "pub type Box<T: NumberLiteral> = T;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import containers::Box; pub type Local = u8; pub type Good = Box<Local>;",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn deprecated_is_metadata_on_any_named_type_and_does_not_warn_on_use() {
        let source = concat!(
            "pub type OldText = string @deprecated; ",
            "pub type OldRecord = { value: OldText } @deprecated(); ",
            "pub type OldChoice = enum { Yes: 1, No: 0 } @deprecated; ",
            "pub type OldTuple = [OldText, ?bool] @deprecated; ",
            "pub type OldSet = set [1, 2] @deprecated; ",
            "pub type OldUnion = |string|u32 @deprecated; ",
            "pub type OldGeneric<T> = T @deprecated; ",
            "pub type OldNever = NEVER @deprecated; ",
            "pub type OldReason = string @deprecated(\"Use Current instead.\"); ",
            "pub type OldEmptyReason = string @deprecated(\"\"); ",
            "pub type Current = { record: OldRecord, value: OldGeneric<OldText> };",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));
        let module = &cache["test"];
        assert!(module.borrow_parser_errors().is_empty());
        let diagnostics = Analyzer::new(false, &[]).unwrap().run(
            module.borrow_ast(),
            module,
            &[],
            &cache,
            &[],
            &PluginConfigs::new(),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn deprecated_rejects_non_string_reasons_and_extra_arguments() {
        let source = concat!(
            "pub type Old = string @deprecated(1); ",
            "pub type Other = string @deprecated(true); ",
            "pub type Reference = string @deprecated(string); ",
            "pub type Excess = string @deprecated(\"first\", \"second\");",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert_eq!(
            errors
                .iter()
                .filter(|error| error.contains("expects StringLiteral"))
                .count(),
            3
        );
        assert!(errors
            .iter()
            .any(|error| error
                .contains("Annotation '@deprecated' expects 0 to 1 argument(s), got 2")));
    }

    #[test]
    fn multipleof_accepts_numeric_declarations_aliases_and_decimal_increments() {
        let source = concat!(
            "pub type Milliseconds = u32 @multipleof(10); ",
            "pub type Step<T: Numeric> = T @multipleof(0.1); ",
            "pub type Seconds = ?f64 @multipleof(0.1); ",
            "pub type Fraction = f32 @multipleof(0.125 as f32); ",
            "pub type Precise = decimal @multipleof(0.000000000000000000000000000001); ",
            "pub type Large = bigint @multipleof(184467440737095516160); ",
            "pub type Signed = i32 @multipleof(10 as i32); ",
            "pub type Timeline = { milliseconds: Milliseconds, seconds: Seconds };",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));
        let module = &cache["test"];
        assert!(module.borrow_parser_errors().is_empty());
        let diagnostics = Analyzer::new(false, &[]).unwrap().run(
            module.borrow_ast(),
            module,
            &[],
            &cache,
            &[],
            &PluginConfigs::new(),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn multipleof_rejects_non_numeric_targets() {
        let source = concat!(
            "pub type Text = string @multipleof(10); ",
            "pub type Flag = bool @multipleof(10); ",
            "pub type List = number[] @multipleof(10); ",
            "pub type Mixed = |number|string @multipleof(10);",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert!(errors.iter().all(|error| {
            error.contains("Annotation '@multipleof' is not applicable")
                && error.contains("Required trait(s): Numeric")
        }));
    }

    #[test]
    fn multipleof_requires_exactly_one_numeric_literal() {
        let source = concat!(
            "pub type Missing = number @multipleof(); ",
            "pub type Multiple = number @multipleof(1, 2); ",
            "pub type Text = number @multipleof(\"0.1\"); ",
            "pub type Flag = number @multipleof(true); ",
            "pub type Reference = number @multipleof(number);",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 5, "{errors:#?}");
        assert!(errors
            .iter()
            .any(|error| error.contains("expects 1 argument(s), got 0")));
        assert!(errors
            .iter()
            .any(|error| error.contains("expects 1 argument(s), got 2")));
        assert_eq!(
            errors
                .iter()
                .filter(|error| error.contains("expects NumberLiteral"))
                .count(),
            3,
            "{errors:#?}"
        );
    }

    #[test]
    fn multipleof_rejects_zero_and_negative_increments() {
        let source = concat!(
            "pub type Zero = number @multipleof(0); ",
            "pub type DecimalZero = decimal @multipleof(0.0); ",
            "pub type Negative = integer @multipleof(-10); ",
            "pub type DecimalNegative = f64 @multipleof(-0.1); ",
            "pub type Conditional = number @if(true, @multipleof(0));",
        );
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 5, "{errors:#?}");
        assert!(errors
            .iter()
            .all(|error| error
                == "Annotation '@multipleof' argument 'value' must be greater than zero."));
    }

    #[test]
    fn match_accepts_string_and_string_descendants() {
        let source = "pub type Email = string @match(/.*@.*\\.com/) @minlen(10); pub type BuiltinDescendant = uuid @match(/^[0-9a-f-]+$/); pub type UserDescendant = Email @match(/^alias$/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn match_rejects_types_that_do_not_descend_from_string() {
        let source = "pub type Number = u8 @match(/^[0-9]+$/); pub type Bytes = binary @match(/.*/); pub type Date = date @match(/^2026-/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 3, "unexpected errors: {errors:#?}");
        assert!(errors
            .iter()
            .all(|error| error.contains("Annotation '@match' is not applicable")));
        assert!(errors
            .iter()
            .all(|error| error.contains("Required type(s): string")));
    }

    #[test]
    fn match_requires_exactly_one_regex_literal() {
        let source = "pub type Missing = string @match(); pub type StringArg = string @match(\"pattern\"); pub type Multiple = string @match(/a/, /b/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 3, "unexpected errors: {errors:#?}");
        assert!(errors
            .iter()
            .any(|error| error.contains("expects 1 argument(s), got 0")));
        assert!(errors
            .iter()
            .any(|error| error.contains("expects RegexLiteral, got string literal")));
        assert!(errors
            .iter()
            .any(|error| error.contains("expects 1 argument(s), got 2")));
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    fn registry() -> XenoRegistry {
        let root = std::env::temp_dir().join(format!(
            "xeno-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("index.xen"), "pub type Root = string;").unwrap();
        let mut config = Config::default_with_workdir(root);
        config.parser.entry = "index".into();
        XenoRegistry::with_config(config, false).unwrap()
    }
    #[test]
    fn cycles_defer_scans_and_recover_without_blocking_recursive_types() {
        let registry = registry();
        fs::write(registry.root.join("a.xen"), "import b::B; pub type A = B;").unwrap();
        fs::write(
            registry.root.join("b.xen"),
            "import a::A; pub type B = number;",
        )
        .unwrap();
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "import a::A; pub type Root = A;".into(),
        );
        for path in ["a", "b", "index"] {
            assert!(registry
                .get_all_errors_for(path)
                .iter()
                .any(|d| d.message.contains("cycle")));
        }
        let errors = registry.load_module_from_source(
            &registry.root.join("b.xen"),
            "pub type B = number; pub type TreeNode = { children: Tree[] }; pub type Tree = |TreeNode|number;"
                .into(),
        );
        assert!(
            !errors.iter().any(|d| d.severity == XenoDiagSeverity::Err),
            "{errors:?}"
        );
        fs::remove_dir_all(&registry.root).unwrap();
    }
    #[test]
    fn invalid_source_clears_old_indexes_and_updates_dependents() {
        let registry = registry();
        fs::write(registry.root.join("a.xen"), "pub type A = number;").unwrap();
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "import a::A; pub type Root = A;".into(),
        );
        let handle = registry.module_cache.read().unwrap()["a"].clone();
        registry.load_module_from_source(&registry.root.join("a.xen"), "\u{e9}".into());
        assert!(registry.find_declaration("index", "A").is_none());
        assert!(registry
            .get_all_errors_for("index")
            .iter()
            .any(|d| d.message.contains("Unknown type 'A'")));
        assert_eq!(handle.read().unwrap().borrow_source(), "\u{e9}");
        registry
            .load_module_from_source(&registry.root.join("a.xen"), "pub type A = string;".into());
        assert!(!registry
            .get_all_errors_for("index")
            .iter()
            .any(|d| d.severity == XenoDiagSeverity::Err));
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn declaration_metadata_is_reused_and_unresolved_references_are_invalidated() {
        let registry = registry();
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "pub type First = string; pub type Second = number;".into(),
        );
        let first = registry
            .find_declaration("index", "First")
            .unwrap()
            .semantic;
        let second = registry
            .find_declaration("index", "Second")
            .unwrap()
            .semantic;
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "\n\npub type First = string; pub type Second = bool;".into(),
        );
        let updated = registry.find_declaration("index", "First").unwrap();
        assert!(Arc::ptr_eq(&first, &updated.semantic));
        assert_eq!(updated.line, 2);
        assert!(!Arc::ptr_eq(
            &second,
            &registry
                .find_declaration("index", "Second")
                .unwrap()
                .semantic
        ));

        fs::write(
            registry.root.join("consumer.xen"),
            "import index::Missing; pub type Consumer = Missing;",
        )
        .unwrap();
        registry.load_module(&["consumer"], true, None);
        assert!(registry
            .get_all_errors_for("consumer")
            .iter()
            .any(|d| d.message.contains("Unknown type 'Missing'")));
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "pub type Missing = string;".into(),
        );
        assert!(!registry
            .get_all_errors_for("consumer")
            .iter()
            .any(|d| d.message.contains("Unknown type 'Missing'")));
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn self_import_is_deferred_and_an_unchanged_document_preserves_metadata() {
        let registry = registry();
        let path = registry.root.join("index.xen");
        registry
            .load_module_from_source(&path, "import index::First; pub type Root = string;".into());
        assert!(registry
            .get_all_errors_for("index")
            .iter()
            .any(|d| d.message.contains("cycle")));
        registry.load_module_from_source(&path, "pub type Root = string;".into());
        let root = registry.find_declaration("index", "Root").unwrap().semantic;
        registry.load_module_from_source(&path, "pub type Root = string;".into());
        assert!(Arc::ptr_eq(
            &root,
            &registry.find_declaration("index", "Root").unwrap().semantic
        ));
        assert!(registry.get_all_errors_for("index").is_empty());
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn removing_duplicate_declarations_rebuilds_the_surviving_semantic_type() {
        let registry = registry();
        let path = registry.root.join("index.xen");
        registry.load_module_from_source(
            &path,
            "pub type Duplicate = string; pub type Duplicate = number;".into(),
        );
        registry.load_module_from_source(&path, "pub type Duplicate = string;".into());
        let declaration = registry.find_declaration("index", "Duplicate").unwrap();
        assert_eq!(
            declaration.semantic.body,
            crate::semantic::OwnedType::named("string")
        );
        assert!(registry.get_all_errors_for("index").is_empty());
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn selected_imports_only_expose_public_names_and_reexports_resolve_to_the_owner() {
        let registry = registry();
        fs::create_dir(registry.root.join("models")).unwrap();
        fs::write(
            registry.root.join("models/api.xen"),
            "type Helper = string; pub type Api = Helper; pub type Unselected = number;",
        )
        .unwrap();
        fs::write(
            registry.root.join("barrel.xen"),
            "pub import models::api::{Api}; type Private = string;",
        )
        .unwrap();
        let errors = registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "pub import barrel::Api; type Unused = string;".into(),
        );
        assert!(
            !errors.iter().any(|d| d.severity == XenoDiagSeverity::Err),
            "{errors:?}"
        );
        assert_eq!(
            registry
                .find_declaration("index", "Api")
                .unwrap()
                .module_path,
            "models/api"
        );
        for name in ["Helper", "Unselected", "Private"] {
            assert!(registry.find_declaration("index", name).is_none());
        }
        assert_eq!(
            registry.reachable_declarations(),
            HashSet::from([
                ("models/api".into(), "Api".into()),
                ("models/api".into(), "Helper".into())
            ])
        );
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn core_usage_diagnostics_follow_entry_export_changes_without_rescanning_dependencies() {
        let registry = registry();
        fs::write(
            registry.root.join("other.xen"),
            "type Helper = string; pub type Api = Helper; pub type Spare = string;",
        )
        .unwrap();
        let path = registry.root.join("index.xen");
        registry
            .load_module_from_source(&path, "pub import other::Api; type Unused = string;".into());
        let api = registry.find_declaration("other", "Api").unwrap().semantic;
        let unused_types = || {
            registry
                .get_all_errors_for("other")
                .into_iter()
                .filter(|d| d.tags.contains(&crate::XenoDiagnosticTag::Unnecessary))
                .collect::<Vec<_>>()
        };
        let diagnostics = unused_types();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].phase, ErrorPhase::Analyzer);
        assert_eq!(diagnostics[0].severity, XenoDiagSeverity::Info);
        assert_eq!(diagnostics[0].location, Some((0, 45, 23)));

        registry.load_module_from_source(&path, "import other::Api; type Unused = string;".into());
        assert_eq!(unused_types().len(), 3);
        let errors = registry.get_all_errors_for("index");
        let import = errors
            .iter()
            .find(|d| d.message == "Unused import 'Api'.")
            .unwrap();
        assert_eq!(import.phase, ErrorPhase::Analyzer);
        assert_eq!(import.severity, XenoDiagSeverity::Warn);
        assert_eq!(import.tags, [crate::XenoDiagnosticTag::Unnecessary]);
        assert_eq!(import.location, Some((0, 14, 3)));
        assert!(Arc::ptr_eq(
            &api,
            &registry.find_declaration("other", "Api").unwrap().semantic
        ));

        registry.load_module_from_source(&path, "import other::Api; pub type Root = Api;".into());
        assert_eq!(unused_types().len(), 1);
        assert!(registry.get_all_errors_for("index").is_empty());
        registry.load_module_from_source(&path, "pub import other::Api;".into());
        assert_eq!(unused_types().len(), 1);
        assert!(registry.get_all_errors_for("index").is_empty());
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn entry_exports_keep_transitive_dependencies_but_discard_unused_imports_and_exports() {
        let registry = registry();
        fs::write(
            registry.root.join("other.xen"),
            "pub type Used = string; pub type Spare = string; pub type Unselected = string;",
        )
        .unwrap();
        let errors = registry.load_module_from_source(&registry.root.join("index.xen"), "import other::{Used, Spare}; type Helper = { Spare: Used }; pub type Root = ?Helper[]; type Dead = Spare;".into());
        assert!(
            !errors.iter().any(|d| d.severity == XenoDiagSeverity::Err),
            "{errors:?}"
        );
        assert_eq!(
            registry.reachable_declarations(),
            HashSet::from([
                ("index".into(), "Root".into()),
                ("index".into(), "Helper".into()),
                ("other".into(), "Used".into())
            ])
        );
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn reachability_follows_generic_constraints_annotations_and_recursive_types() {
        let registry = registry();
        registry.load_module_from_source(&registry.root.join("index.xen"), "type Bound = string; type Argument = string; type Annotated = string; type Recursive = { next: ?Recursive }; pub type Root<T: Bound> = [Argument, Recursive] @custom(Annotated); type Dead = number;".into());
        let names: HashSet<_> = registry
            .reachable_declarations()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(
            names,
            HashSet::from(
                ["Root", "Bound", "Argument", "Annotated", "Recursive"].map(str::to_string)
            )
        );
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn private_missing_and_ambiguous_imports_are_reported_at_the_selected_name() {
        let registry = registry();
        fs::write(
            registry.root.join("other.xen"),
            "type Private = string; pub type Public = number;",
        )
        .unwrap();
        registry.load_module_from_source(
            &registry.root.join("index.xen"),
            "import other::{Private, Missing, Public, Public}; type Public = string;".into(),
        );
        let errors = registry.get_all_errors_for("index");
        for text in [
            "does not export 'Private'",
            "does not export 'Missing'",
            "Duplicate import of 'Public'",
            "conflicts with another declaration",
        ] {
            assert!(
                errors.iter().any(|d| d.message.contains(text)),
                "{errors:?}"
            );
        }
        fs::remove_dir_all(&registry.root).unwrap();
    }

    #[test]
    fn changing_pub_and_selections_updates_reachability_and_import_scopes() {
        let registry = registry();
        let entry = registry.root.join("index.xen");
        let other = registry.root.join("other.xen");
        fs::write(&other, "pub type First = string; pub type Second = number;").unwrap();
        registry.load_module_from_source(&entry, "pub import other::First;".into());
        assert!(registry
            .reachable_declarations()
            .contains(&("other".into(), "First".into())));
        registry.load_module_from_source(&entry, "pub import other::Second;".into());
        assert!(registry.find_declaration("index", "First").is_none());
        assert_eq!(
            registry.reachable_declarations(),
            HashSet::from([("other".into(), "Second".into())])
        );
        registry.load_module_from_source(
            &other,
            "pub type First = string; type Second = number;".into(),
        );
        assert!(registry.find_declaration("index", "Second").is_none());
        assert!(registry.reachable_declarations().is_empty());
        assert!(registry
            .get_all_errors_for("index")
            .iter()
            .any(|d| d.message.contains("does not export 'Second'")));
        fs::remove_dir_all(&registry.root).unwrap();
    }
}
