use std::hash::{DefaultHasher, Hash, Hasher};

use crate::TokenData;

/// Just slices the value of the token to remove the comment boundries '/**' and '*/',
/// and trims the result to remove any leading or trailing whitespace.
pub fn extract_documentation<'src>(token: &TokenData<'src>) -> &'src str {
    let len = token.v.len();
    token.v[3..len - 2].trim()
}

pub fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

/// Builds the relative path from one module to another, e.g. `./address` or
/// `../shared/address`. Targets that need an extension append their own.
pub fn relative_module_path(from_module_path: &str, to_module_path: &str) -> String {
    let mut from_dir = module_path_parts(from_module_path);
    from_dir.pop();

    let to_parts = module_path_parts(to_module_path);
    let common_len = from_dir
        .iter()
        .zip(&to_parts)
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative_parts = vec![".."; from_dir.len().saturating_sub(common_len)];
    relative_parts.extend(to_parts[common_len..].iter().copied());

    let path = relative_parts.join("/");
    if path.starts_with("..") {
        path
    } else {
        format!("./{path}")
    }
}

pub fn module_path_parts(module_path: &str) -> Vec<&str> {
    module_path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect()
}
