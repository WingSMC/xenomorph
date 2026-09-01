use std::collections::{BTreeSet, HashSet};

use crate::config::{FormatterConfig, IndentKind, LineEnding};
use crate::lexer::{Lexer, Token, TokenVariant};
use crate::parser::{Declaration, Parser};

#[derive(Clone, Copy, Debug)]
struct FormatToken {
    variant: TokenVariant,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

#[derive(Clone, Copy, Debug)]
struct Delimiter {
    close: TokenVariant,
    core_root: bool,
    base_indent: usize,
}

/// Formats Xenomorph source without removing any existing line breaks.
///
/// Existing line boundaries are hard layout constraints. The formatter only
/// normalizes indentation and adds line breaks where declarations need to be
/// expanded.
pub fn format_xenomorph(source: &str, config: &FormatterConfig) -> String {
    let Ok(tokens) = Lexer::tokenize(source) else {
        let output_ending = output_line_ending(source, config.line_ending);
        let source = normalize_line_endings(source);
        return convert_line_endings(&source, output_ending);
    };
    let (ast, _) = Parser::parse(&tokens);
    format_xenomorph_with_syntax(source, &tokens, &ast, config)
}

/// Formats source using a previously produced token stream and AST.
///
/// This entry point is intended for language servers and other callers that
/// already retain parser results. It does not lex or parse the source again.
pub fn format_xenomorph_with_syntax(
    source: &str,
    tokens: &[Token<'_>],
    ast: &[Declaration<'_>],
    config: &FormatterConfig,
) -> String {
    let output_ending = output_line_ending(source, config.line_ending);
    let source = normalize_line_endings(source);
    let max_line_length = config.max_line_length.max(1);
    let indent_width = config.indent_width.max(1);

    let tokens = format_tokens(&source, tokens);
    let declarations = type_declaration_ranges(&tokens, ast);
    let roots = core_root_openers(&tokens, &declarations);
    let mut breaks = annotation_breaks(
        &source,
        &tokens,
        &declarations,
        max_line_length,
        indent_width,
    );
    breaks.extend(structural_breaks(
        &source,
        &tokens,
        &declarations,
        &breaks,
        max_line_length,
        indent_width,
    ));
    let trailing_commas = trailing_comma_insertions(&source, &tokens, &breaks);
    let wrapped = apply_insertions(&source, &breaks, &trailing_commas);
    let remapped_tokens = remap_token_lines(&source, &tokens, &breaks);
    let formatted = reindent(&wrapped, &remapped_tokens, &roots, config);

    convert_line_endings(&formatted, output_ending)
}

fn normalize_line_endings(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

fn output_line_ending(source: &str, configured: LineEnding) -> &'static str {
    match configured {
        LineEnding::Lf => "\n",
        LineEnding::Crlf => "\r\n",
        LineEnding::Auto if source.contains("\r\n") || source.contains('\r') => "\r\n",
        LineEnding::Auto => "\n",
    }
}

fn convert_line_endings(source: &str, line_ending: &str) -> String {
    if line_ending == "\n" {
        source.to_string()
    } else {
        source.replace('\n', line_ending)
    }
}

fn format_tokens(source: &str, tokens: &[Token<'_>]) -> Vec<FormatToken> {
    let line_starts = line_starts(source);
    tokens
        .iter()
        .map(|(variant, data)| {
            let start = byte_offset(source, &line_starts, data.l as usize, data.c as usize);
            FormatToken {
                variant: *variant,
                start,
                end: start + normalize_line_endings(data.v).len(),
                line: data.l as usize,
                column: data.c as usize,
            }
        })
        .collect()
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        source
            .match_indices('\n')
            .map(|(index, _)| index.saturating_add(1)),
    );
    starts
}

fn byte_offset(source: &str, starts: &[usize], line: usize, column: usize) -> usize {
    let start = starts.get(line).copied().unwrap_or(source.len());
    let end = source[start..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset);
    source[start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(offset, _)| start + offset)
}

fn structural_breaks(
    source: &str,
    tokens: &[FormatToken],
    declarations: &[(usize, usize)],
    annotation_breaks: &BTreeSet<usize>,
    max_line_length: usize,
    indent_width: usize,
) -> BTreeSet<usize> {
    let mut breaks = BTreeSet::new();

    for &(index, end) in declarations {
        let Some(eq) =
            (index..end).find(|candidate| tokens[*candidate].variant == TokenVariant::Eq)
        else {
            continue;
        };
        let core_start = eq + 1;
        if core_start >= end {
            continue;
        }

        let annotation_indices = top_level_annotations(tokens, core_start, end);
        let core_end = annotation_indices.first().copied().unwrap_or(end);
        let core_variant = tokens[core_start].variant;

        if matches!(core_variant, TokenVariant::Or | TokenVariant::And) {
            for operator in (core_start..core_end).filter(|candidate| {
                tokens[*candidate].variant == core_variant
                    && delimiter_depth(tokens, core_start, *candidate) == 0
            }) {
                insert_before_if_same_line(tokens, operator, &mut breaks);
            }
            continue;
        }

        let core_last = core_end.saturating_sub(1);
        let core_suffix_end = if annotation_indices.is_empty() {
            tokens[end].end
        } else {
            tokens[core_last].end
        };
        if !source_range_exceeds_margin(
            source,
            tokens[index].start,
            core_suffix_end,
            0,
            max_line_length,
            indent_width,
        ) {
            wrap_declaration_generics_if_needed(
                source,
                tokens,
                index,
                eq,
                max_line_length,
                indent_width,
                &mut breaks,
            );
            continue;
        }

        let mut wrapped = false;
        if let Some(root_open) = core_collection_opener(tokens, core_start, core_end) {
            if let Some(root_close) = matching_delimiter(tokens, root_open, core_end) {
                if root_close > root_open + 1 {
                    add_delimited_list_breaks(tokens, root_open, root_close, &mut breaks);
                    wrapped = true;
                }
            }
        }

        if !wrapped {
            if tokens[eq].line == tokens[core_start].line {
                breaks.insert(tokens[core_start].start);
            }

            if source_range_exceeds_margin(
                source,
                tokens[core_start].start,
                core_suffix_end,
                indent_width,
                max_line_length,
                indent_width,
            ) {
                if let Some(generic_open) = (core_start..core_end)
                    .find(|candidate| tokens[*candidate].variant == TokenVariant::Lt)
                {
                    if let Some(generic_close) = matching_delimiter(tokens, generic_open, core_end)
                    {
                        add_delimited_list_breaks(tokens, generic_open, generic_close, &mut breaks);
                    }
                }
            }
        }

        wrap_declaration_generics_if_needed(
            source,
            tokens,
            index,
            eq,
            max_line_length,
            indent_width,
            &mut breaks,
        );

        breaks.extend(annotation_breaks.iter().copied());
    }

    breaks
}

fn wrap_declaration_generics_if_needed(
    source: &str,
    tokens: &[FormatToken],
    declaration_start: usize,
    eq: usize,
    max_line_length: usize,
    indent_width: usize,
    breaks: &mut BTreeSet<usize>,
) {
    if !source_range_exceeds_margin(
        source,
        tokens[declaration_start].start,
        tokens[eq].end,
        0,
        max_line_length,
        indent_width,
    ) {
        return;
    }

    let Some(generic_open) =
        (declaration_start..eq).find(|candidate| tokens[*candidate].variant == TokenVariant::Lt)
    else {
        return;
    };
    if let Some(generic_close) = matching_delimiter(tokens, generic_open, eq) {
        add_delimited_list_breaks(tokens, generic_open, generic_close, breaks);
    }
}

fn annotation_breaks(
    source: &str,
    tokens: &[FormatToken],
    declarations: &[(usize, usize)],
    max_line_length: usize,
    indent_width: usize,
) -> BTreeSet<usize> {
    let starts = line_starts(source);
    let lines = source.split('\n').collect::<Vec<_>>();
    let mut breaks = BTreeSet::new();

    for &(declaration_start, end) in declarations {
        let Some(eq) = (declaration_start..end)
            .find(|candidate| tokens[*candidate].variant == TokenVariant::Eq)
        else {
            continue;
        };
        let core_start = eq + 1;
        let annotations = top_level_annotations(tokens, core_start, end);
        if annotations.is_empty() {
            continue;
        }

        let force_annotation_line = matches!(
            tokens.get(core_start).map(|token| token.variant),
            Some(TokenVariant::Or | TokenVariant::And)
        );
        let annotation_ranges = annotations
            .iter()
            .enumerate()
            .map(|(position, start)| {
                let boundary = annotations.get(position + 1).copied().unwrap_or(end);
                let last_token = boundary.saturating_sub(1).max(*start);
                (*start, last_token)
            })
            .collect::<Vec<_>>();

        let mut previous_end = annotations[0].saturating_sub(1);
        let mut current_length = visual_prefix_length(
            source,
            &starts,
            tokens[annotations[0]].line,
            tokens[annotations[0]].start,
            indent_width,
        );

        for (position, (annotation_start, annotation_end)) in
            annotation_ranges.iter().copied().enumerate()
        {
            let start_token = tokens[annotation_start];
            let end_token = tokens[annotation_end];
            let follows_on_same_line = tokens[previous_end].line == start_token.line;
            if !follows_on_same_line {
                current_length = visual_prefix_length(
                    source,
                    &starts,
                    start_token.line,
                    start_token.start,
                    indent_width,
                );
            }

            let gap_length = if follows_on_same_line {
                visual_length(
                    &source[tokens[previous_end].end..start_token.start],
                    indent_width,
                )
            } else {
                0
            };
            let annotation_text = &source[start_token.start..end_token.end];
            let first_line_length = visual_length(
                annotation_text.split('\n').next().unwrap_or_default(),
                indent_width,
            );
            let suffix_length =
                if position + 1 == annotation_ranges.len() && end_token.line == tokens[end].line {
                    visual_length(&source[end_token.end..tokens[end].end], indent_width)
                } else {
                    0
                };
            let projected_length = current_length
                .saturating_add(gap_length)
                .saturating_add(first_line_length)
                .saturating_add(suffix_length);
            let line_is_overlong = lines
                .get(start_token.line)
                .is_some_and(|line| visual_length(line, indent_width) > max_line_length);
            let starts_after_content = follows_on_same_line
                && !source[line_start(&starts, start_token.line)..start_token.start]
                    .trim()
                    .is_empty();
            let start_new_line = starts_after_content
                && ((position == 0 && (force_annotation_line || line_is_overlong))
                    || projected_length > max_line_length);

            if start_new_line {
                breaks.insert(start_token.start);
                current_length = indent_width.saturating_add(first_line_length);
            } else {
                current_length = current_length
                    .saturating_add(gap_length)
                    .saturating_add(first_line_length);
            }

            if annotation_text.contains('\n') {
                current_length = visual_length(
                    annotation_text.rsplit('\n').next().unwrap_or_default(),
                    indent_width,
                );
            }
            previous_end = annotation_end;
        }
    }

    breaks
}

fn type_declaration_ranges(tokens: &[FormatToken], ast: &[Declaration<'_>]) -> Vec<(usize, usize)> {
    ast.iter()
        .filter_map(|declaration| {
            let Declaration::Type { from, to, .. } = declaration else {
                return None;
            };
            let start = tokens.iter().position(|token| {
                token.variant == TokenVariant::Type
                    && token.line == from.l as usize
                    && token.column == from.c as usize
            })?;
            let last_syntax_token = tokens
                .iter()
                .rposition(|token| token.line == to.l as usize && token.column == to.c as usize)?;
            let end = (last_syntax_token..tokens.len())
                .find(|candidate| tokens[*candidate].variant == TokenVariant::Semicolon)?;
            Some((start, end))
        })
        .collect()
}

fn top_level_annotations(tokens: &[FormatToken], start: usize, end: usize) -> Vec<usize> {
    let mut stack = Vec::new();
    let mut annotations = Vec::new();

    for (index, token) in tokens.iter().enumerate().take(end).skip(start) {
        if token.variant == TokenVariant::At && stack.is_empty() {
            annotations.push(index);
        }
        update_delimiter_stack(&mut stack, token.variant, false, 0);
    }

    annotations
}

fn core_collection_opener(
    tokens: &[FormatToken],
    core_start: usize,
    core_end: usize,
) -> Option<usize> {
    match tokens.get(core_start)?.variant {
        TokenVariant::LCurly | TokenVariant::LBracket => Some(core_start),
        TokenVariant::Enum => (core_start + 1..core_end)
            .find(|candidate| tokens[*candidate].variant == TokenVariant::LCurly),
        TokenVariant::Set => (core_start + 1..core_end)
            .find(|candidate| tokens[*candidate].variant == TokenVariant::LBracket),
        _ => None,
    }
}

fn core_root_openers(tokens: &[FormatToken], declarations: &[(usize, usize)]) -> HashSet<usize> {
    let mut roots = HashSet::new();

    for &(index, end) in declarations {
        if let Some(eq) =
            (index..end).find(|candidate| tokens[*candidate].variant == TokenVariant::Eq)
        {
            let core_start = eq + 1;
            let annotations = top_level_annotations(tokens, core_start, end);
            let core_end = annotations.first().copied().unwrap_or(end);
            if let Some(root) = core_collection_opener(tokens, core_start, core_end) {
                roots.insert(tokens[root].start);
            }
        }
    }

    roots
}

fn matching_delimiter(tokens: &[FormatToken], open: usize, end: usize) -> Option<usize> {
    let expected = closing_variant(tokens.get(open)?.variant)?;
    let mut stack = Vec::new();

    for (index, token) in tokens.iter().enumerate().take(end).skip(open) {
        if let Some(close) = closing_variant(token.variant) {
            stack.push(close);
        } else if is_closing(token.variant) {
            if stack.pop() != Some(token.variant) {
                return None;
            }
            if stack.is_empty() && token.variant == expected {
                return Some(index);
            }
        }
    }

    None
}

fn delimiter_depth(tokens: &[FormatToken], start: usize, end: usize) -> usize {
    let mut stack = Vec::new();
    for token in tokens.iter().take(end).skip(start) {
        update_delimiter_stack(&mut stack, token.variant, false, 0);
    }
    stack.len()
}

fn add_delimited_list_breaks(
    tokens: &[FormatToken],
    open: usize,
    close: usize,
    breaks: &mut BTreeSet<usize>,
) {
    if tokens[open].line == tokens[open + 1].line {
        breaks.insert(tokens[open].end);
    }

    let mut stack = Vec::new();
    for index in open..close {
        let token = tokens[index];
        if token.variant == TokenVariant::Comma
            && stack.len() == 1
            && tokens
                .get(index + 1)
                .is_some_and(|next| next.line == token.line)
        {
            breaks.insert(token.end);
        }
        update_delimiter_stack(&mut stack, token.variant, false, 0);
    }

    let previous = tokens[close - 1];
    if previous.variant != TokenVariant::Comma && previous.line == tokens[close].line {
        breaks.insert(tokens[close].start);
    }
}

fn insert_before_if_same_line(tokens: &[FormatToken], index: usize, breaks: &mut BTreeSet<usize>) {
    if index > 0 && tokens[index - 1].line == tokens[index].line {
        breaks.insert(tokens[index].start);
    }
}

fn source_range_exceeds_margin(
    source: &str,
    start: usize,
    end: usize,
    first_line_prefix: usize,
    max_line_length: usize,
    indent_width: usize,
) -> bool {
    source[start..end]
        .split('\n')
        .enumerate()
        .any(|(line, content)| {
            visual_length(content, indent_width) + if line == 0 { first_line_prefix } else { 0 }
                > max_line_length
        })
}

fn trailing_comma_insertions(
    source: &str,
    tokens: &[FormatToken],
    breaks: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut commas = BTreeSet::new();

    for (open, token) in tokens.iter().enumerate() {
        if closing_variant(token.variant).is_none() {
            continue;
        }
        let Some(close) = matching_delimiter(tokens, open, tokens.len()) else {
            continue;
        };
        if close <= open + 1 || tokens[close - 1].variant == TokenVariant::Comma {
            continue;
        }

        let is_multiline = tokens[open].line != tokens[close].line
            || breaks
                .range(tokens[open].end..=tokens[close].start)
                .next()
                .is_some();
        if is_multiline && !source[tokens[close - 1].end..tokens[close].start].contains(',') {
            commas.insert(tokens[close - 1].end);
        }
    }

    commas
}

fn apply_insertions(source: &str, breaks: &BTreeSet<usize>, commas: &BTreeSet<usize>) -> String {
    if breaks.is_empty() && commas.is_empty() {
        return source.to_string();
    }

    let offsets = breaks.union(commas).copied().collect::<BTreeSet<_>>();
    let mut output = String::with_capacity(source.len() + breaks.len() + commas.len());
    let mut previous = 0;
    for offset in offsets {
        if offset > source.len() || offset < previous {
            continue;
        }
        output.push_str(&source[previous..offset]);
        if commas.contains(&offset) {
            output.push(',');
        }
        if breaks.contains(&offset)
            && !output.ends_with('\n')
            && !source[offset..].starts_with('\n')
        {
            output.push('\n');
        }
        previous = offset;
    }
    output.push_str(&source[previous..]);
    output
}

fn remap_token_lines(
    source: &str,
    tokens: &[FormatToken],
    breaks: &BTreeSet<usize>,
) -> Vec<FormatToken> {
    tokens
        .iter()
        .map(|token| FormatToken {
            line: token.line
                + breaks
                    .iter()
                    .filter(|offset| {
                        **offset <= token.start
                            && !source[..**offset].ends_with('\n')
                            && !source[**offset..].starts_with('\n')
                    })
                    .count(),
            ..*token
        })
        .collect()
}

fn reindent(
    source: &str,
    tokens: &[FormatToken],
    roots: &HashSet<usize>,
    config: &FormatterConfig,
) -> String {
    let indent_unit = match config.indent_kind {
        IndentKind::Space => " ".repeat(config.indent_width.max(1)),
        IndentKind::Tab => "\t".to_string(),
    };
    let lines = source.split('\n').collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len());
    let mut stack: Vec<Delimiter> = Vec::new();
    let mut token_index = 0;
    let mut in_type_declaration = false;
    let mut after_equals = false;

    for (line_index, line) in lines.iter().enumerate() {
        let first_token = token_index;
        while token_index < tokens.len() && tokens[token_index].line == line_index {
            token_index += 1;
        }
        let line_tokens = &tokens[first_token..token_index];

        let mut indent_level = stack
            .last()
            .map_or(0, |delimiter| delimiter.base_indent + 1);
        let mut leading_depth = stack.len();
        let mut closes_core_root = false;
        for token in line_tokens {
            let Some(last) = leading_depth
                .checked_sub(1)
                .and_then(|index| stack.get(index))
            else {
                break;
            };
            if token.variant != last.close {
                break;
            }
            closes_core_root |= last.core_root;
            indent_level = last.base_indent;
            leading_depth -= 1;
        }

        let starts_type = line_tokens
            .iter()
            .any(|token| token.variant == TokenVariant::Type);
        let starts_semicolon = line_tokens
            .first()
            .is_some_and(|token| token.variant == TokenVariant::Semicolon);
        let continuation = in_type_declaration
            && after_equals
            && leading_depth == 0
            && !closes_core_root
            && !starts_type
            && !starts_semicolon;
        indent_level = indent_level.max(usize::from(continuation));
        let trimmed = line.trim();

        if !trimmed.is_empty() {
            output.push_str(&indent_unit.repeat(indent_level));
            output.push_str(&normalize_operator_prefix(trimmed));
        }
        if line_index + 1 < lines.len() {
            output.push('\n');
        }

        for token in line_tokens {
            if token.variant == TokenVariant::Type && stack.is_empty() {
                in_type_declaration = true;
                after_equals = false;
            }
            if token.variant == TokenVariant::Eq && in_type_declaration && stack.is_empty() {
                after_equals = true;
            }
            let delimiter_indent = stack
                .last()
                .map_or(indent_level, |delimiter| delimiter.base_indent + 1);
            update_delimiter_stack(
                &mut stack,
                token.variant,
                roots.contains(&token.start),
                delimiter_indent,
            );
            if token.variant == TokenVariant::Semicolon && stack.is_empty() {
                in_type_declaration = false;
                after_equals = false;
            }
        }
    }

    output
}

fn normalize_operator_prefix(line: &str) -> String {
    let mut characters = line.chars();
    let Some(operator @ ('|' | '&')) = characters.next() else {
        return line.to_string();
    };
    let remainder = characters.as_str().trim_start();
    if remainder.is_empty() {
        operator.to_string()
    } else {
        format!("{operator} {remainder}")
    }
}

fn update_delimiter_stack(
    stack: &mut Vec<Delimiter>,
    variant: TokenVariant,
    core_root: bool,
    base_indent: usize,
) {
    if let Some(close) = closing_variant(variant) {
        stack.push(Delimiter {
            close,
            core_root,
            base_indent,
        });
    } else if is_closing(variant)
        && stack
            .last()
            .is_some_and(|delimiter| delimiter.close == variant)
    {
        stack.pop();
    }
}

fn closing_variant(variant: TokenVariant) -> Option<TokenVariant> {
    match variant {
        TokenVariant::LParen => Some(TokenVariant::RParen),
        TokenVariant::LCurly => Some(TokenVariant::RCurly),
        TokenVariant::LBracket => Some(TokenVariant::RBracket),
        TokenVariant::Lt => Some(TokenVariant::Gt),
        _ => None,
    }
}

fn is_closing(variant: TokenVariant) -> bool {
    matches!(
        variant,
        TokenVariant::RParen | TokenVariant::RCurly | TokenVariant::RBracket | TokenVariant::Gt
    )
}

fn line_start(starts: &[usize], line: usize) -> usize {
    starts.get(line).copied().unwrap_or_default()
}

fn visual_prefix_length(
    source: &str,
    starts: &[usize],
    line: usize,
    end: usize,
    indent_width: usize,
) -> usize {
    visual_length(&source[line_start(starts, line)..end], indent_width)
}

fn visual_length(text: &str, indent_width: usize) -> usize {
    let tab_width = indent_width.max(1);
    text.chars().fold(0, |column, character| {
        if character == '\t' {
            column + tab_width - (column % tab_width)
        } else {
            column + 1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatter(max_line_length: usize) -> FormatterConfig {
        FormatterConfig {
            max_line_length,
            ..FormatterConfig::default()
        }
    }

    #[test]
    fn puts_every_sum_member_and_its_annotations_on_continuation_lines() {
        let source = "type UNameT1 = string @match(/^[a-zA-Z0-9_]{0,20}$/);\ntype UNameT2 = string @match(/^_[a-zA-Z0-9$]{0,19}$/);\ntype UName =| UNameT1| UNameT2 @maxlen(20);";
        let expected = "type UNameT1 = string @match(/^[a-zA-Z0-9_]{0,20}$/);\ntype UNameT2 = string @match(/^_[a-zA-Z0-9$]{0,19}$/);\ntype UName =\n    | UNameT1\n    | UNameT2\n    @maxlen(20);";

        assert_eq!(format_xenomorph(source, &formatter(100)), expected);
    }

    #[test]
    fn cached_syntax_entry_point_matches_the_standalone_formatter() {
        let source = "type Choice = | First | Second @maxlen(20);";
        let tokens = Lexer::tokenize(source).expect("fixture should tokenize");
        let (ast, diagnostics) = Parser::parse(&tokens);
        assert!(diagnostics.is_empty(), "fixture should parse cleanly");

        assert_eq!(
            format_xenomorph_with_syntax(source, &tokens, &ast, &formatter(100)),
            format_xenomorph(source, &formatter(100))
        );
    }

    #[test]
    fn puts_every_intersection_member_on_its_own_line() {
        let source = "type Admin = & User<string> & Audited & Enabled;";
        let expected = "type Admin =\n    & User<string>\n    & Audited\n    & Enabled;";

        assert_eq!(format_xenomorph(source, &formatter(100)), expected);
    }

    #[test]
    fn wraps_struct_tuple_and_enum_members_when_they_exceed_the_margin() {
        let source = "type Person = { firstName: string, lastName: string, age: u8 };\ntype Pair = [VeryLongFirstType, VeryLongSecondType];\ntype Role = enum { Administrator: i8, StandardUser: i8 };";
        let expected = "type Person = {\n    firstName: string,\n    lastName: string,\n    age: u8,\n};\ntype Pair = [\n    VeryLongFirstType,\n    VeryLongSecondType,\n];\ntype Role = enum {\n    Administrator: i8,\n    StandardUser: i8,\n};";

        assert_eq!(format_xenomorph(source, &formatter(40)), expected);
    }

    #[test]
    fn keeps_generics_inline_when_annotations_are_the_only_overlong_component() {
        let source = "type Gt0<T: Numeric> = T @min(1);\ntype BigInt<T: Numeric> = Gt0<T>[] @minlen(1) @Lombok(Getter);";
        let expected = "type Gt0<T: Numeric> = T\n    @min(1);\ntype BigInt<T: Numeric> =\n    Gt0<T>[]\n    @minlen(1)\n    @Lombok(Getter);";

        assert_eq!(format_xenomorph(source, &formatter(30)), expected);
    }

    #[test]
    fn wraps_generics_last_with_indentation_and_trailing_commas() {
        let source = "type ExtremelyLongDeclarationName<FirstType: Numeric, SecondType: Numeric> = ExtremelyLongContainer<FirstType, SecondType>;";
        let expected = "type ExtremelyLongDeclarationName<\n    FirstType: Numeric,\n    SecondType: Numeric,\n> =\n    ExtremelyLongContainer<\n        FirstType,\n        SecondType,\n    >;";

        assert_eq!(format_xenomorph(source, &formatter(30)), expected);
    }

    #[test]
    fn keeps_annotation_parameters_together_and_wraps_annotation_groups() {
        let source = "type Name = string @match(/^[A-Z][a-z]+$/) @minlen(2) @maxlen(40);";
        let expected =
            "type Name = string\n    @match(/^[A-Z][a-z]+$/)\n    @minlen(2) @maxlen(40);";

        assert_eq!(format_xenomorph(source, &formatter(35)), expected);
    }

    #[test]
    fn preserves_all_existing_line_breaks_and_annotation_separation() {
        let source = "type Name = string\n\n    @minlen(2)\n    @maxlen(40);\n";
        let formatted = format_xenomorph(source, &formatter(100));

        assert_eq!(formatted, source);
        assert_eq!(
            formatted.matches('\n').count(),
            source.matches('\n').count()
        );
    }

    #[test]
    fn supports_tab_indentation_and_crlf_output() {
        let config = FormatterConfig {
            indent_kind: IndentKind::Tab,
            indent_width: 4,
            max_line_length: 80,
            line_ending: LineEnding::Crlf,
        };

        assert_eq!(
            format_xenomorph("type Choice = | A | B;", &config),
            "type Choice =\r\n\t| A\r\n\t| B;"
        );
    }

    #[test]
    fn auto_line_endings_preserve_crlf() {
        let config = FormatterConfig {
            line_ending: LineEnding::Auto,
            ..formatter(80)
        };

        assert_eq!(
            format_xenomorph("type Choice = | A | B;\r\n", &config),
            "type Choice =\r\n    | A\r\n    | B;\r\n"
        );
    }
}
