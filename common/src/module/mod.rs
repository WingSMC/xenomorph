use ouroboros::self_referencing;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod types;

use crate::config::Config;
use crate::lexer::{Lexer, Token, XenoTokens};
use crate::module::types::{DeclarationInfo, ErrorPhase, ModuleError, ModulePath};
use crate::parser::{Declaration, Expr, Parser, XenoAst};
use crate::plugins::XenoPlugin;
use crate::semantic::analyze;
use crate::utils::calculate_hash;

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
    pub lexer_errors: Vec<ModuleError>,
    /// Parser errors.
    pub parser_errors: Vec<ModuleError>,
    /// Semantic analyzer errors.
    pub analyzer_errors: Vec<ModuleError>,
    /// Module-level errors (file not found, import resolution, etc.)
    pub module_errors: Vec<ModuleError>,
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
fn get_root() -> Result<(PathBuf, String), ModuleError> {
    let config = Config::get();
    let mut joined = config.workdir.join(Path::new(&config.parser.entry));
    joined.add_extension("xen");
    let entry_file = joined.canonicalize().map_err(|e| ModuleError {
        module_path: config.parser.entry.clone(),
        message: format!("Cannot resolve entry file '{:?}': {}", joined, e),
        location: None,
        phase: ErrorPhase::Module,
    })?;

    let root_err = || ModuleError {
        module_path: config.parser.entry.clone(),
        message: format!(
            "Cannot determine workspace root from entry file '{}'",
            entry_file.display()
        ),
        location: None,
        phase: ErrorPhase::Module,
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
            .ok_or_else(|| ModuleError {
                module_path: config.parser.entry.clone(),
                message: format!(
                    "Entry file '{}' does not have a valid stem",
                    entry_file.display()
                ),
                location: None,
                phase: ErrorPhase::Module,
            })?,
    ))
}

/// Thread-safe module registry. Single source of truth for all module data.
pub struct XenoRegistry {
    pub module_cache: RwLock<HashMap<ModulePath, ModuleData>>,
    pub root: PathBuf,
    pub entry: String,
    pub plugins: &'static Vec<&'static XenoPlugin<'static>>,
}

impl XenoRegistry {
    pub fn new() -> Result<XenoRegistry, ModuleError> {
        let (root, entry) = get_root()?;
        Ok(XenoRegistry {
            module_cache: RwLock::new(HashMap::default()),
            root,
            entry,
            plugins: XenoPlugin::get_plugins(),
        })
    }

    /// Initializes a new `XenoRegistry` and loads the entire workspace starting from the entry module.
    pub fn load_workspace() -> Result<XenoRegistry, Vec<ModuleError>> {
        let reg = XenoRegistry::new().map_err(|e| vec![e])?;
        let errs = reg.load_module(&[&reg.entry], true, None);
        if !errs.is_empty() {
            return Err(errs);
        }
        Ok(reg)
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
    pub fn load_module_from_uri(&self, uri: &str) -> Vec<ModuleError> {
        let path_res = PathBuf::from(uri).canonicalize().map_err(|e| ModuleError {
            module_path: uri.to_string(),
            message: format!("Cannot resolve URI '{}': {}", uri, e),
            location: None,
            phase: ErrorPhase::Module,
        });
        let path = match path_res {
            Ok(p) => p,
            Err(e) => return vec![e],
        };

        let relative = match path.strip_prefix(&self.root) {
            Ok(r) => r,
            Err(e) => {
                return vec![ModuleError {
                    module_path: uri.to_string(),
                    message: format!(
                        "URI '{}' is not within workspace root '{}': {}",
                        uri,
                        self.root.display(),
                        e
                    ),
                    location: None,
                    phase: ErrorPhase::Module,
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
    pub fn load_module_from_source(&self, abs_path: &Path, source: String) -> Vec<ModuleError> {
        let module_path = match self.abs_path_to_module_path(abs_path) {
            Some(mp) => mp,
            None => {
                return vec![ModuleError {
                    module_path: abs_path.to_string_lossy().to_string(),
                    message: format!(
                        "Path '{}' is not within workspace root '{}'",
                        abs_path.display(),
                        self.root.display()
                    ),
                    location: None,
                    phase: ErrorPhase::Module,
                }]
            }
        };

        let canonical = match abs_path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                return vec![ModuleError {
                    module_path,
                    message: format!("Cannot canonicalize '{}': {}", abs_path.display(), e),
                    location: None,
                    phase: ErrorPhase::Module,
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
    ) -> Vec<ModuleError> {
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
                return vec![ModuleError {
                    module_path,
                    message: format!("Failed to read file '{}': {}", abs_path.display(), e),
                    location: None,
                    phase: ErrorPhase::Module,
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

        self._load_module_inner(module_path, abs_path, source, hash)
    }

    fn _load_module_inner(
        &self,
        module_path: ModulePath,
        abs_path: PathBuf,
        source: String,
        hash: u64,
    ) -> Vec<ModuleError> {
        let mut errors: Vec<ModuleError> = Vec::new();

        let md = match Self::_create_module_data(&module_path, abs_path, source, hash) {
            Ok(r) => r,
            Err(e) => {
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

            let mut known_types: HashSet<&str> = HashSet::new();
            let mut known_annotations: HashSet<&str> = HashSet::new();

            for t in &crate::semantic::BUILTIN_TYPES {
                known_types.insert(t.name);
            }
            for a in &crate::semantic::BUILTIN_ANNOTATIONS {
                known_annotations.insert(a.name);
            }
            for plugin in self.plugins {
                if let Some(provide) = plugin.provide_types {
                    for pc in provide() {
                        known_types.insert(pc.label);
                    }
                }
                if let Some(provide) = plugin.provide_annotations {
                    for pc in provide() {
                        known_annotations.insert(pc.label);
                    }
                }
            }
            for name in md.borrow_declarations().keys() {
                known_types.insert(name);
            }
            for import in &imports {
                if let Some(m) = cache.get(import) {
                    for name in m.borrow_declarations().keys() {
                        known_types.insert(name);
                    }
                }
            }

            let analyzer_errors: Vec<ModuleError> =
                analyze(md.borrow_ast(), &known_types, &known_annotations)
                    .iter()
                    .map(|e| ModuleError {
                        module_path: module_path.clone(),
                        message: e.message.clone(),
                        location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                        phase: ErrorPhase::Analyzer,
                    })
                    .collect();

            let import_errors = self.validate_imports(md, &module_path);
            let lexer_errs = md.borrow_lexer_errors().clone();
            let parser_errs = md.borrow_parser_errors().clone();

            (analyzer_errors, import_errors, lexer_errs, parser_errs)
        };

        // ── Step 4: Write error fields back into the cached module ──
        {
            let mut cache = self.module_cache.write().unwrap();
            let md = cache.get_mut(&module_path).unwrap();
            md.with_analyzer_errors_mut(|errs| *errs = analyzer_errors.clone());
            md.with_module_errors_mut(|errs| *errs = import_errors.clone());
        }

        errors.extend(lexer_errs);
        errors.extend(parser_errs);
        errors.extend(analyzer_errors);
        errors.extend(import_errors);

        errors
    }

    // ── Import resolution & validation ──────────────────────────────

    /// Resolves an import path (e.g. `["a", "b"]`) relative to the workspace root.
    pub fn resolve_import(
        &self,
        import_array: &[&str],
        import_str: Option<&str>,
    ) -> Result<(ModulePath, PathBuf), ModuleError> {
        let import_str = import_str
            .map(|s| s.to_string())
            .unwrap_or_else(|| import_array.join("/"));
        let mut pathbuf = self.root.join(&import_str);
        pathbuf.add_extension("xen");

        match pathbuf.canonicalize() {
            Ok(p) => Ok((import_str, p)),
            Err(e) => Err(ModuleError {
                module_path: import_str.clone(),
                message: format!("Cannot resolve import '{}': {}", import_str, e),
                location: None,
                phase: ErrorPhase::Module,
            }),
        }
    }

    /// Validates all import declarations in a module.
    fn validate_imports(&self, module: &ModuleData, module_path: &str) -> Vec<ModuleError> {
        let mut errors = Vec::new();
        for decl in module.borrow_ast().iter() {
            if let Declaration::Import { path, location } = decl {
                let segments: Vec<&str> = path.iter().copied().collect();
                match self.resolve_import(&segments, None) {
                    Ok((_, abs_path)) => {
                        if !abs_path.exists() {
                            errors.push(ModuleError {
                                module_path: module_path.to_string(),
                                message: format!(
                                    "Module '{}' not found (expected at '{}')",
                                    path.join("/"),
                                    abs_path.display()
                                ),
                                location: Some((location.l, location.c, location.v.len() as u32)),
                                phase: ErrorPhase::Analyzer,
                            });
                        }
                    }
                    Err(_) => {
                        errors.push(ModuleError {
                            module_path: module_path.to_string(),
                            message: format!("Cannot resolve module '{}'", path.join("/")),
                            location: Some((location.l, location.c, location.v.len() as u32)),
                            phase: ErrorPhase::Analyzer,
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

    /// Runs a closure with read access to a module's cached tokens and AST.
    pub fn with_module<T, F>(&self, module_path: &str, f: F) -> Option<T>
    where
        F: for<'a> FnOnce(&'a [Token<'a>], &'a [Declaration<'a>], &'a ModuleData) -> T,
    {
        let cache = self.module_cache.read().unwrap();
        let module = cache.get(module_path)?;
        Some(f(module.borrow_tokens(), module.borrow_ast(), module))
    }

    /// Gets all errors for a specific module.
    pub fn get_all_errors_for(&self, module_path: &str) -> Vec<ModuleError> {
        let cache = self.module_cache.read().unwrap();
        if let Some(module) = cache.get(module_path) {
            let mut all = Vec::new();
            all.extend(module.borrow_lexer_errors().iter().cloned());
            all.extend(module.borrow_parser_errors().iter().cloned());
            all.extend(module.borrow_analyzer_errors().iter().cloned());
            all.extend(module.borrow_module_errors().iter().cloned());
            all
        } else {
            vec![]
        }
    }

    /// Gets errors of a specific phase for a module.
    pub fn get_errors_by_phase(&self, module_path: &str, phase: ErrorPhase) -> Vec<ModuleError> {
        let cache = self.module_cache.read().unwrap();
        if let Some(module) = cache.get(module_path) {
            match phase {
                ErrorPhase::Lexer => module.borrow_lexer_errors().clone(),
                ErrorPhase::Parser => module.borrow_parser_errors().clone(),
                ErrorPhase::Analyzer => module.borrow_analyzer_errors().clone(),
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
    ) -> Result<ModuleData, Vec<ModuleError>> {
        // Collect parser errors via shared mutability since ouroboros closures
        // can't write to head fields during construction.
        let parser_errors_cell: std::cell::RefCell<Vec<ModuleError>> =
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
            module_errors: Vec::new(),
            imports: Vec::new(),
            tokens_builder: |source| {
                Lexer::tokenize(source).map_err(|e| {
                    vec![ModuleError {
                        module_path: module_path.clone(),
                        message: format!("{}", e.message),
                        location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                        phase: ErrorPhase::Lexer,
                    }]
                })
            },
            ast_builder: |tokens| {
                let (ast, parse_errors) = Parser::parse(tokens);

                parser_errors_cell
                    .borrow_mut()
                    .extend(parse_errors.iter().map(|e| ModuleError {
                        module_path: module_path.clone(),
                        message: format!("{}", e.message),
                        location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                        phase: ErrorPhase::Parser,
                    }));

                Ok(ast)
            },
            declarations_builder: |ast: &XenoAst, abs_path: &PathBuf, module_path: &ModulePath| {
                Ok(ast
                    .iter()
                    .filter_map(|d| match d {
                        Declaration::TypeDecl { docs, name, t } => Some((
                            name.v,
                            DeclarationInfo {
                                name: name.v.to_string(),
                                module_path: module_path.to_string(),
                                abs_path: abs_path.clone(),
                                docs: docs.map(|d| d.to_string()),
                                line: name.l,
                                column: name.c,
                                name_len: name.v.len() as u32,
                                fields: {
                                    let v = t
                                        .iter()
                                        .filter_map(|item| match item {
                                            Expr::Struct(fields) => {
                                                Some(fields.iter().filter_map(|(d, e)| {
                                                    let t = e.get(0)?;
                                                    match t {
                                                        Expr::Identifier(id) => Some((
                                                            d.v.to_string(),
                                                            id.v.to_string(),
                                                        )),
                                                        _ => None,
                                                    }
                                                }))
                                            }
                                            _ => None,
                                        })
                                        .flatten()
                                        .collect::<Vec<(String, String)>>();
                                    (!v.is_empty()).then_some(v)
                                },
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
