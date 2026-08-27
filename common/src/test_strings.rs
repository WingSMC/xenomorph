//! Shared source text for lexer and parser tests.
//! Builders compose larger grammar cases recursively with `format!`.

pub mod lexer {
    pub const EMPTY_SOURCES: &[&str] = &["", " ", "\t\r\n  "];
    pub const SINGLE_CHARS: &[&str] = &[
        ":", "?", ",", ";", "@", "-", "!", "|", "&", "{", "}", "[", "]", ">", "<", "(", ")", "=",
    ];
    pub const WORDS: &str = "type T_1 = set enum true false validator;";
    pub const POSITIONED_WORDS: &str = "foo\n  bar";
    pub const IMPORT: &str = "import foo2/bar_3;";
    pub const EMPTY_IMPORT: &str = "import ;";
    pub const NUMBERS: &str = "0 -42 3.14";
    pub const MINUS_IDENTIFIER: &str = "-value";
    pub const NOT_EQUAL: &str = "!=";
    pub const DELIMITED: &str = "\"hello\" /a\\/b/ /** docs */";
    pub const SKIPPED_COMMENTS: &str = "// line\n/* block */";
    pub const EMPTY_COMMENT: &str = "/**/";
    pub const UNTERMINATED_STRING: &str = "\"hello";
    pub const UNTERMINATED_REGEX: &str = "/abc";
    pub const NEWLINE_REGEX: &str = "/abc\n";
    pub const UNTERMINATED_COMMENT: &str = "/* comment";
    pub const WORD_DELIMITER: &str = "name;";
    pub const NUMBER_DELIMITER: &str = "-42;";
    pub const IMPORT_DELIMITER: &str = "import foo/bar;";
    pub const WORD_EOF: &str = "name";
    pub const NUMBER_EOF: &str = "-42";
    pub const IMPORT_EOF: &str = "import foo/bar";
    pub const UTF8_DELIMITED: &str = "\"é\";/ø/;";
    pub const UNKNOWN_UTF8: &str = "é";
    pub const UNKNOWN_ASCII: &str = ".";

    pub const STRING_ERROR: &str = "String not terminated";
    pub const REGEX_ERROR: &str = "Malformed regex";
    pub const COMMENT_ERROR: &str = "Comment not terminated";
    pub const UNKNOWN_ERROR: &str = "Token not recognized";
}

pub mod parser {
    pub const TYPE_NAME: &str = "Example";
    pub const SECOND_TYPE_NAME: &str = "After";
    pub const TYPE_DOCS: &str = "A documented type.";
    pub const FIELD_DOCS: &str = "A documented field.";
    pub const DANGLING_DOCS: &str = "No field follows.";
    pub const IMPORT_PATH: &str = "models/user";
    pub const IDENTIFIER: &str = "u8";
    pub const CUSTOM_IDENTIFIER: &str = "User";
    pub const INTEGER: &str = "42";
    pub const FLOAT: &str = "3.14";
    pub const STRING: &str = "\"hello\"";
    pub const TRUE: &str = "true";
    pub const FALSE: &str = "false";
    pub const REGEX: &str = "/^[a-z]+$/";

    pub const UNKNOWN_DECLARATION_ERROR: &str = "Unknown declaration";
    pub const EOF_ERROR: &str = "Unexpected end of file";
    pub const DANGLING_DOCS_WARNING: &str = "Documentation comment without a field";
    pub const IMPORT_DOCS_INFO: &str = "Import declarations cannot have documentation comments";

    pub fn documented(docs: &str, source: &str) -> String {
        format!("/** {docs} */\n{source}")
    }

    pub fn type_decl(name: &str, ty: &str) -> String {
        format!("type {name} = {ty};")
    }

    pub fn type_decl_without_semicolon(name: &str, ty: &str) -> String {
        format!("type {name} = {ty}")
    }

    pub fn generic_type_decl(name: &str, generics: &[(&str, Option<&str>)], ty: &str) -> String {
        let params = generics
            .iter()
            .map(|(name, constraint)| match constraint {
                Some(constraint) => format!("{name}: {constraint}"),
                None => (*name).to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("type {name}<{params}> = {ty};")
    }

    pub fn import(path: &str) -> String {
        format!("import {path};")
    }

    pub fn optional(ty: &str) -> String {
        format!("?{ty}")
    }

    pub fn array(ty: &str) -> String {
        format!("{ty}[]")
    }

    pub fn tuple(items: &[&str]) -> String {
        format!("[{}]", items.join(", "))
    }

    pub fn tuple_with_trailing_comma(items: &[&str]) -> String {
        format!("[{},]", items.join(", "))
    }

    pub fn set(items: &[&str]) -> String {
        format!("set {}", tuple(items))
    }

    pub fn sum(items: &[&str]) -> String {
        format!("| {}", items.join(" | "))
    }

    pub fn intersection(items: &[&str]) -> String {
        format!("& {}", items.join(" & "))
    }

    pub fn field(name: &str, ty: &str) -> String {
        format!("{name}: {ty}")
    }

    pub fn documented_field(docs: &str, name: &str, ty: &str) -> String {
        documented(docs, &field(name, ty))
    }

    pub fn struct_type(fields: &[String]) -> String {
        format!("{{ {} }}", fields.join(", "))
    }

    pub fn struct_with_trailing_comma(fields: &[String]) -> String {
        format!("{{ {}, }}", fields.join(", "))
    }

    pub fn enum_type(variants: &[String]) -> String {
        format!("enum {}", struct_type(variants))
    }

    pub fn annotation(name: &str, args: &[&str]) -> String {
        format!("@{name}({})", args.join(", "))
    }

    pub fn annotation_with_trailing_comma(name: &str, args: &[&str]) -> String {
        format!("@{name}({},)", args.join(", "))
    }

    pub fn marker_annotation(name: &str) -> String {
        format!("@{name}")
    }

    pub fn annotated_type(ty: &str, annotations: &[String]) -> String {
        match annotations.is_empty() {
            true => ty.to_string(),
            false => format!("{ty} {}", annotations.join(" ")),
        }
    }

    pub fn all_simple_types() -> String {
        [
            type_decl("Identifier", IDENTIFIER),
            type_decl("OptionalIdentifier", &optional(IDENTIFIER)),
            type_decl("Array", &array(IDENTIFIER)),
            type_decl("OptionalArray", &optional(&array(IDENTIFIER))),
            type_decl("Integer", INTEGER),
            type_decl("OptionalInteger", &optional(INTEGER)),
            type_decl("Float", FLOAT),
            type_decl("String", STRING),
            type_decl("True", TRUE),
            type_decl("False", FALSE),
        ]
        .join("\n")
    }

    pub fn all_composite_types() -> String {
        let fields = vec![
            documented_field(FIELD_DOCS, "id", IDENTIFIER),
            field("name", STRING),
        ];
        let variants = vec![field("Enabled", TRUE), field("Disabled", FALSE)];
        [
            type_decl("EmptyTuple", &tuple(&[])),
            type_decl("Tuple", &tuple(&[IDENTIFIER, STRING])),
            type_decl("Set", &set(&[INTEGER, FLOAT])),
            type_decl("EmptyStruct", &struct_type(&[])),
            type_decl("Struct", &struct_type(&fields)),
            type_decl("Enum", &enum_type(&variants)),
            type_decl("Sum", &sum(&[IDENTIFIER, STRING])),
            type_decl(
                "Intersection",
                &intersection(&[IDENTIFIER, CUSTOM_IDENTIFIER]),
            ),
        ]
        .join("\n")
    }

    pub fn all_annotation_expressions() -> String {
        let nested = annotation("inner", &[INTEGER]);
        let tuple_arg = tuple(&[IDENTIFIER, STRING]);
        let outer = annotation("outer", &[REGEX, &nested, &tuple_arg, TRUE]);
        let annotations = vec![marker_annotation("marker"), annotation("empty", &[]), outer];
        type_decl(TYPE_NAME, &annotated_type(IDENTIFIER, &annotations))
    }

    pub fn recoverable_struct() -> String {
        let body = format!(
            "{{ {}, brokenColon string, {}, missingType: , {}, brokenArray: u8[, {}, endsBad: }}",
            field("first", IDENTIFIER),
            field("second", TRUE),
            field("third", STRING),
            field("fourth", FLOAT),
        );
        format!(
            "{}\n{}",
            type_decl("Recovered", &body),
            type_decl(SECOND_TYPE_NAME, STRING)
        )
    }

    pub fn struct_with_dangling_docs() -> String {
        let docs = format!("/** {DANGLING_DOCS} */");
        type_decl(
            TYPE_NAME,
            &format!("{{ {}, {docs} }}", field("id", IDENTIFIER)),
        )
    }

    pub fn declaration_recovery() -> String {
        format!("unknown;\n{}", type_decl(SECOND_TYPE_NAME, STRING))
    }

    pub fn array_and_tuple() -> String {
        [
            type_decl("Array", &array(IDENTIFIER)),
            type_decl("OptionalArray", &optional(&array(IDENTIFIER))),
            type_decl("Tuple", &tuple(&[IDENTIFIER])),
        ]
        .join("\n")
    }

    pub fn documented_import() -> String {
        documented(TYPE_DOCS, &import(IMPORT_PATH))
    }

    pub fn if_else_with_nested_annotation() -> String {
        let nested = annotation("lt", &[INTEGER]);
        let if_annotation = annotation("if", &[&nested, TRUE]);
        let else_annotation = annotation("else", &[FALSE]);
        type_decl(
            TYPE_NAME,
            &annotated_type(IDENTIFIER, &[if_annotation, else_annotation]),
        )
    }

    pub fn else_without_if() -> String {
        type_decl(
            TYPE_NAME,
            &annotated_type(IDENTIFIER, &[annotation("else", &[FALSE])]),
        )
    }

    pub fn trailing_separators() -> String {
        let fields = vec![field("id", IDENTIFIER)];
        let annotations = vec![annotation_with_trailing_comma("check", &[INTEGER])];
        [
            type_decl("Tuple", &tuple_with_trailing_comma(&[IDENTIFIER])),
            type_decl("Struct", &struct_with_trailing_comma(&fields)),
            type_decl("Annotated", &annotated_type(IDENTIFIER, &annotations)),
        ]
        .join("\n")
    }
}
