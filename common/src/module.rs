use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::{fmt, fs};

use crate::lexer::Lexer;
use crate::parser::{Declaration, Parser};

/// A relative path from the workspace root, using '/' separators (e.g. "a/b").
/// This is the canonical key into the ModuleMap.
pub type ModulePath = String;

/// Information about a single module (one .xen file).
/// Owns the source text so that all borrows from tokens/ast remain valid.
pub struct ModuleData {
    /// The relative path from the workspace root (canonical key).
    pub path: ModulePath,
    /// The absolute filesystem path.
    pub abs_path: PathBuf,
    /// Owned source text — tokens and AST borrow from this.
    pub source: String,
}

impl fmt::Debug for ModuleData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleData")
            .field("path", &self.path)
            .field("abs_path", &self.abs_path)
            .field("source_len", &self.source.len())
            .finish()
    }
}

/// A cache entry for a declaration found during analysis.
/// Records which module the declaration lives in.
#[derive(Debug, Clone)]
pub struct DeclarationInfo {
    /// The name of the declared type.
    pub name: String,
    /// Which module (ModulePath) this declaration comes from.
    pub module_path: ModulePath,
    /// Documentation string, if any.
    pub docs: Option<String>,
}

/// The shared module map — designed to be wrapped in Arc<RwLock<>> for
/// future multi-threaded support.
#[derive(Debug, Default)]
pub struct ModuleRegistry {
    /// All loaded modules keyed by their relative path from workspace root.
    pub modules: HashMap<ModulePath, ModuleData>,
}

/// Thread-safe handle to a ModuleRegistry.
pub type SharedModuleRegistry = Arc<RwLock<ModuleRegistry>>;

/// Creates a new empty SharedModuleRegistry.
pub fn new_registry() -> SharedModuleRegistry {
    Arc::new(RwLock::new(ModuleRegistry::default()))
}

/// Errors that can occur during module loading.
#[derive(Debug, Clone)]
pub struct ModuleError {
    pub module_path: ModulePath,
    pub message: String,
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.module_path, self.message)
    }
}

/// Resolves an import path (e.g. `["a", "b"]`) relative to the directory
/// containing `current_file`, returning the canonical ModulePath and absolute
/// filesystem path.
///
/// - `import_segments`: the parsed path segments, e.g. `["a", "b"]` for `import a/b;`
/// - `current_file_abs`: absolute path of the file containing the import statement
/// - `workspace_root`: absolute path of the workspace root (folder of the entry file)
fn resolve_import(
    import_segments: &[&str],
    current_file_abs: &Path,
    workspace_root: &Path,
) -> Result<(ModulePath, PathBuf), ModuleError> {
    let current_dir = current_file_abs.parent().unwrap_or(workspace_root);
    let relative_from_current = import_segments.join("/");
    let abs_path = current_dir
        .join(format!("{}.xen", relative_from_current))
        .canonicalize()
        .or_else(|_| {
            // Try without canonicalize for better error messages
            Ok::<PathBuf, std::io::Error>(current_dir.join(format!("{}.xen", relative_from_current)))
        })
        .unwrap();

    // Compute the module path relative to the workspace root
    let module_path = abs_path
        .strip_prefix(workspace_root)
        .unwrap_or(&abs_path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");

    Ok((module_path, abs_path))
}

/// Recursively loads a .xen file and all its imports into the registry.
///
/// - `file_abs`: absolute path of the file to load
/// - `workspace_root`: absolute path of the workspace root
/// - `registry`: the shared module registry to populate
///
/// Returns a list of module-level errors (file not found, parse errors, etc.)
pub fn load_module(
    file_abs: &Path,
    workspace_root: &Path,
    registry: &SharedModuleRegistry,
) -> Vec<ModuleError> {
    let mut errors = Vec::new();

    // Compute module path
    let module_path = file_abs
        .strip_prefix(workspace_root)
        .unwrap_or(file_abs)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");

    // Check if already loaded
    {
        let reg = registry.read().unwrap();
        if reg.modules.contains_key(&module_path) {
            return errors;
        }
    }

    // Read file
    let source = match fs::read_to_string(file_abs) {
        Ok(s) => s,
        Err(e) => {
            errors.push(ModuleError {
                module_path: module_path.clone(),
                message: format!("Failed to read file '{}': {}", file_abs.display(), e),
            });
            return errors;
        }
    };

    // Collect imports from this file before inserting into registry.
    // We need to lex and parse to find import declarations.
    let import_paths: Vec<(Vec<String>, String)> = {
        match Lexer::tokenize(&source) {
            Err(e) => {
                errors.push(ModuleError {
                    module_path: module_path.clone(),
                    message: format!("Lexer error: {} at {}", e.message, e.location),
                });
                Vec::new()
            }
            Ok(tokens) => {
                let (ast, parse_errors) = Parser::parse(&tokens);
                for e in &parse_errors {
                    errors.push(ModuleError {
                        module_path: module_path.clone(),
                        message: format!("Parse error: {} at {}", e.message, e.location),
                    });
                }
                ast.iter()
                    .filter_map(|decl| match decl {
                        Declaration::Import { path, .. } => {
                            let owned_path: Vec<String> =
                                path.iter().map(|s| s.to_string()).collect();
                            let joined = path.join("/");
                            Some((owned_path, joined))
                        }
                        _ => None,
                    })
                    .collect()
            }
        }
    };

    // Insert this module into the registry
    {
        let mut reg = registry.write().unwrap();
        reg.modules.insert(
            module_path.clone(),
            ModuleData {
                path: module_path.clone(),
                abs_path: file_abs.to_path_buf(),
                source,
            },
        );
    }

    // Recursively load imports
    for (segments, _joined) in &import_paths {
        let segments_str: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        match resolve_import(&segments_str, file_abs, workspace_root) {
            Ok((_, import_abs)) => {
                if !import_abs.exists() {
                    errors.push(ModuleError {
                        module_path: module_path.clone(),
                        message: format!(
                            "Imported module '{}' not found at '{}'",
                            _joined,
                            import_abs.display()
                        ),
                    });
                    continue;
                }
                let sub_errors = load_module(&import_abs, workspace_root, registry);
                errors.extend(sub_errors);
            }
            Err(e) => errors.push(e),
        }
    }

    errors
}

/// Builds a declaration cache from all modules in the registry.
/// Maps declaration name → DeclarationInfo (including which module it came from).
pub fn build_declaration_cache(registry: &SharedModuleRegistry) -> HashMap<String, DeclarationInfo> {
    let reg = registry.read().unwrap();
    let mut cache: HashMap<String, DeclarationInfo> = HashMap::new();

    for (module_path, module_data) in &reg.modules {
        // Re-lex and parse to extract declarations
        if let Ok(tokens) = Lexer::tokenize(&module_data.source) {
            let (ast, _) = Parser::parse(&tokens);
            for decl in &ast {
                match decl {
                    Declaration::TypeDecl { name, docs, .. } => {
                        cache.insert(
                            name.v.to_string(),
                            DeclarationInfo {
                                name: name.v.to_string(),
                                module_path: module_path.clone(),
                                docs: docs.map(|d| d.to_string()),
                            },
                        );
                    }
                    Declaration::Import { .. } => {}
                }
            }
        }
    }

    cache
}

/// Convenience: load everything starting from an entry file path and return the registry.
pub fn load_workspace(entry_file: &Path) -> (SharedModuleRegistry, Vec<ModuleError>) {
    let workspace_root = entry_file.parent().unwrap_or(Path::new("."));
    let registry = new_registry();
    let errors = load_module(entry_file, workspace_root, &registry);
    (registry, errors)
}
