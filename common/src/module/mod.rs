use ouroboros::self_referencing;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod types;

use crate::config::Config;
use crate::lexer::{Lexer, XenoTokens};
use crate::module::types::{DeclarationInfo, ModuleError, ModulePath};
use crate::parser::{Declaration, Expr, Parser, XenoAst};
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
    /// Errors encountered during lexing/parsing/analysis of this module.
    pub analysis_errors: Vec<ModuleError>,
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
/// Returns `(workspace_root, entry_module_name)` or a `ModuleError` if the entry file cannot be resolved.
fn get_root() -> Result<(PathBuf, String), ModuleError> {
    let config = Config::get();
    let mut joined = config.workdir.join(Path::new(&config.parser.entry));
    joined.add_extension("xen");
    let entry_file = joined.canonicalize().map_err(|e| ModuleError {
        module_path: config.parser.entry.clone(),
        message: format!("Cannot resolve entry file '{:?}': {}", joined, e),
        location: None,
    })?;

    let root_err = || ModuleError {
        module_path: config.parser.entry.clone(),
        message: format!(
            "Cannot determine workspace root from entry file '{}'",
            entry_file.display()
        ),
        location: None,
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
            })?,
    ))
}

/// Thread-safe handle to a ModuleRegistry.
pub struct XenoRegistry {
    pub module_cache: RwLock<HashMap<ModulePath, ModuleData>>,
    pub root: PathBuf,
    pub entry: String,
}

impl XenoRegistry {
    /// Initializes the registry by determining the workspace root and entry module from the config.
    /// If the **entry file** or **root directory** cannot be resolved, returns a ModuleError.
    pub fn new() -> Result<XenoRegistry, ModuleError> {
        let (root, entry) = get_root()?;
        Ok(XenoRegistry {
            module_cache: RwLock::new(HashMap::default()),
            root,
            entry,
        })
    }

    /// Initializes a new `XenoRegistry` and loads
    /// the entire workspace starting from the entry module.
    pub fn load_workspace() -> Result<XenoRegistry, Vec<ModuleError>> {
        let reg = XenoRegistry::new().map_err(|e| vec![e])?;
        let errs = reg.load_module(&[&reg.entry], true, None);

        if !errs.is_empty() {
            return Err(errs);
        }

        Ok(reg)
    }

    /// Loads a module from a given URI (e.g. "C:/workspace/src/api/user.xen")
    /// and all its imports into the registry. Wrapper for `XenoRegistry::load_module`.
    pub fn load_module_from_uri(&self, uri: &str) -> Vec<ModuleError> {
        let path_res = PathBuf::from(uri).canonicalize().map_err(|e| ModuleError {
            module_path: uri.to_string(),
            message: format!("Cannot resolve URI '{}': {}", uri, e),
            location: None,
        });
        let path = match path_res {
            Ok(p) => p,
            Err(e) => return vec![e],
        };

        let root = self.root.as_path();
        let relative = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(e) => {
                return vec![ModuleError {
                    module_path: uri.to_string(),
                    message: format!(
                        "URI '{}' is not within the workspace root '{}': {}",
                        uri,
                        root.display(),
                        e
                    ),
                    location: None,
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

    /// Recursively loads a .xen file and all its imports into the registry.
    /// Returns a list of module-level errors (file not found, parse errors, etc.)
    /// - `import_segments`: the absolute import path segments, e.g. `["a", "b"]` for `import a/b;`
    /// - `force`: If true, forces reloading (this is skipped later if hash matches).
    ///            This is used when we want to reload a module due to a file change.
    /// Returns a list of module-level errors (file not found, parse errors, etc.) and a boolean indicating whether the module was **actually** (re)loaded.
    /// The boolean is used for change detection.
    // TODO import diagnostics & completion & recovery
    pub fn load_module(
        &self,
        import_segments: &[&str],
        force: bool,
        import_str: Option<&str>,
    ) -> Vec<ModuleError> {
        println!("Load module: {:?}", import_segments);

        // Resolve the import segment array to an absolute
        // file path and a canonical module path
        let (module_path, abs_path) = match self.resolve_import(import_segments, import_str) {
            Err(e) => return vec![e],
            Ok(fp) => fp,
        };

        // Skip if already loaded unless forced
        if !force {
            if self.module_cache.read().unwrap().contains_key(&module_path) {
                return vec![];
            }
        }

        // Read the source text
        let source = match fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                return vec![ModuleError {
                    module_path,
                    message: format!("Failed to read file '{}': {}", abs_path.display(), e),
                    location: None,
                }];
            }
        };

        // Calculate hash and check if we can skip reloading
        let hash = calculate_hash(&source);
        if force {
            let r = self.module_cache.read().unwrap();
            if let Some(existing) = r.get(&module_path) {
                if *existing.borrow_hash() == hash {
                    return vec![];
                }
            }
        }

        // Insert this module into the registry and extract imports so we can recursively load them.
        let mut errors: Vec<ModuleError> = Vec::new();
        let imports: Vec<(Vec<String>, String)> = {
            let module_data_res = XenoRegistry::_create_module_data(
                &module_path,
                abs_path,
                source,
                hash,
                &mut errors,
            );

            let mut md = match module_data_res {
                Ok(r) => r,
                Err(e) => {
                    errors.extend(e);
                    return errors;
                }
            };

            md.with_analysis_errors_mut(|ae| ae.extend(errors.iter().cloned()));

            // Collect imports from this file before inserting into registry.
            // We need to lex and parse to find import declarations.
            let import_paths = md
                .borrow_ast()
                .iter()
                .filter_map(|d| match d {
                    Declaration::Import { path, .. } => {
                        let owned_path =
                            path.iter().map(|s| s.to_string()).collect::<Vec<String>>();
                        let joined = owned_path.join("/");
                        Some((owned_path, joined))
                    }
                    _ => None,
                })
                .collect();

            self.module_cache
                .write()
                .unwrap()
                .insert(module_path.clone(), md);

            import_paths
        };

        // Recursively load imports
        for (segments, joined) in imports {
            errors.extend(self.load_module(
                &segments.iter().map(|s| s.as_str()).collect::<Vec<&str>>(),
                false,
                Some(&joined),
            ));
        }

        errors
    }

    /// Resolves an import path (e.g. `["a", "b"]`) relative to the entry file.
    /// - `import_array`: the parsed path segments, e.g. `["a", "b"]` for `import a/b;`
    pub fn resolve_import(
        &self,
        import_array: &[&str],
        import_str: Option<&str>,
    ) -> Result<(ModulePath, PathBuf), ModuleError> {
        let root = self.root.as_path();
        let import_str = import_str
            .map(|s| s.to_string())
            .unwrap_or_else(|| import_array.join("/"));
        let mut pathbuf = root.join(&import_str);
        pathbuf.add_extension("xen");

        match pathbuf.canonicalize() {
            Ok(p) => Ok((import_str, p)),
            Err(e) => Err(ModuleError {
                module_path: import_str.clone(),
                message: format!("Cannot resolve import '{}': {}", import_str, e),
                location: None,
            }),
        }
    }

    /// Suggest available module paths that start with the given `path_so_far`.
    /// - `path_so_far`: the partial import path typed by the user, e.g. `api/u` for `import api/u...`
    pub fn suggest_import(&self, path_so_far: &str) -> Vec<(String, PathBuf)> {
        let root = self.root.as_path();
        let split = path_so_far.split('/');
        let prefix = split
            .clone()
            .take(split.clone().count() - 1)
            .collect::<Vec<&str>>()
            .join("/");
        let last_segment = split.last().unwrap_or(&"");
        let dir_to_search = root.join(&prefix);

        if let Ok(entries) = fs::read_dir(dir_to_search) {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let path = e.path();
                    if path.is_file() {
                        if let Some(stem) = path.file_stem() {
                            if let Some(stem_str) = stem.to_str() {
                                if stem_str.ends_with(".xen") {
                                    let candidate = stem_str.trim_end_matches(".xen");
                                    if candidate.starts_with(last_segment) {
                                        return Some((candidate.to_string(), path));
                                    }
                                }
                            }
                        }
                    }
                    None
                })
                .collect()
        } else {
            Vec::new()
        }
    }

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
        let d_opt = module.borrow_declarations().get(&name);
        match d_opt {
            Some(d) => return Some(d.clone()),
            _ => {}
        }

        for import in module.borrow_imports() {
            if tried.contains(import.as_str()) {
                return None;
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

        let module = cache.get(current_module);
        if let Some(m) = module {
            decls.extend(m.borrow_declarations().values().cloned());

            for import in m.borrow_imports() {
                if tried.contains(import.as_str()) {
                    continue;
                }

                let imported_module = cache.get(import);
                if let Some(im) = imported_module {
                    decls.extend(im.borrow_declarations().values().cloned());
                }
            }
        }
    }

    fn _create_module_data(
        module_path: &ModulePath,
        abs_path: PathBuf,
        source: String,
        hash: u64,
        errors: &mut Vec<ModuleError>,
    ) -> Result<ModuleData, Vec<ModuleError>> {
        ModuleDataTryBuilder {
            abs_path,
            module_path: module_path.clone(),
            source,
            hash,
            changed: true,
            analysis_errors: Vec::new(),
            imports: Vec::new(),
            tokens_builder: |source| {
                Lexer::tokenize(source).map_err(|e| {
                    vec![ModuleError {
                        module_path: module_path.clone(),
                        message: format!("Lexer error: {} at {}", e.message, e.location),
                        location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
                    }]
                })
            },
            ast_builder: |tokens| {
                let (ast, parse_errors) = Parser::parse(tokens);

                errors.extend(parse_errors.iter().map(|e| ModuleError {
                    module_path: module_path.clone(),
                    message: format!("Parse error: {} at {}", e.message, e.location),
                    location: Some((e.location.l, e.location.c, e.location.v.len() as u32)),
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
                                        // TODO guarantee 0th expr is type
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
        .try_build()
    }
}
