use ouroboros::self_referencing;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

pub mod types;

use crate::config::Config;
use crate::lexer::{Lexer, XenoTokens};
use crate::module::types::{DeclarationInfo, ModuleError, ModulePath};
use crate::parser::{Declaration, Parser, XenoAst};
use crate::utils::calculate_hash;

/// Information about a single module (one .xen file).
/// Owns the source text so that all borrows from tokens/ast remain valid.
#[self_referencing]
pub struct ModuleData {
    /// The absolute filesystem path.
    pub abs_path: PathBuf,
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
}

/// Determines the workspace root and entry module path from the config.
/// Returns `(workspace_root, entry_module_name)` or a `ModuleError` if the entry file cannot be resolved.
fn get_root() -> Result<(PathBuf, String), ModuleError> {
    let config = Config::get();
    let joined = config.workdir.join(&config.parser.entry);
    let entry_file = joined.canonicalize().map_err(|e| ModuleError {
        module_path: config.parser.entry.clone(),
        message: format!("Cannot resolve entry file '{:?}': {}", joined, e),
        location: None,
    })?;

    let root = entry_file
        .parent()
        .ok_or_else(|| ModuleError {
            module_path: config.parser.entry.clone(),
            message: format!(
                "Cannot determine workspace root from entry file '{}'",
                entry_file.display()
            ),
            location: None,
        })?
        .to_path_buf();

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
    module_cache: RwLock<HashMap<ModulePath, ModuleData>>,
    declaration_cache: RwLock<HashMap<String, DeclarationInfo>>,
    root: PathBuf,
    entry: String,
}

impl XenoRegistry {
    /// Initializes the registry by determining the workspace root and entry module from the config.
    /// If the **entry file** or **root directory** cannot be resolved, returns a ModuleError.
    pub fn new() -> Result<XenoRegistry, ModuleError> {
        let (root, entry) = get_root()?;
        Ok(XenoRegistry {
            module_cache: RwLock::new(HashMap::default()),
            declaration_cache: RwLock::new(HashMap::default()),
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

    /// Recursively loads a .xen file and all its imports into the registry.
    /// Returns a list of module-level errors (file not found, parse errors, etc.)
    /// - `import_segments`: the absolute import path segments, e.g. `["a", "b"]` for `import a/b;`
    /// - `force`: If true, forces reloading (this is skipped later if hash matches).
    ///            This is used when we want to reload a module due to a file change.
    /// Returns a list of module-level errors (file not found, parse errors, etc.) and a boolean indicating whether the module was **actually** (re)loaded.
    /// The boolean is used for change detection.
    pub fn load_module(
        &self,
        import_segments: &[&str],
        force: bool,
        import_str: Option<&str>,
    ) -> Vec<ModuleError> {
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

        let mut errors: Vec<ModuleError> = Vec::new();

        // Insert this module into the registry and extract imports so we can recursively load them.
        let imports: Vec<(Vec<String>, String)> = {
            let module_data_res: Result<ModuleData, Vec<ModuleError>> = ModuleDataTryBuilder {
                abs_path,
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
            }
            .try_build();

            let mut md = match module_data_res {
                Err(e) => return e,
                Ok(r) => r,
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
            let ss = segments.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
            match self.resolve_import(&ss, Some(&joined)) {
                Err(e) => errors.push(e),
                Ok((_joined, import_abs)) => {
                    if !import_abs.exists() {
                        errors.push(ModuleError {
                            module_path: module_path.clone(),
                            message: format!(
                                "Imported module '{}' not found at '{}'",
                                joined,
                                import_abs.display()
                            ),
                            location: None,
                        });
                        continue;
                    }
                    errors.extend(self.load_module(&ss, false, Some(&joined)));
                }
            }
        }

        errors
    }

    /// Builds a declaration cache from all modules in the registry.
    /// Maps declaration name → DeclarationInfo (including which module it came from).
    pub fn build_declaration_cache(&self) -> () {
        let modules = self.module_cache.read().unwrap();
        let mut cache = self.declaration_cache.write().unwrap();

        for (module_path, module_data) in modules.iter() {
            let ast = module_data.borrow_ast();
            for decl in ast {
                match decl {
                    Declaration::Import { .. } => {}
                    Declaration::TypeDecl { name, docs, .. } => {
                        cache.insert(
                            name.v.to_string(),
                            DeclarationInfo {
                                name: name.v.to_string(),
                                module_path: module_path.clone(),
                                abs_path: module_data.borrow_abs_path().clone(),
                                docs: docs.map(|d| d.to_string()),
                                line: name.l,
                                column: name.c,
                                name_len: name.v.len() as u32,
                            },
                        );
                    }
                }
            }
        }
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
        let canonical = root
            .join(&import_str)
            .with_added_extension(".xen")
            .canonicalize();

        match canonical {
            Ok(p) => Ok((import_str, p)),
            Err(e) => Err(ModuleError {
                module_path: import_str.clone(),
                message: format!("Cannot resolve import '{}': {}", import_str, e),
                location: None,
            }),
        }
    }

    pub fn purge(&self) {
        let mut reg = self.module_cache.write().unwrap();
        reg.clear();
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
}
