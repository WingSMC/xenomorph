// TODO: implement a more sophisticated formatter
pub fn format_xenomorph(text: &str) -> String {
    let mut result = String::new();
    let mut indent_level: u32 = 0;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        if trimmed.starts_with('}') || trimmed.starts_with(')') || trimmed.starts_with(']') {
            indent_level = indent_level.saturating_sub(1);
        }

        for _ in 0..indent_level {
            result.push_str("    ");
        }

        result.push_str(trimmed);
        result.push('\n');

        if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            indent_level += 1;
        }
    }

    result
}
