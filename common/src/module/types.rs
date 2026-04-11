use std::{fmt, path::PathBuf};

/// Errors that can occur during module loading.
#[derive(Debug, Clone)]
pub struct ModuleError {
    pub module_path: ModulePath,
    pub message: String,
    pub location: Option<(u32, u32, u32)>, // line, column, length
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.module_path, self.message)
    }
}

/// A relative path from the workspace root, using '/' separators (e.g. "a/b").
/// This is the canonical key into the ModuleMap.
pub type ModulePath = String;

/// A cache entry for a declaration found during analysis.
/// Records which module the declaration lives in.
#[derive(Debug, Clone)]
pub struct DeclarationInfo {
    /// The name of the declared type.
    pub name: String,
    /// Which module (ModulePath) this declaration comes from.
    pub module_path: ModulePath,
    /// The absolute filesystem path of the module.
    pub abs_path: PathBuf,
    /// Documentation string, if any.
    pub docs: Option<String>,
    /// Line number (0-indexed) of the declaration name in its file.
    pub line: u32,
    /// Column number (0-indexed) of the declaration name in its file.
    pub column: u32,
    /// Length of the declaration name.
    pub name_len: u32,
}
