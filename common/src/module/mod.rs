use ouroboros::self_referencing;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod types;

use crate::config::Config;
use crate::lexer::{Lexer, Token, XenoTokens};
use crate::module::types::{DeclarationInfo, ErrorPhase, ModuleDiagnostic, ModulePath};
use crate::parser::{Declaration, Parser, XenoAst};
use crate::plugins::XenoPlugin;
use crate::semantic::{Analyzer, NameCollisionValidator, TypeDeclarationInfo, BUILTIN_TYPES};
use crate::utils::calculate_hash;
use crate::XenoDiagSeverity;

/// Information about a single module (one .xen file).
/// Owns the source text so that all borrows from tokens/ast remain valid.
#[self_referencing]
pub struct ModuleData {
    /// The absolute filesystem path.
    pub abs_path: PathBuf,
    /// Module path relative to workspace root, using '/' separators (e.g. "a/b").
    pub module_path: ModulePath,
    /// Owned source text — tokens and AST borrow from this.
    pub source: String,
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
    #[borrows(source)]
    #[covariant]
    pub tokens: XenoTokens<'this>,
    /// AST of the module
    #[borrows(tokens)]
    #[covariant]
    pub ast: XenoAst<'this>,
    #[borrows(ast, abs_path, module_path)]
    #[covariant]
    pub declarations: HashMap<&'this str, DeclarationInfo>,
}

/// Determines the workspace root and entry module path from the config.
fn get_root() -> Result<(PathBuf, String), ModuleDiagnostic> {
    let config = Config::get();
    let mut joined = config.workdir.join(Path::new(&config.parser.entry));
    joined.add_extension("xen");
    let entry_file = joined.canonicalize().map_err(|e| ModuleDiagnostic {
        module_path: config.parser.entry.clone(),
        message: format!("Cannot resolve entry file '{:?}': {}", joined, e),
        location: None,
        phase: ErrorPhase::Module,
        severity: XenoDiagSeverity::Err,
    })?;

    let root_err = || ModuleDiagnostic {
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
    // TODO per-module RwLock
    pub module_cache: RwLock<HashMap<ModulePath, ModuleData>>,
    pub root: PathBuf,
    pub entry: String,
    pub plugins: &'static Vec<&'static XenoPlugin<'static>>,
    pub analyzer: Analyzer,
}

impl XenoRegistry {
    pub fn new(generation_mode: bool) -> Result<XenoRegistry, ModuleDiagnostic> {
        let (root, entry) = get_root()?;
        let plugins = XenoPlugin::get_plugins();
        Ok(XenoRegistry {
            module_cache: RwLock::new(HashMap::default()),
            root,
            entry,
            analyzer: Analyzer::new(generation_mode, plugins),
            plugins,
        })
    }

    /// Initializes a new `XenoRegistry` and loads the entire workspace starting from the entry module.
    pub fn load_workspace(generation_mode: bool) -> Result<XenoRegistry, Vec<ModuleDiagnostic>> {
        let reg = XenoRegistry::new(generation_mode).map_err(|e| vec![e])?;
        let errs = reg.load_module(&[&reg.entry], true, None);
        if Self::has_fatal_diagnostics(&errs) {
            return Err(errs);
        }
        Ok(reg)
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
                    module_path,
                    message: format!("Cannot canonicalize '{}': {}", abs_path.display(), e),
                    location: None,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                }]
            }
        };

        // Hash-based change detection — skip if unchanged
        let hash = calculate_hash(&source);
        {
            let cache = self.module_cache.read().unwrap();
            if let Some(existing) = cache.get(&module_path) {
                if *existing.borrow_hash() == hash {
                    return self.get_all_errors_for(&module_path);
                }
            }
        }

        self._load_module_inner(module_path, canonical, source, hash)
    }

    /// Recursively loads a .xen file from disk and all its imports.
    pub fn load_module(
        &self,
        import_segments: &[&str],
        force: bool,
        import_str: Option<&str>,
    ) -> Vec<ModuleDiagnostic> {
        let (module_path, abs_path) = match self.resolve_import(import_segments, import_str) {
            Err(e) => return vec![e],
            Ok(fp) => fp,
        };

        // Skip if already loaded unless forced
        if !force && self.module_cache.read().unwrap().contains_key(&module_path) {
            return vec![];
        }

        let source = match fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                return vec![ModuleDiagnostic {
                    module_path,
                    message: format!("Failed to read file '{}': {}", abs_path.display(), e),
                    location: None,
                    phase: ErrorPhase::Module,
                    severity: XenoDiagSeverity::Err,
                }];
            }
        };

        // Hash-based skip when forced
        let hash = calculate_hash(&source);
        if force {
            let r = self.module_cache.read().unwrap();
            if let Some(existing) = r.get(&module_path) {
                if *existing.borrow_hash() == hash {
                    return vec![];
                }
            }
        }

        let is_root_load = import_str.is_none();
        let initial_errors = self._load_module_inner(module_path, abs_path, source, hash);
        if !is_root_load {
            return initial_errors;
        }

        // Recursive loading analyzes dependencies as soon as they become
        // available. Re-run validation after the root graph is complete so
        // cache-wide name constraints do not depend on sibling load order.
        self.refresh_name_collision_diagnostics();
        let validation_errors = self.get_all_cached_errors();
        if self.analyzer.generation_mode
            && !Self::has_fatal_diagnostics(&initial_errors)
            && !Self::has_fatal_diagnostics(&validation_errors)
        {
            self.generate_all_cached_modules();
            self.refresh_name_collision_diagnostics();
        }
        let mut errors = self.get_all_cached_errors();
        for diagnostic in initial_errors {
            if !errors.iter().any(|existing| {
                existing.module_path == diagnostic.module_path
                    && existing.message == diagnostic.message
                    && existing.location == diagnostic.location
                    && existing.phase == diagnostic.phase
                    && existing.severity == diagnostic.severity
            }) {
                errors.push(diagnostic);
            }
        }
        errors
    }

    fn _load_module_inner(
        &self,
        module_path: ModulePath,
        abs_path: PathBuf,
        source: String,
        hash: u64,
    ) -> Vec<ModuleDiagnostic> {
        let mut errors: Vec<ModuleDiagnostic> = Vec::new();

        let md = match Self::_create_module_data(&module_path, abs_path, source, hash) {
            Ok(r) => r,
            Err(e) => {
                // Do not leave declarations from the previous version visible
                // when the replacement source cannot be lexed.
                self.module_cache.write().unwrap().remove(&module_path);
                errors.extend(e);
                return errors;
            }
        };

        // ── Step 1: Insert into cache immediately to break import cycles ──
        // Any recursive load_module call for this module will now find it and return early.
        let imports = md.borrow_imports().to_vec();
        {
            self.module_cache
                .write()
                .unwrap()
                .insert(module_path.clone(), md);
        }

        // ── Step 2: Load imports (cycle-safe now) ──
        for import in &imports {
            let segments: Vec<&str> = import.split('/').collect();
            errors.extend(self.load_module(&segments, false, Some(import)));
        }

        // ── Step 3: Analyze with full scope (read lock only) ──
        let (analyzer_errors, import_errors, lexer_errs, parser_errs) = {
            let cache = self.module_cache.read().unwrap();
            let md = cache.get(&module_path).unwrap();

            let import_errors = self.validate_imports(md, &module_path);
            let lexer_errs = md.borrow_lexer_errors().clone();
            let parser_errs = md.borrow_parser_errors().clone();
            let generation_allowed = !Self::has_fatal_diagnostics(&errors)
                && !Self::has_fatal_diagnostics(&import_errors)
                && !Self::has_fatal_diagnostics(&lexer_errs)
                && !Self::has_fatal_diagnostics(&parser_errs);

            // When an earlier phase failed, run validators without generator
            // listeners. Warnings and infos leave generation mode unchanged.
            let analysis_only;
            let analyzer = if generation_allowed && !self.analyzer.generation_mode {
                &self.analyzer
            } else {
                analysis_only = Analyzer::new(false, self.plugins);
                &analysis_only
            };

            let xeno_errors = analyzer.run(
                md.borrow_ast(),
                md,
                &imports,
                &cache,
                self.plugins,
                &Config::get().plugins.config,
            );

            let analyzer_errors: Vec<ModuleDiagnostic> = xeno_errors
                .iter()
                .map(|e| ModuleDiagnostic {
                    module_path: module_path.clone(),
                    message: e.message.clone(),
                    location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                    phase: ErrorPhase::Analyzer,
                    severity: e.severity,
                })
                .collect();

            (analyzer_errors, import_errors, lexer_errs, parser_errs)
        };

        // ── Step 4: Write error fields back into the cached module ──
        {
            let mut cache = self.module_cache.write().unwrap();
            let md = cache.get_mut(&module_path).unwrap();
            md.with_analyzer_errors_mut(|errs: &mut Vec<ModuleDiagnostic>| {
                *errs = analyzer_errors.clone()
            });
            md.with_module_errors_mut(|errs: &mut Vec<ModuleDiagnostic>| {
                *errs = import_errors.clone()
            });
        }

        errors.extend(lexer_errs);
        errors.extend(parser_errs);
        errors.extend(analyzer_errors);
        errors.extend(import_errors);

        errors
    }

    /// Re-runs module and semantic validation for every cached module that
    /// directly or transitively imports `changed_module`.
    ///
    /// Importers are returned in dependency order (nearest first). A visited
    /// set prevents circular imports from revalidating a module more than once.
    pub fn revalidate_importers(&self, changed_module: &str) -> Vec<ModulePath> {
        let importers = {
            let cache = self.module_cache.read().unwrap();
            Self::transitive_importers(&cache, changed_module)
        };

        importers
            .into_iter()
            .filter(|module_path| self.revalidate_cached_module(module_path))
            .collect()
    }

    /// Refreshes cache-wide collision diagnostics without rebuilding semantic
    /// type hierarchies for unaffected modules.
    pub fn refresh_name_collision_diagnostics(&self) -> Vec<ModulePath> {
        let diagnostics = {
            let cache = self.module_cache.read().unwrap();
            let mut type_owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for (module_path, module) in cache.iter() {
                for declaration in module.borrow_declarations().values() {
                    type_owners
                        .entry(declaration.name.clone())
                        .or_default()
                        .insert(module_path.clone());
                }
            }

            let mut static_types: HashSet<String> = BUILTIN_TYPES
                .iter()
                .map(|semantic_type| semantic_type.name.to_string())
                .collect();
            for plugin in self.plugins {
                if let Some(provide_types) = plugin.provide_types {
                    static_types.extend(
                        provide_types()
                            .iter()
                            .map(|semantic_type| semantic_type.name.to_string()),
                    );
                }
            }

            cache
                .iter()
                .map(|(module_path, module)| {
                    let collisions =
                        NameCollisionValidator::new(module_path, &type_owners, &static_types)
                            .validate(module.borrow_ast());
                    let collision_diagnostics = collisions
                        .into_iter()
                        .map(|diagnostic| ModuleDiagnostic {
                            module_path: module_path.clone(),
                            message: diagnostic.message,
                            location: Some((
                                diagnostic.location.l,
                                diagnostic.location.c,
                                diagnostic.location.v.len() as u32,
                            )),
                            phase: ErrorPhase::Analyzer,
                            severity: diagnostic.severity,
                        })
                        .collect();
                    (module_path.clone(), collision_diagnostics)
                })
                .collect::<HashMap<_, _>>()
        };

        let mut module_paths: Vec<_> = diagnostics.keys().cloned().collect();
        module_paths.sort();
        let mut cache = self.module_cache.write().unwrap();
        for (module_path, collision_diagnostics) in diagnostics {
            if let Some(module) = cache.get_mut(&module_path) {
                module.with_collision_errors_mut(|errors| *errors = collision_diagnostics);
            }
        }
        module_paths
    }

    fn generate_all_cached_modules(&self) {
        let module_paths = {
            let cache = self.module_cache.read().unwrap();
            Self::dependency_order(&cache, &self.entry)
        };
        for module_path in module_paths {
            self.revalidate_cached_module(&module_path);
        }
    }

    fn dependency_order(cache: &HashMap<ModulePath, ModuleData>, entry: &str) -> Vec<ModulePath> {
        fn visit(
            module_path: &str,
            cache: &HashMap<ModulePath, ModuleData>,
            visiting: &mut HashSet<ModulePath>,
            visited: &mut HashSet<ModulePath>,
            result: &mut Vec<ModulePath>,
        ) {
            if visited.contains(module_path) || !visiting.insert(module_path.to_string()) {
                return;
            }
            if let Some(module) = cache.get(module_path) {
                for import in module.borrow_imports() {
                    visit(import, cache, visiting, visited, result);
                }
            }
            visiting.remove(module_path);
            if visited.insert(module_path.to_string()) {
                result.push(module_path.to_string());
            }
        }

        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        visit(entry, cache, &mut visiting, &mut visited, &mut result);
        let mut remaining: Vec<_> = cache
            .keys()
            .filter(|module_path| !visited.contains(*module_path))
            .cloned()
            .collect();
        remaining.sort();
        for module_path in remaining {
            visit(
                &module_path,
                cache,
                &mut visiting,
                &mut visited,
                &mut result,
            );
        }
        result
    }

    fn transitive_importers(
        cache: &HashMap<ModulePath, ModuleData>,
        changed_module: &str,
    ) -> Vec<ModulePath> {
        let mut reverse_imports: HashMap<ModulePath, Vec<ModulePath>> = HashMap::new();
        for (module_path, module) in cache {
            for import in module.borrow_imports() {
                reverse_imports
                    .entry(import.clone())
                    .or_default()
                    .push(module_path.clone());
            }
        }
        for importers in reverse_imports.values_mut() {
            importers.sort();
            importers.dedup();
        }

        let mut visited = HashSet::from([changed_module.to_string()]);
        let mut pending = VecDeque::from([changed_module.to_string()]);
        let mut result = Vec::new();

        while let Some(imported_module) = pending.pop_front() {
            let Some(importers) = reverse_imports.get(&imported_module) else {
                continue;
            };
            for importer in importers {
                if visited.insert(importer.clone()) {
                    result.push(importer.clone());
                    pending.push_back(importer.clone());
                }
            }
        }

        result
    }

    fn revalidate_cached_module(&self, module_path: &str) -> bool {
        let validation = {
            let cache = self.module_cache.read().unwrap();
            let Some(module) = cache.get(module_path) else {
                return false;
            };

            let imports = module.borrow_imports().to_vec();
            let import_errors = self.validate_imports(module, module_path);
            let lexer_errors = module.borrow_lexer_errors().clone();
            let parser_errors = module.borrow_parser_errors().clone();
            let imports_have_fatal_diagnostics = imports.iter().any(|import| {
                cache.get(import).is_some_and(|imported_module| {
                    imported_module
                        .borrow_lexer_errors()
                        .iter()
                        .chain(imported_module.borrow_parser_errors().iter())
                        .chain(imported_module.borrow_analyzer_errors().iter())
                        .chain(imported_module.borrow_collision_errors().iter())
                        .chain(imported_module.borrow_module_errors().iter())
                        .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err)
                })
            });
            let generation_allowed = !imports_have_fatal_diagnostics
                && !Self::has_fatal_diagnostics(&import_errors)
                && !Self::has_fatal_diagnostics(&lexer_errors)
                && !Self::has_fatal_diagnostics(&parser_errors);

            let analysis_only;
            let analyzer = if generation_allowed {
                &self.analyzer
            } else {
                analysis_only = Analyzer::new(false, self.plugins);
                &analysis_only
            };
            let diagnostics = analyzer.run(
                module.borrow_ast(),
                module,
                &imports,
                &cache,
                self.plugins,
                &Config::get().plugins.config,
            );
            let analyzer_errors = diagnostics
                .iter()
                .map(|diagnostic| ModuleDiagnostic {
                    module_path: module_path.to_string(),
                    message: diagnostic.message.clone(),
                    location: Some((
                        diagnostic.location.l,
                        diagnostic.location.c,
                        diagnostic.location.v.len() as u32,
                    )),
                    phase: ErrorPhase::Analyzer,
                    severity: diagnostic.severity,
                })
                .collect::<Vec<_>>();

            (analyzer_errors, import_errors)
        };

        let mut cache = self.module_cache.write().unwrap();
        let Some(module) = cache.get_mut(module_path) else {
            return false;
        };
        module.with_analyzer_errors_mut(|errors| *errors = validation.0);
        module.with_module_errors_mut(|errors| *errors = validation.1);
        true
    }

    // ── Import resolution & validation ──────────────────────────────

    /// Resolves an import path (e.g. `["a", "b"]`) relative to the workspace root.
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
            if let Declaration::Import { path, location } = decl {
                let segments = path.to_vec();
                match self.resolve_import(&segments, None) {
                    Ok((_, abs_path)) => {
                        if !abs_path.exists() {
                            errors.push(ModuleDiagnostic {
                                module_path: module_path.to_string(),
                                message: format!(
                                    "Module '{}' not found (expected at '{}')",
                                    path.join("/"),
                                    abs_path.display()
                                ),
                                location: Some((location.l, location.c, location.v.len() as u32)),
                                phase: ErrorPhase::Analyzer,
                                severity: XenoDiagSeverity::Err,
                            });
                        }
                    }
                    Err(_) => {
                        errors.push(ModuleDiagnostic {
                            module_path: module_path.to_string(),
                            message: format!("Cannot resolve module '{}'", path.join("/")),
                            location: Some((location.l, location.c, location.v.len() as u32)),
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
        let segments: Vec<&str> = path_so_far.split('/').collect();
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

    /// Purges every loaded module and returns the number removed.
    pub fn purge_module_cache(&self) -> usize {
        let mut cache = self.module_cache.write().unwrap();
        let removed = cache.len();
        cache.clear();
        removed
    }

    /// Runs a closure with read access to a module's cached tokens and AST.
    // allow async closures
    pub fn with_module<T, F>(&self, module_path: &str, f: F) -> Option<T>
    where
        F: for<'a> FnOnce(&'a [Token<'a>], &'a [Declaration<'a>], &'a ModuleData) -> T,
    {
        let cache = self.module_cache.read().unwrap();
        let module = cache.get(module_path)?;
        Some(f(module.borrow_tokens(), module.borrow_ast(), module))
    }

    /// Gets all errors for a specific module.
    pub fn get_all_errors_for(&self, module_path: &str) -> Vec<ModuleDiagnostic> {
        let cache = self.module_cache.read().unwrap();
        if let Some(module) = cache.get(module_path) {
            let mut all = Vec::new();
            all.extend(module.borrow_lexer_errors().iter().cloned());
            all.extend(module.borrow_parser_errors().iter().cloned());
            all.extend(module.borrow_analyzer_errors().iter().cloned());
            all.extend(module.borrow_collision_errors().iter().cloned());
            all.extend(module.borrow_module_errors().iter().cloned());
            all
        } else {
            vec![]
        }
    }

    fn get_all_cached_errors(&self) -> Vec<ModuleDiagnostic> {
        let cache = self.module_cache.read().unwrap();
        let mut module_paths: Vec<_> = cache.keys().cloned().collect();
        module_paths.sort();
        module_paths
            .into_iter()
            .flat_map(|module_path| {
                let module = cache
                    .get(&module_path)
                    .expect("cached module path should remain available");
                module
                    .borrow_lexer_errors()
                    .iter()
                    .chain(module.borrow_parser_errors())
                    .chain(module.borrow_analyzer_errors())
                    .chain(module.borrow_collision_errors())
                    .chain(module.borrow_module_errors())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Gets errors of a specific phase for a module.
    pub fn get_errors_by_phase(
        &self,
        module_path: &str,
        phase: ErrorPhase,
    ) -> Vec<ModuleDiagnostic> {
        let cache = self.module_cache.read().unwrap();
        if let Some(module) = cache.get(module_path) {
            match phase {
                ErrorPhase::Lexer => module.borrow_lexer_errors().clone(),
                ErrorPhase::Parser => module.borrow_parser_errors().clone(),
                ErrorPhase::Analyzer => module
                    .borrow_analyzer_errors()
                    .iter()
                    .chain(module.borrow_collision_errors())
                    .cloned()
                    .collect(),
                ErrorPhase::Module => module.borrow_module_errors().clone(),
            }
        } else {
            vec![]
        }
    }

    // ── Declaration lookup ──────────────────────────────────────────

    pub fn find_declaration(&self, current_module: &str, name: &str) -> Option<DeclarationInfo> {
        self._find_declaration(
            current_module,
            name,
            &self.module_cache.read().unwrap(),
            &mut HashSet::new(),
        )
    }

    fn _find_declaration<'s, 'c: 's>(
        &self,
        current_module: &'c str,
        name: &str,
        cache: &'c HashMap<String, ModuleData>,
        tried: &'s mut HashSet<&'c str>,
    ) -> Option<DeclarationInfo> {
        tried.insert(current_module);

        let module = cache.get(current_module)?;
        if let Some(d) = module.borrow_declarations().get(&name) {
            return Some(d.clone());
        }

        for import in module.borrow_imports() {
            if tried.contains(import.as_str()) {
                continue;
            }
            tried.insert(import);
            if let Some(m) = cache.get(import) {
                if let Some(d) = m.borrow_declarations().get(name) {
                    return Some(d.clone());
                }
            }
        }

        None
    }

    pub fn get_all_declarations_in_scope(&self, current_module: &str) -> Vec<DeclarationInfo> {
        let mut decls = Vec::new();
        self._get_all_declarations_in_scope(
            current_module,
            &mut decls,
            &self.module_cache.read().unwrap(),
            &mut HashSet::new(),
        );
        decls
    }

    fn _get_all_declarations_in_scope<'s, 'c: 's>(
        &self,
        current_module: &'c str,
        decls: &mut Vec<DeclarationInfo>,
        cache: &'c HashMap<String, ModuleData>,
        tried: &'s mut HashSet<&'c str>,
    ) {
        tried.insert(current_module);

        if let Some(m) = cache.get(current_module) {
            decls.extend(m.borrow_declarations().values().cloned());

            for import in m.borrow_imports() {
                if tried.contains(import.as_str()) {
                    continue;
                }
                if let Some(im) = cache.get(import) {
                    decls.extend(im.borrow_declarations().values().cloned());
                }
            }
        }
    }

    // ── Internal ────────────────────────────────────────────────────

    fn _create_module_data(
        module_path: &ModulePath,
        abs_path: PathBuf,
        source: String,
        hash: u64,
    ) -> Result<ModuleData, Vec<ModuleDiagnostic>> {
        // Collect parser errors via shared mutability since ouroboros closures
        // can't write to head fields during construction.
        let parser_errors_cell: std::cell::RefCell<Vec<ModuleDiagnostic>> =
            std::cell::RefCell::new(Vec::new());

        let mut md = ModuleDataTryBuilder {
            abs_path,
            module_path: module_path.clone(),
            source,
            hash,
            changed: true,
            lexer_errors: Vec::new(),
            parser_errors: Vec::new(),
            analyzer_errors: Vec::new(),
            collision_errors: Vec::new(),
            module_errors: Vec::new(),
            imports: Vec::new(),
            tokens_builder: |source| {
                Lexer::tokenize(source)
                    .inspect(|ts| {
                        if Config::get().debug.tokens {
                            eprint!("{:?}", ts);
                        }
                    })
                    .map_err(|e| {
                        vec![ModuleDiagnostic {
                            module_path: module_path.clone(),
                            message: e.message.to_string(),
                            location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                            phase: ErrorPhase::Lexer,
                            severity: e.severity,
                        }]
                    })
            },
            ast_builder: |tokens| {
                let (ast, diagnostics) = Parser::parse(tokens);

                if Config::get().debug.ast {
                    eprint!("{:?}", ast);
                }

                parser_errors_cell
                    .borrow_mut()
                    .extend(diagnostics.iter().map(|e| ModuleDiagnostic {
                        module_path: module_path.clone(),
                        message: e.message.to_string(),
                        location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                        phase: ErrorPhase::Parser,
                        severity: e.severity,
                    }));

                Ok(ast)
            },
            declarations_builder: |ast: &XenoAst, abs_path: &PathBuf, module_path: &ModulePath| {
                Ok(ast
                    .iter()
                    .filter_map(|d| match d {
                        Declaration::Type {
                            docs,
                            name,
                            generics,
                            ty,
                            ..
                        } => Some((
                            name.v,
                            DeclarationInfo {
                                name: name.v.to_string(),
                                module_path: module_path.to_string(),
                                abs_path: abs_path.clone(),
                                docs: docs.map(|d| d.to_string()),
                                line: name.l,
                                column: name.c,
                                name_len: name.v.len() as u32,
                                semantic: TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0),
                            },
                        )),
                        _ => None,
                    })
                    .collect())
            },
        }
        .try_build()?;

        // Populate parser_errors field from what was collected during build
        let collected_parser_errors = parser_errors_cell.into_inner();
        md.with_parser_errors_mut(|errs| *errs = collected_parser_errors);

        // Populate imports list
        let import_list: Vec<ModulePath> = md
            .borrow_ast()
            .iter()
            .filter_map(|d| match d {
                Declaration::Import { path, .. } => Some(path.join("/")),
                _ => None,
            })
            .collect();
        md.with_imports_mut(|imports| *imports = import_list);

        Ok(md)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        path::PathBuf,
    };

    use super::{ErrorPhase, ModuleData, ModuleDiagnostic};
    use crate::{
        config::PluginConfigs,
        module::XenoRegistry,
        semantic::{Analyzer, NameCollisionValidator, BUILTIN_TYPES},
        utils::calculate_hash,
        XenoDiagSeverity,
    };

    fn diagnostic(severity: XenoDiagSeverity) -> ModuleDiagnostic {
        ModuleDiagnostic {
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
        )
        .expect("semantic test source should parse")
    }

    #[test]
    fn transitive_importers_are_deduplicated_across_cycles_and_diamonds() {
        let mut cache = HashMap::new();
        cache.insert(
            "leaf".to_string(),
            parsed_module("leaf", "type Leaf = string;"),
        );
        cache.insert(
            "a".to_string(),
            parsed_module("a", "import leaf; import top; type A = Leaf;"),
        );
        cache.insert(
            "b".to_string(),
            parsed_module("b", "import leaf; type B = Leaf;"),
        );
        cache.insert(
            "top".to_string(),
            parsed_module("top", "import a; import b; type Top = A | B;"),
        );

        assert_eq!(
            XenoRegistry::transitive_importers(&cache, "leaf"),
            vec!["a".to_string(), "b".to_string(), "top".to_string()]
        );
    }

    #[test]
    fn purging_module_cache_removes_all_modules() {
        static EMPTY_PLUGINS: Vec<&'static crate::plugins::XenoPlugin<'static>> = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "test".to_string(),
            parsed_module("test", "type Test = string;"),
        );
        let registry = XenoRegistry {
            module_cache: std::sync::RwLock::new(cache),
            root: PathBuf::new(),
            entry: "test".to_string(),
            plugins: &EMPTY_PLUGINS,
            analyzer: Analyzer::new(false, &EMPTY_PLUGINS),
        };

        assert_eq!(registry.purge_module_cache(), 1);
        assert!(registry.module_cache.read().unwrap().is_empty());
    }

    fn analyze(cache: &HashMap<String, ModuleData>, module_path: &str) -> Vec<String> {
        let module = cache.get(module_path).expect("module should be cached");
        let imports = module.borrow_imports().to_vec();
        let mut errors = Analyzer::new(false, &[])
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
            NameCollisionValidator::new(module_path, &type_owners, &static_types)
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
        let source = "type Before = Later; type Later = string;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn unknown_annotations_warn_while_unknown_types_error() {
        let mut cache = HashMap::new();
        cache.insert(
            "test".to_string(),
            parsed_module("test", "type Test = Missing @Lombok;"),
        );
        let module = cache.get("test").expect("module should be cached");

        let diagnostics = Analyzer::new(false, &[]).run(
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
        let source = "type Duplicate = string; type Duplicate = u8;";
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
            parsed_module("first", "type Shared = string;"),
        );
        cache.insert(
            "second".to_string(),
            parsed_module("second", "type Shared = u8;"),
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
            "type User = { id: string, id: u8 }; type State = enum { ready: string, ready: u8 };";
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
        let source = "type First = { id: string }; type Second = { id: u8 }; type Left = enum { ready: string }; type Right = enum { ready: u8 };";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn semantic_pass_rejects_duplicate_and_shadowing_generic_parameters() {
        let source = "type Existing = string; type Pair<T, T> = T; type UserGeneric<Existing> = Existing; type BuiltinGeneric<string> = string;";
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
            parsed_module("shared", "type Existing = string;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module("test", "type UserGeneric<Existing> = Existing;"),
        );

        let errors = analyze(&cache, "test");
        assert_eq!(
            errors,
            vec!["Generic parameter 'Existing' shadows an existing type"]
        );
    }

    #[test]
    fn semantic_pass_allows_generic_names_to_repeat_in_separate_declarations() {
        let source = "type First<T> = T; type Second<T> = T;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn full_cache_revalidation_finds_types_added_after_generic_declarations() {
        static EMPTY_PLUGINS: Vec<&'static crate::plugins::XenoPlugin<'static>> = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "early".to_string(),
            parsed_module("early", "type Wrapper<Late> = Late;"),
        );
        cache.insert(
            "late".to_string(),
            parsed_module("late", "type Late = string;"),
        );
        let registry = XenoRegistry {
            module_cache: std::sync::RwLock::new(cache),
            root: PathBuf::new(),
            entry: "early".to_string(),
            plugins: &EMPTY_PLUGINS,
            analyzer: Analyzer::new(false, &EMPTY_PLUGINS),
        };

        registry.refresh_name_collision_diagnostics();

        let errors = registry.get_all_errors_for("early");
        assert!(errors.iter().any(|diagnostic| {
            diagnostic.message == "Generic parameter 'Late' shadows an existing type"
        }));
    }

    #[test]
    fn collision_refresh_reports_duplicate_types_on_both_declarations() {
        static EMPTY_PLUGINS: Vec<&'static crate::plugins::XenoPlugin<'static>> = Vec::new();

        let mut cache = HashMap::new();
        cache.insert(
            "first".to_string(),
            parsed_module("first", "type Shared = string;"),
        );
        cache.insert(
            "second".to_string(),
            parsed_module("second", "type Shared = u8;"),
        );
        let registry = XenoRegistry {
            module_cache: std::sync::RwLock::new(cache),
            root: PathBuf::new(),
            entry: "first".to_string(),
            plugins: &EMPTY_PLUGINS,
            analyzer: Analyzer::new(false, &EMPTY_PLUGINS),
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
            parsed_module("leaf", "type Leaf = string;"),
        );
        cache.insert(
            "middle".to_string(),
            parsed_module("middle", "import leaf; type Middle = Leaf;"),
        );
        cache.insert(
            "entry".to_string(),
            parsed_module("entry", "import middle; type Entry = Middle;"),
        );

        assert_eq!(
            XenoRegistry::dependency_order(&cache, "entry"),
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
            parsed_module("base", "type Imported = string;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import base; type UsesImported = Imported @minlen(1);",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn generic_constraints_accept_trait_names_and_follow_parentage() {
        let source = "type Before = Box<u8>; type Box<T: NumberLiteral> = T; type Identity<T> = T; type Forward<T: NumberLiteral> = Box<T>; type Nested<T: NumberLiteral> = Box<Identity<T>>; type Float = Box<f32>; type Bad = Box<string>;";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        let errors = analyze(&cache, "test");
        assert_eq!(errors.len(), 1, "unexpected errors: {errors:#?}");
        assert!(errors[0].contains("does not satisfy constraint 'NumberLiteral'"));
        assert!(errors[0].contains("string"));
    }

    #[test]
    fn hierarchy_preserves_duplicate_names_from_loaded_modules() {
        let mut cache = HashMap::new();
        cache.insert(
            "text".to_string(),
            parsed_module("text", "type Shared = string;"),
        );
        cache.insert(
            "numeric".to_string(),
            parsed_module("numeric", "type Shared = u8;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module("test", "import text; type Local = Shared @minlen(1);"),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn imported_types_resolve_parents_through_their_own_imports() {
        let mut cache = HashMap::new();
        cache.insert(
            "base".to_string(),
            parsed_module("base", "type Text = string;"),
        );
        cache.insert(
            "middle".to_string(),
            parsed_module("middle", "import base; type Imported = Text;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import middle; type UsesImported = Imported @minlen(1);",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn imported_generics_resolve_arguments_in_the_calling_module() {
        let mut cache = HashMap::new();
        cache.insert(
            "containers".to_string(),
            parsed_module("containers", "type Box<T: NumberLiteral> = T;"),
        );
        cache.insert(
            "test".to_string(),
            parsed_module(
                "test",
                "import containers; type Local = u8; type Good = Box<Local>;",
            ),
        );

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn match_accepts_string_and_string_descendants() {
        let source = "type Email = string @match(/.*@.*\\.com/) @minlen(10); type BuiltinDescendant = uuid @match(/^[0-9a-f-]+$/); type UserDescendant = Email @match(/^alias$/);";
        let mut cache = HashMap::new();
        cache.insert("test".to_string(), parsed_module("test", source));

        assert!(analyze(&cache, "test").is_empty());
    }

    #[test]
    fn match_rejects_types_that_do_not_descend_from_string() {
        let source = "type Number = u8 @match(/^[0-9]+$/); type Bytes = binary @match(/.*/); type Date = date @match(/^2026-/);";
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
        let source = "type Missing = string @match(); type StringArg = string @match(\"pattern\"); type Multiple = string @match(/a/, /b/);";
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
