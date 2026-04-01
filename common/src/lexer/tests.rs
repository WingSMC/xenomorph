#[cfg(test)]
mod tests {
    use crate::lexer::{Lexer, TokenVariant};

    // ── Helpers ──────────────────────────────────────────────────────

    /// Tokenize and return vec of (variant, value_str) for easy assertion.
    fn tok(src: &str) -> Vec<(TokenVariant, &str)> {
        Lexer::tokenize(src)
            .unwrap()
            .into_iter()
            .map(|(v, td)| (v, td.v))
            .collect()
    }

    /// Tokenize and return vec of (variant, value, line, column).
    fn tok_pos(src: &str) -> Vec<(TokenVariant, &str, u32, u32)> {
        Lexer::tokenize(src)
            .unwrap()
            .into_iter()
            .map(|(v, td)| (v, td.v, td.l, td.c))
            .collect()
    }

    /// Expect a lexer error containing `msg_fragment`.
    fn tok_err(src: &str, msg_fragment: &str) {
        let err = Lexer::tokenize(src).unwrap_err();
        assert!(
            err.message.contains(msg_fragment),
            "Expected error containing '{}', got '{}'",
            msg_fragment,
            err.message
        );
    }

    // ── Empty / whitespace-only ─────────────────────────────────────

    #[test]
    fn empty_source() {
        assert!(tok("").is_empty());
    }

    #[test]
    fn whitespace_only() {
        let cases = [" ", "  ", "\t", "\n", "\r\n", "  \t \n \r\n  "];
        for src in cases {
            assert!(tok(src).is_empty(), "Expected empty for {:?}", src);
        }
    }

    // ── Single-character token matrix ───────────────────────────────

    #[test]
    fn single_char_tokens() {
        let matrix: &[(&str, TokenVariant)] = &[
            ("@", TokenVariant::At),
            (":", TokenVariant::Colon),
            ("$", TokenVariant::Dollar),
            ("|", TokenVariant::Or),
            ("&", TokenVariant::And),
            ("(", TokenVariant::LParen),
            (")", TokenVariant::RParen),
            (",", TokenVariant::Comma),
            ("{", TokenVariant::LCurly),
            ("}", TokenVariant::RCurly),
            ("[", TokenVariant::LBracket),
            ("]", TokenVariant::RBracket),
            (">", TokenVariant::Gt),
            (";", TokenVariant::Semicolon),
            ("+", TokenVariant::Plus),
            ("-", TokenVariant::Minus),
            ("*", TokenVariant::Asterix),
            ("^", TokenVariant::Caret),
            ("=", TokenVariant::Eq),
            ("\\", TokenVariant::Backslash),
        ];

        for &(src, expected_variant) in matrix {
            let tokens = tok(src);
            assert_eq!(
                tokens.len(),
                1,
                "Expected 1 token for '{}', got {:?}",
                src,
                tokens
            );
            assert_eq!(tokens[0].0, expected_variant, "Wrong variant for '{}'", src);
            assert_eq!(tokens[0].1, src, "Wrong value for '{}'", src);
        }
    }

    // ── Keyword matrix ──────────────────────────────────────────────

    #[test]
    fn keywords() {
        let matrix: &[(&str, TokenVariant)] = &[
            ("type", TokenVariant::Type),
            ("import", TokenVariant::Import),
            ("set", TokenVariant::Set),
            ("enum", TokenVariant::Enum),
            ("true", TokenVariant::True),
            ("false", TokenVariant::False),
        ];

        for &(src, expected_variant) in matrix {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for '{}'", src);
            assert_eq!(tokens[0].0, expected_variant, "Wrong variant for '{}'", src);
            assert_eq!(tokens[0].1, src);
        }
    }

    // ── Identifiers ─────────────────────────────────────────────────

    #[test]
    fn identifiers() {
        let cases = [
            "foo",
            "Bar",
            "_x",
            "_",
            "__double__",
            "a1",
            "hello_world",
            "CamelCase123",
            "type_extended",   // starts with keyword prefix but not exact
            "important",       // starts with "import" prefix
            "setting",         // starts with "set" prefix
            "enumerate",       // starts with "enum" prefix
            "trueish",         // starts with "true" prefix
            "falsehood",       // starts with "false" prefix
        ];
        for src in cases {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for '{}'", src);
            assert_eq!(
                tokens[0].0,
                TokenVariant::Identifier,
                "Wrong variant for '{}'",
                src
            );
            assert_eq!(tokens[0].1, src);
        }
    }

    // ── Numbers ─────────────────────────────────────────────────────

    #[test]
    fn integer_numbers() {
        let cases = ["0", "1", "42", "100", "999999"];
        for src in cases {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for '{}'", src);
            assert_eq!(tokens[0].0, TokenVariant::Number, "Wrong variant for '{}'", src);
            assert_eq!(tokens[0].1, src);
        }
    }

    #[test]
    fn float_numbers() {
        let cases = ["1.0", "3.14", "0.5", "100.001"];
        for src in cases {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for '{}'", src);
            assert_eq!(tokens[0].0, TokenVariant::Number, "Wrong variant for '{}'", src);
            assert_eq!(tokens[0].1, src);
        }
    }

    #[test]
    fn number_before_range() {
        // `10..` should parse as Number("10") Range("..")
        let tokens = tok("10..20");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (TokenVariant::Number, "10"));
        assert_eq!(tokens[1], (TokenVariant::Range, ".."));
        assert_eq!(tokens[2], (TokenVariant::Number, "20"));
    }

    #[test]
    fn number_before_dot_non_digit() {
        // `10.x` → Number("10") Dot(".") Identifier("x")
        let tokens = tok("10.x");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (TokenVariant::Number, "10"));
        assert_eq!(tokens[1], (TokenVariant::Dot, "."));
        assert_eq!(tokens[2], (TokenVariant::Identifier, "x"));
    }

    // ── Strings ─────────────────────────────────────────────────────

    #[test]
    fn valid_strings() {
        let matrix: &[(&str, &str)] = &[
            (r#""""#, r#""""#),              // empty string
            (r#""hello""#, r#""hello""#),
            (r#""with spaces""#, r#""with spaces""#),
            (r#""line\nbreak""#, r#""line\nbreak""#),  // backslash-n in string (raw)
        ];
        for &(src, expected_value) in matrix {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for {}", src);
            assert_eq!(tokens[0].0, TokenVariant::String, "Wrong variant for {}", src);
            assert_eq!(tokens[0].1, expected_value);
        }
    }

    #[test]
    fn unterminated_string() {
        tok_err(r#""hello"#, "String not terminated");
    }

    #[test]
    fn unterminated_string_empty() {
        tok_err(r#"""#, "String not terminated");
    }

    // ── Regex ───────────────────────────────────────────────────────

    #[test]
    fn valid_regex() {
        let matrix: &[(&str, &str)] = &[
            ("/abc/", "/abc/"),
            ("/[0-9]+/", "/[0-9]+/"),
            (r"/escaped\//", r"/escaped\//"),           // escaped slash inside
            (r"/double\\\//", r"/double\\\//"),         // double backslash then escaped slash
        ];
        for &(src, expected_value) in matrix {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for {}", src);
            assert_eq!(tokens[0].0, TokenVariant::Regex, "Wrong variant for {}", src);
            assert_eq!(tokens[0].1, expected_value);
        }
    }

    #[test]
    fn malformed_regex_newline() {
        // Regex broken by newline before closing slash
        tok_err("/abc\n/", "Malformed regex");
    }

    #[test]
    fn malformed_regex_unterminated() {
        // Never closed
        tok_err("/abc", "Malformed regex");
    }

    // ── Not / Neq ───────────────────────────────────────────────────

    #[test]
    fn not_and_neq() {
        let matrix: &[(&str, TokenVariant, &str)] = &[
            ("!", TokenVariant::Not, "!"),
            ("!=", TokenVariant::Neq, "!="),
        ];
        for &(src, expected_variant, expected_value) in matrix {
            let tokens = tok(src);
            assert_eq!(tokens.len(), 1, "Expected 1 token for '{}'", src);
            assert_eq!(tokens[0].0, expected_variant, "Wrong variant for '{}'", src);
            assert_eq!(tokens[0].1, expected_value);
        }
    }

    // ── Dot / Range / Lt / SymmDiff matrix ──────────────────────────

    #[test]
    fn dot_range_lt_symmdiff() {
        let matrix: &[(&str, Vec<(TokenVariant, &str)>)] = &[
            (".", vec![(TokenVariant::Dot, ".")]),
            ("..", vec![(TokenVariant::Range, "..")]),
            (".<", vec![(TokenVariant::Range, ".<")]),
            ("<.", vec![(TokenVariant::Range, "<.")]),
            ("<.<", vec![(TokenVariant::Range, "<.<")]),
            ("<>", vec![(TokenVariant::SymmDiff, "<>")]),
            ("<", vec![(TokenVariant::Lt, "<")]),
        ];

        for (src, expected) in matrix {
            let tokens = tok(src);
            assert_eq!(
                tokens.len(),
                expected.len(),
                "Wrong token count for '{}'",
                src
            );
            for (i, (ev, eval)) in expected.iter().enumerate() {
                assert_eq!(tokens[i].0, *ev, "Wrong variant at pos {} for '{}'", i, src);
                assert_eq!(tokens[i].1, *eval, "Wrong value at pos {} for '{}'", i, src);
            }
        }
    }

    // ── Comments ────────────────────────────────────────────────────

    #[test]
    fn line_comment_skipped() {
        let cases = [
            ("// this is a comment\n", vec![]),
            ("// comment", vec![]),  // EOF terminates comment too
            ("foo // inline\nbar", vec![
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::Identifier, "bar"),
            ]),
        ];
        for (src, expected) in cases {
            let tokens = tok(src);
            assert_eq!(tokens, expected, "Mismatch for {:?}", src);
        }
    }

    #[test]
    fn block_comment_skipped() {
        let cases = [
            ("/* block */", vec![]),
            ("/* multi\nline */", vec![]),
            ("foo /* comment */ bar", vec![
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::Identifier, "bar"),
            ]),
        ];
        for (src, expected) in cases {
            let tokens = tok(src);
            assert_eq!(tokens, expected, "Mismatch for {:?}", src);
        }
    }

    #[test]
    fn empty_block_comment_skipped() {
        // /**/ is a special case – empty block
        assert!(tok("/**/").is_empty());
    }

    #[test]
    fn doc_comment() {
        let tokens = tok("/** hello */");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Documentation);
        assert_eq!(tokens[0].1, "/** hello */");
    }

    #[test]
    fn doc_comment_multiline() {
        let src = "/** line1\n * line2\n */";
        let tokens = tok(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Documentation);
        assert_eq!(tokens[0].1, src);
    }

    #[test]
    fn doc_comment_with_star_inside() {
        // Make sure a stray * in the doc doesn't close early
        let src = "/** a * b */";
        let tokens = tok(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Documentation);
    }

    #[test]
    fn unterminated_block_comment() {
        tok_err("/* oops", "Comment not terminated");
    }

    #[test]
    fn unterminated_doc_comment() {
        tok_err("/** oops", "Comment not terminated");
    }

    // ── Slash context (import paths) ────────────────────────────────

    #[test]
    fn slash_in_import_path() {
        // import foo/bar => Import Identifier Slash Identifier
        let tokens = tok("import foo/bar");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].0, TokenVariant::Import);
        assert_eq!(tokens[1].0, TokenVariant::Identifier);
        assert_eq!(tokens[1].1, "foo");
        assert_eq!(tokens[2].0, TokenVariant::Slash);
        assert_eq!(tokens[2].1, "/");
        assert_eq!(tokens[3].0, TokenVariant::Identifier);
        assert_eq!(tokens[3].1, "bar");
    }

    #[test]
    fn chained_slash_in_import_path() {
        // import a/b/c => Import Identifier Slash Identifier Slash Identifier
        let tokens = tok("import a/b/c");
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].0, TokenVariant::Import);
        assert_eq!(tokens[1], (TokenVariant::Identifier, "a"));
        assert_eq!(tokens[2], (TokenVariant::Slash, "/"));
        assert_eq!(tokens[3], (TokenVariant::Identifier, "b"));
        assert_eq!(tokens[4], (TokenVariant::Slash, "/"));
        assert_eq!(tokens[5], (TokenVariant::Identifier, "c"));
    }

    #[test]
    fn slash_not_in_import_is_regex() {
        // Just `foo /bar/` — not import context → regex
        let tokens = tok("foo /bar/");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo"));
        assert_eq!(tokens[1].0, TokenVariant::Regex);
    }

    #[test]
    fn slash_after_standalone_identifier_is_regex() {
        // Only one token before `/`, so slash_context = false → regex
        let tokens = tok("x /y/");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "x"));
        assert_eq!(tokens[1].0, TokenVariant::Regex);
    }

    #[test]
    fn slash_at_start_is_regex() {
        let tokens = tok("/hello/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Regex);
    }

    // ── Unrecognized token ──────────────────────────────────────────

    #[test]
    fn unrecognized_char() {
        let bad_chars = ['#', '~', '`', '?', '%'];
        for c in bad_chars {
            let src = c.to_string();
            tok_err(&src, "Token not recognized");
        }
    }

    // ── Line / column tracking ──────────────────────────────────────

    #[test]
    fn position_tracking_single_line() {
        // "foo bar"  → foo at (0,0), bar at (0,4)
        let tokens = tok_pos("foo bar");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo", 0, 0));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "bar", 0, 4));
    }

    #[test]
    fn position_tracking_multiline() {
        let src = "foo\nbar\nbaz";
        let tokens = tok_pos(src);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo", 0, 0));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "bar", 1, 0));
        assert_eq!(tokens[2], (TokenVariant::Identifier, "baz", 2, 0));
    }

    #[test]
    fn position_tracking_with_indent() {
        let src = "  foo";
        let tokens = tok_pos(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo", 0, 2));
    }

    #[test]
    fn position_tracking_after_comment() {
        let src = "// comment\nfoo";
        let tokens = tok_pos(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo", 1, 0));
    }

    // ── Composite / integration scenarios ───────────────────────────

    #[test]
    fn type_declaration() {
        let src = r#"type Foo = string;"#;
        let tokens = tok(src);
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Type, "type"),
                (TokenVariant::Identifier, "Foo"),
                (TokenVariant::Eq, "="),
                (TokenVariant::Identifier, "string"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
    }

    #[test]
    fn annotated_field() {
        let src = "@min(1) @max(100)";
        let tokens = tok(src);
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::At, "@"),
                (TokenVariant::Identifier, "min"),
                (TokenVariant::LParen, "("),
                (TokenVariant::Number, "1"),
                (TokenVariant::RParen, ")"),
                (TokenVariant::At, "@"),
                (TokenVariant::Identifier, "max"),
                (TokenVariant::LParen, "("),
                (TokenVariant::Number, "100"),
                (TokenVariant::RParen, ")"),
            ]
        );
    }

    #[test]
    fn struct_with_fields() {
        let src = "type Point = {\n  x: number,\n  y: number\n};";
        let tokens = tok(src);
        let variants: Vec<TokenVariant> = tokens.iter().map(|t| t.0).collect();
        assert_eq!(
            variants,
            vec![
                TokenVariant::Type,
                TokenVariant::Identifier, // Point
                TokenVariant::Eq,
                TokenVariant::LCurly,
                TokenVariant::Identifier, // x
                TokenVariant::Colon,
                TokenVariant::Identifier, // number
                TokenVariant::Comma,
                TokenVariant::Identifier, // y
                TokenVariant::Colon,
                TokenVariant::Identifier, // number
                TokenVariant::RCurly,
                TokenVariant::Semicolon,
            ]
        );
    }

    #[test]
    fn union_type() {
        let src = "type AB = A | B;";
        let tokens = tok(src);
        let variants: Vec<TokenVariant> = tokens.iter().map(|t| t.0).collect();
        assert_eq!(
            variants,
            vec![
                TokenVariant::Type,
                TokenVariant::Identifier,
                TokenVariant::Eq,
                TokenVariant::Identifier,
                TokenVariant::Or,
                TokenVariant::Identifier,
                TokenVariant::Semicolon,
            ]
        );
    }

    #[test]
    fn intersection_type() {
        let src = "type AB = A & B;";
        let tokens = tok(src);
        let variants: Vec<TokenVariant> = tokens.iter().map(|t| t.0).collect();
        assert_eq!(
            variants,
            vec![
                TokenVariant::Type,
                TokenVariant::Identifier,
                TokenVariant::Eq,
                TokenVariant::Identifier,
                TokenVariant::And,
                TokenVariant::Identifier,
                TokenVariant::Semicolon,
            ]
        );
    }

    #[test]
    fn set_declaration() {
        let src = r#"type Colors = set { "red", "blue" };"#;
        let tokens = tok(src);
        let variants: Vec<TokenVariant> = tokens.iter().map(|t| t.0).collect();
        assert_eq!(
            variants,
            vec![
                TokenVariant::Type,
                TokenVariant::Identifier,
                TokenVariant::Eq,
                TokenVariant::Set,
                TokenVariant::LCurly,
                TokenVariant::String,
                TokenVariant::Comma,
                TokenVariant::String,
                TokenVariant::RCurly,
                TokenVariant::Semicolon,
            ]
        );
    }

    #[test]
    fn enum_declaration() {
        let src = "type Dir = enum { N, S, E, W };";
        let tokens = tok(src);
        let variants: Vec<TokenVariant> = tokens.iter().map(|t| t.0).collect();
        assert_eq!(
            variants,
            vec![
                TokenVariant::Type,
                TokenVariant::Identifier,
                TokenVariant::Eq,
                TokenVariant::Enum,
                TokenVariant::LCurly,
                TokenVariant::Identifier,
                TokenVariant::Comma,
                TokenVariant::Identifier,
                TokenVariant::Comma,
                TokenVariant::Identifier,
                TokenVariant::Comma,
                TokenVariant::Identifier,
                TokenVariant::RCurly,
                TokenVariant::Semicolon,
            ]
        );
    }

    #[test]
    fn regex_annotation() {
        let src = r#"@pattern(/^[a-z]+$/)"#;
        let tokens = tok(src);
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[0], (TokenVariant::At, "@"));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "pattern"));
        assert_eq!(tokens[2], (TokenVariant::LParen, "("));
        assert_eq!(tokens[3].0, TokenVariant::Regex);
        assert_eq!(tokens[4], (TokenVariant::RParen, ")"));
    }

    #[test]
    fn range_expression() {
        let src = "1..10";
        let tokens = tok(src);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (TokenVariant::Number, "1"));
        assert_eq!(tokens[1], (TokenVariant::Range, ".."));
        assert_eq!(tokens[2], (TokenVariant::Number, "10"));
    }

    #[test]
    fn import_with_semicolon() {
        let src = "import foo/bar;";
        let tokens = tok(src);
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Import, "import"),
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::Slash, "/"),
                (TokenVariant::Identifier, "bar"),
                (TokenVariant::Semicolon, ";"),
            ]
        );
    }

    #[test]
    fn dollar_identifier() {
        let src = "$ref";
        let tokens = tok(src);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Dollar, "$"));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "ref"));
    }

    #[test]
    fn boolean_values() {
        let tokens = tok("true false");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::True, "true"),
                (TokenVariant::False, "false"),
            ]
        );
    }

    #[test]
    fn caret_and_asterisk_operators() {
        let tokens = tok("^ *");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Caret, "^"),
                (TokenVariant::Asterix, "*"),
            ]
        );
    }

    #[test]
    fn gt_lt_comparison() {
        let tokens = tok("> <");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Gt, ">"),
                (TokenVariant::Lt, "<"),
            ]
        );
    }

    #[test]
    fn brackets() {
        let tokens = tok("[](){}");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::LBracket, "["),
                (TokenVariant::RBracket, "]"),
                (TokenVariant::LParen, "("),
                (TokenVariant::RParen, ")"),
                (TokenVariant::LCurly, "{"),
                (TokenVariant::RCurly, "}"),
            ]
        );
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn adjacent_tokens_no_whitespace() {
        let tokens = tok("foo:bar");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Identifier, "foo"),
                (TokenVariant::Colon, ":"),
                (TokenVariant::Identifier, "bar"),
            ]
        );
    }

    #[test]
    fn multiple_whitespace_types() {
        let tokens = tok("a \t\r\n b");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "a"));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "b"));
    }

    #[test]
    fn neq_followed_by_identifier() {
        let tokens = tok("!=foo");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Neq, "!="));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "foo"));
    }

    #[test]
    fn not_followed_by_non_eq() {
        let tokens = tok("!foo");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Not, "!"));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "foo"));
    }

    #[test]
    fn doc_comment_before_type() {
        let src = "/** Docs */ type Foo = string;";
        let tokens = tok(src);
        assert_eq!(tokens[0].0, TokenVariant::Documentation);
        assert_eq!(tokens[1].0, TokenVariant::Type);
    }

    #[test]
    fn regex_with_backslash_escape() {
        // /a\/b/ → the \/ is an escaped slash, should not terminate regex
        let tokens = tok(r"/a\/b/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Regex);
        assert_eq!(tokens[0].1, r"/a\/b/");
    }

    #[test]
    fn regex_with_double_backslash() {
        // /a\\/ → \\ is double backslash (escape resets), then / terminates
        let tokens = tok(r"/a\\/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Regex);
    }

    #[test]
    fn symm_diff_in_expression() {
        let tokens = tok("A <> B");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Identifier, "A"),
                (TokenVariant::SymmDiff, "<>"),
                (TokenVariant::Identifier, "B"),
            ]
        );
    }

    #[test]
    fn all_range_variants() {
        let matrix: &[(&str, Vec<(TokenVariant, &str)>)] = &[
            ("1..10", vec![
                (TokenVariant::Number, "1"),
                (TokenVariant::Range, ".."),
                (TokenVariant::Number, "10"),
            ]),
            ("1.<10", vec![
                (TokenVariant::Number, "1"),
                (TokenVariant::Range, ".<"),
                (TokenVariant::Number, "10"),
            ]),
            ("1<.10", vec![
                (TokenVariant::Number, "1"),
                (TokenVariant::Range, "<."),
                (TokenVariant::Number, "10"),
            ]),
        ];
        for (src, expected) in matrix {
            let tokens = tok(src);
            assert_eq!(tokens.len(), expected.len(), "Wrong count for '{}'", src);
            for (i, (ev, eval)) in expected.iter().enumerate() {
                assert_eq!(tokens[i].0, *ev, "Wrong variant at {} for '{}'", i, src);
                assert_eq!(tokens[i].1, *eval, "Wrong value at {} for '{}'", i, src);
            }
        }
    }

    #[test]
    fn exclusive_range_both_sides() {
        // <.< is exclusive on both sides
        let tokens = tok("1<.<10");
        assert_eq!(tokens[0], (TokenVariant::Number, "1"));
        assert_eq!(tokens[1], (TokenVariant::Range, "<.<"));
        assert_eq!(tokens[2], (TokenVariant::Number, "10"));
    }

    #[test]
    fn comment_with_star_not_doc() {
        // /* not a doc comment */ should be skipped
        let tokens = tok("/* hello */ foo");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo"));
    }

    #[test]
    fn line_comment_eof_no_newline() {
        // Line comment at EOF without trailing newline
        let tokens = tok("foo // trailing");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo"));
    }

    #[test]
    fn slash_context_requires_two_preceding_tokens() {
        // With only 1 token before `/`, slash_context is false → regex
        // But `type /foo/` — 'type' is keyword, then / is regex context
        let tokens = tok("type /foo/");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].0, TokenVariant::Type);
        assert_eq!(tokens[1].0, TokenVariant::Regex);
    }

    #[test]
    fn slash_context_non_import_two_tokens() {
        // Two tokens but second_last is not Import or Slash → regex
        // "type Foo /abc/" → Type, Identifier, Regex
        let tokens = tok("type Foo /abc/");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].0, TokenVariant::Type);
        assert_eq!(tokens[1].0, TokenVariant::Identifier);
        assert_eq!(tokens[2].0, TokenVariant::Regex);
    }

    #[test]
    fn error_location_unrecognized() {
        let err = Lexer::tokenize("foo #").unwrap_err();
        assert_eq!(err.message, "Token not recognized");
        assert_eq!(err.location.l, 0);
        assert_eq!(err.location.c, 4);
        assert_eq!(err.location.v, "#");
    }

    #[test]
    fn error_location_unterminated_string_multiline() {
        // Unterminated string starting on line 1
        let err = Lexer::tokenize("foo\n\"hello").unwrap_err();
        assert_eq!(err.message, "String not terminated");
        assert_eq!(err.location.l, 1);
    }

    #[test]
    fn plus_minus_in_expression() {
        let tokens = tok("a + b - c");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Identifier, "a"),
                (TokenVariant::Plus, "+"),
                (TokenVariant::Identifier, "b"),
                (TokenVariant::Minus, "-"),
                (TokenVariant::Identifier, "c"),
            ]
        );
    }

    #[test]
    fn backslash_token() {
        let tokens = tok("\\");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Backslash, "\\"));
    }

    #[test]
    fn doc_comment_unterminated_after_star() {
        // "/**x" — starts as doc comment (/**), never finds */
        tok_err("/**x", "Comment not terminated");
    }

    #[test]
    fn block_comment_star_then_non_slash() {
        // "/* * */" — star inside block that's not followed by / right away
        let tokens = tok("/* * */");
        assert!(tokens.is_empty());
    }

    #[test]
    fn number_single_digit_before_dot_dot() {
        // "5.." → Number("5") Range("..")
        let tokens = tok("5..");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Number, "5"));
        assert_eq!(tokens[1], (TokenVariant::Range, ".."));
    }

    #[test]
    fn number_float_then_range() {
        // "1.5..10" → Number("1.5") Range("..") Number("10")
        let tokens = tok("1.5..10");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (TokenVariant::Number, "1.5"));
        assert_eq!(tokens[1], (TokenVariant::Range, ".."));
        assert_eq!(tokens[2], (TokenVariant::Number, "10"));
    }

    #[test]
    fn underscore_only_identifier() {
        let tokens = tok("_");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "_"));
    }

    #[test]
    fn identifier_starting_with_underscore_and_digits() {
        let tokens = tok("_123abc");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "_123abc"));
    }

    #[test]
    fn complex_real_world_example() {
        let src = r#"
/** User type definition */
import common/base;

type User = {
  @pattern(/^[a-z]+$/)
  name: string,
  age: 0..150,
  active: true | false
};
"#;
        let tokens = tok(src);
        // Just check it tokenizes without error and has reasonable count
        assert!(tokens.len() > 20, "Expected >20 tokens, got {}", tokens.len());
        // Verify first few tokens
        assert_eq!(tokens[0].0, TokenVariant::Documentation);
        assert_eq!(tokens[1].0, TokenVariant::Import);
        assert_eq!(tokens[2].0, TokenVariant::Identifier); // common
        assert_eq!(tokens[3].0, TokenVariant::Slash);
        assert_eq!(tokens[4].0, TokenVariant::Identifier); // base
        assert_eq!(tokens[5].0, TokenVariant::Semicolon);
        assert_eq!(tokens[6].0, TokenVariant::Type);
    }

    // ── Carriage return handling ────────────────────────────────────

    #[test]
    fn crlf_line_endings() {
        let src = "foo\r\nbar";
        let tokens = tok_pos(src);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo", 0, 0));
        // \r is consumed as whitespace, \n increments line
        assert_eq!(tokens[1], (TokenVariant::Identifier, "bar", 1, 0));
    }

    // ── Token value exact slicing ───────────────────────────────────

    #[test]
    fn string_value_includes_quotes() {
        let tokens = tok(r#""hello world""#);
        assert_eq!(tokens[0].1, r#""hello world""#);
    }

    #[test]
    fn regex_value_includes_slashes() {
        let tokens = tok("/abc/");
        assert_eq!(tokens[0].1, "/abc/");
    }

    #[test]
    fn doc_comment_value_includes_delimiters() {
        let tokens = tok("/** docs */");
        assert_eq!(tokens[0].1, "/** docs */");
    }

    // ── Sequential numbers ──────────────────────────────────────────

    #[test]
    fn two_numbers_space_separated() {
        let tokens = tok("42 99");
        assert_eq!(
            tokens,
            vec![
                (TokenVariant::Number, "42"),
                (TokenVariant::Number, "99"),
            ]
        );
    }

    // ── Dot exclusive range with extra context ──────────────────────

    #[test]
    fn dot_followed_by_lt() {
        // ".<" → Range
        let tokens = tok("5.<10");
        assert_eq!(tokens[0], (TokenVariant::Number, "5"));
        assert_eq!(tokens[1], (TokenVariant::Range, ".<"));
        assert_eq!(tokens[2], (TokenVariant::Number, "10"));
    }

    #[test]
    fn lt_alone() {
        let tokens = tok("< foo");
        assert_eq!(tokens[0], (TokenVariant::Lt, "<"));
        assert_eq!(tokens[1], (TokenVariant::Identifier, "foo"));
    }

    #[test]
    fn doc_comment_immediate_close() {
        // "/**/" → empty, should be skipped
        let tokens = tok("/**/ foo");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (TokenVariant::Identifier, "foo"));
    }

    // ── Block comment unterminated with star-but-no-slash ───────────

    #[test]
    fn block_comment_unterminated_star_no_slash() {
        // "/* hello *" — has a * but never */
        tok_err("/* hello *", "Comment not terminated");
    }

    // ── Regex escape toggle ─────────────────────────────────────────

    #[test]
    fn regex_backslash_toggle() {
        // /a\b/ — \b is an escape, then / closes
        let tokens = tok(r"/a\b/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Regex);
    }

    #[test]
    fn regex_triple_backslash_before_slash() {
        // /a\\\\/ — four backslashes: escape toggles back to non-escape, / closes
        let tokens = tok(r"/a\\\\/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, TokenVariant::Regex);
    }
}
