use std::hash::{DefaultHasher, Hash, Hasher};

use crate::TokenData;

/// Removes the comment boundaries '/**' and '*/', strips the conventional
/// leading ` * ` from continuation lines and trims surrounding blank space.
pub fn extract_documentation(token: &TokenData, source: &str) -> String {
    let text = token.text(source);
    let inner = &text[3..text.len() - 2];
    let lines: Vec<&str> = inner
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            match trimmed.strip_prefix('*') {
                Some(rest) => rest.strip_prefix(' ').unwrap_or(rest),
                None => trimmed,
            }
            .trim_end()
        })
        .collect();
    lines.join("\n").trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentation_strips_continuation_stars() {
        let source = "/**\n * First line.\n *   indented\n *\n * Last.\n */";
        let token = TokenData::new(0, source.len());
        assert_eq!(
            extract_documentation(&token, source),
            "First line.\n  indented\n\nLast."
        );
    }

    #[test]
    fn single_line_documentation_is_trimmed() {
        let source = "/** Only line. */";
        let token = TokenData::new(0, source.len());
        assert_eq!(extract_documentation(&token, source), "Only line.");
    }
}
