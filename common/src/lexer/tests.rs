#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::lexer::{Lexer, TokenVariant};
    use crate::test_strings::lexer as source;

    fn tok(src: &str) -> Vec<(TokenVariant, &str)> {
        Lexer::tokenize(src)
            .unwrap()
            .into_iter()
            .map(|(variant, data)| (variant, data.text(src)))
            .collect()
    }

    fn tok_pos(src: &str) -> Vec<(TokenVariant, &str, u32, u32)> {
        Lexer::tokenize(src)
            .unwrap()
            .into_iter()
            .map(|(variant, data)| (variant, data.text(src), data.line(src), data.column(src)))
            .collect()
    }

    fn tok_err(src: &str, message: &str) {
        let error = Lexer::tokenize(src).unwrap_err();
        assert!(
            error.message.contains(message),
            "expected error containing {message:?}, got {:?}",
            error.message
        );
    }

    #[test]
    fn empty_and_whitespace_sources_have_no_tokens() {
        for src in source::EMPTY_SOURCES {
            assert!(tok(src).is_empty());
        }
    }

    #[test]
    fn custom_blocks_preserve_opaque_contents_and_matching_hash_fences() {
        let src = "##java @Thing(\"#\");\r\n$invalid \\ text 😀 ### ##; type Next = string;";
        let tokens = Lexer::tokenize(src).unwrap();
        assert_eq!(tokens[0].0, TokenVariant::CustomBlock);
        assert_eq!(
            tokens[0].1.text(src),
            "##java @Thing(\"#\");\r\n$invalid \\ text 😀 ### ##"
        );
        assert_eq!(tokens[1].0, TokenVariant::Semicolon);
        assert_eq!(tokens[2].0, TokenVariant::Type);
        assert_eq!(tokens[2].1.line(src), 1);
        assert_eq!(
            tok("#sql SELECT * FROM users#"),
            vec![(TokenVariant::CustomBlock, "#sql SELECT * FROM users#")]
        );
    }

    #[test]
    fn custom_blocks_require_a_closing_hash_run_of_the_same_length() {
        for src in [
            "#java unfinished",
            "##java only a single #",
            "##java longer ###",
        ] {
            let error = Lexer::tokenize(src).unwrap_err();
            assert!(error.message.contains("Unterminated custom block"));
            assert_eq!(error.location.text(src), src);
        }
    }

    #[test]
    fn custom_block_edits_relex_from_the_block_boundary() {
        let old_source = "type Before = string; ##sql SELECT # old ##; type After = number;";
        let new_source = old_source.replace("# old", "# new 😀 ###");
        let old = Lexer::lex(old_source);
        let edited = Lexer::relex(old_source, &old, &new_source);
        let fresh = Lexer::lex(&new_source);
        assert!(edited.error.is_none());
        assert_eq!(edited.tokens, fresh.tokens);
        let (_, block) = edited
            .tokens
            .iter()
            .find(|(kind, _)| *kind == TokenVariant::CustomBlock)
            .unwrap();
        assert!(!edited
            .checkpoints
            .iter()
            .any(|(offset, _)| *offset > block.start as usize && *offset < block.end as usize));
    }

    #[test]
    fn current_single_character_tokens() {
        let variants = [
            TokenVariant::Colon,
            TokenVariant::Question,
            TokenVariant::Comma,
            TokenVariant::Semicolon,
            TokenVariant::At,
            TokenVariant::Minus,
            TokenVariant::Not,
            TokenVariant::Or,
            TokenVariant::And,
            TokenVariant::LCurly,
            TokenVariant::RCurly,
            TokenVariant::LBracket,
            TokenVariant::RBracket,
            TokenVariant::Gt,
            TokenVariant::Lt,
            TokenVariant::LParen,
            TokenVariant::RParen,
            TokenVariant::Eq,
        ];

        for (source, variant) in source::SINGLE_CHARS.iter().copied().zip(variants) {
            assert_eq!(tok(source), vec![(variant, source)]);
        }
    }

    #[test]
    fn words_keywords_and_positions() {
        assert_eq!(
            tok(source::WORDS),
            vec![
                (TokenVariant::Type, "type"),
                (TokenVariant::Identifier, "T_1"),
                (TokenVariant::Eq, "="),
                (TokenVariant::Set, "set"),
                (TokenVariant::Enum, "enum"),
                (TokenVariant::True, "true"),
                (TokenVariant::False, "false"),
                (TokenVariant::Identifier, "validator"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
        assert_eq!(
            tok_pos(source::POSITIONED_WORDS),
            vec![
                (TokenVariant::Identifier, "foo", 0, 0),
                (TokenVariant::Identifier, "bar", 1, 2),
            ]
        );
    }

    #[test]
    fn as_is_a_keyword_for_literal_casts() {
        assert_eq!(
            tok("1 as u64"),
            vec![
                (TokenVariant::Number, "1"),
                (TokenVariant::As, "as"),
                (TokenVariant::Identifier, "u64"),
            ]
        );
    }

    #[test]
    fn imports_lex_identifiers_and_namespace_separators() {
        assert_eq!(
            tok("pub import models::*;"),
            vec![
                (TokenVariant::Pub, "pub"),
                (TokenVariant::Import, "import"),
                (TokenVariant::Identifier, "models"),
                (TokenVariant::PathSeparator, "::"),
                (TokenVariant::Asterisk, "*"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
        assert_eq!(
            tok(source::IMPORT),
            vec![
                (TokenVariant::Import, "import"),
                (TokenVariant::Identifier, "foo2"),
                (TokenVariant::PathSeparator, "::"),
                (TokenVariant::Identifier, "bar_3"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
        assert_eq!(
            tok(source::EMPTY_IMPORT),
            vec![
                (TokenVariant::Import, "import"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
    }

    #[test]
    fn numbers_include_optional_minus_and_decimal_fraction() {
        assert_eq!(
            tok(source::NUMBERS),
            vec![
                (TokenVariant::Number, "0"),
                (TokenVariant::Number, "-42"),
                (TokenVariant::Number, "3.14"),
            ]
        );
        assert_eq!(
            tok(source::MINUS_IDENTIFIER),
            vec![
                (TokenVariant::Minus, "-"),
                (TokenVariant::Identifier, "value"),
            ]
        );
    }

    #[test]
    fn strings_regexes_and_comments() {
        assert_eq!(
            tok(source::DELIMITED),
            vec![
                (TokenVariant::String, "\"hello\""),
                (TokenVariant::Regex, "/a\\/b/"),
                (TokenVariant::Documentation, "/** docs */"),
            ]
        );
        assert!(tok(source::SKIPPED_COMMENTS).is_empty());
        assert!(tok(source::EMPTY_COMMENT).is_empty());
    }

    #[test]
    fn unterminated_delimited_tokens_report_errors() {
        tok_err(source::UNTERMINATED_STRING, source::STRING_ERROR);
        tok_err(source::UNTERMINATED_REGEX, source::REGEX_ERROR);
        tok_err(source::NEWLINE_REGEX, source::REGEX_ERROR);
        tok_err(source::UNTERMINATED_COMMENT, source::COMMENT_ERROR);
    }

    #[test]
    fn not_equal_is_one_token() {
        assert_eq!(tok(source::NOT_EQUAL), vec![(TokenVariant::Neq, "!=")]);
    }

    #[test]
    fn consumed_spans_stop_before_unconsumed_delimiters() {
        assert_eq!(
            tok(source::WORD_DELIMITER),
            vec![
                (TokenVariant::Identifier, "name"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
        assert_eq!(
            tok(source::NUMBER_DELIMITER),
            vec![
                (TokenVariant::Number, "-42"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
        assert_eq!(
            tok(source::IMPORT_DELIMITER),
            vec![
                (TokenVariant::Import, "import"),
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::PathSeparator, "::"),
                (TokenVariant::Identifier, "bar"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
    }

    #[test]
    fn consumed_spans_can_end_exactly_at_eof() {
        assert_eq!(
            tok(source::WORD_EOF),
            vec![(TokenVariant::Identifier, "name")]
        );
        assert_eq!(tok(source::NUMBER_EOF), vec![(TokenVariant::Number, "-42")]);
        assert_eq!(
            tok(source::IMPORT_EOF),
            vec![
                (TokenVariant::Import, "import"),
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::PathSeparator, "::"),
                (TokenVariant::Identifier, "bar"),
            ]
        );
    }

    #[test]
    fn delimited_token_spans_include_their_consumed_closer_only() {
        assert_eq!(
            tok(source::UTF8_DELIMITED),
            vec![
                (TokenVariant::String, "\"é\""),
                (TokenVariant::Semicolon, ";"),
                (TokenVariant::Regex, "/ø/"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
    }

    #[test]
    fn single_token_error_span_covers_one_utf8_character() {
        let error = Lexer::tokenize(source::UNKNOWN_UTF8).unwrap_err();
        assert_eq!(error.location.text(source::UNKNOWN_UTF8), "é");
        assert_eq!(error.location.column(source::UNKNOWN_UTF8), 0);
    }

    #[test]
    fn unknown_ascii_characters_report_the_current_character() {
        let error = Lexer::tokenize(source::UNKNOWN_ASCII).unwrap_err();
        assert_eq!(error.message, source::UNKNOWN_ERROR);
        assert_eq!(
            error.location.text(source::UNKNOWN_ASCII),
            source::UNKNOWN_ASCII
        );
    }
}

#[cfg(test)]
mod incremental {
    use crate::lexer::Lexer;
    #[test]
    fn edits_match_full_lexing_across_all_lexical_boundaries() {
        let source="/** docs\nmore */ import models::user::User; type A = { key: \"hello\nworld\", value: number } @match(/a+/); /* ignored */ ##java raw #\nextra##; type B = |A|number;";
        let old = Lexer::lex(source);
        assert!(old.error.is_none());
        for offset in source
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(source.len()))
        {
            for inserted in [
                "",
                "x",
                "\n",
                "\r\n",
                "\"",
                "/",
                "/*",
                "\u{e9}",
                "\u{1f600}",
            ] {
                for removed in [0, source[offset..].chars().next().map_or(0, char::len_utf8)] {
                    let replacement = format!(
                        "{}{}{}",
                        &source[..offset],
                        inserted,
                        &source[offset + removed..]
                    );
                    let actual = Lexer::relex(source, &old, &replacement);
                    let expected = Lexer::lex(&replacement);
                    assert_eq!(
                        actual.tokens, expected.tokens,
                        "edit {offset}, {inserted:?}, remove {removed}"
                    );
                    assert_eq!(
                        actual.error.as_ref().map(|e| (&e.message, e.location)),
                        expected.error.as_ref().map(|e| (&e.message, e.location))
                    );
                }
            }
        }
    }
    #[test]
    fn small_edit_converges_before_the_end_of_a_large_document() {
        let source = (0..1000)
            .map(|i| format!("type T{i} = number;\n"))
            .collect::<String>();
        let old = Lexer::lex(&source);
        let replacement = source.replacen("number", "string", 1);
        let actual = Lexer::relex(&source, &old, &replacement);
        assert_eq!(actual.tokens, Lexer::lex(&replacement).tokens);
        assert!(actual.scanned_bytes < 100);
    }
}
