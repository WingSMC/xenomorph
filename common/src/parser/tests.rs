use crate::{
    lexer::Lexer,
    parser::{Declaration, Expr, FloatSize, IntegerSize, Literal, Parser, SimpleType, Type},
    test_strings::parser as source,
    XenoDiagSeverity, XenoDiagnostic,
};

fn parse<'src>(
    tokens: &'src crate::lexer::XenoTokens<'src>,
) -> (Vec<Declaration<'src>>, Vec<XenoDiagnostic<'src>>) {
    Parser::parse(tokens)
}

fn assert_no_errors(diagnostics: &[XenoDiagnostic<'_>]) {
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err),
        "unexpected parse errors: {diagnostics:#?}"
    );
}

fn type_declaration<'src>(declaration: &'src Declaration<'src>) -> (&'src str, &'src Type<'src>) {
    match declaration {
        Declaration::Type { name, ty, .. } => (name.v, &ty.0),
        declaration => panic!("expected type declaration, got {declaration:?}"),
    }
}

#[test]
fn empty_source_has_empty_ast() {
    let tokens = Lexer::tokenize("").expect("empty source must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert!(ast.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn parses_imports_and_keeps_non_fatal_information() {
    let text = source::documented_import();
    let tokens = Lexer::tokenize(&text).expect("import must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 1);
    match &ast[0] {
        Declaration::Import { path, .. } => assert_eq!(path, &["models", "user"]),
        declaration => panic!("expected import, got {declaration:?}"),
    }
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == XenoDiagSeverity::Info
            && diagnostic.message.contains(source::IMPORT_DOCS_INFO)
    }));
}

#[test]
fn parses_documentation_and_generics() {
    let generic = source::generic_type_decl(
        source::TYPE_NAME,
        &[("T", None), ("U", Some(source::IDENTIFIER))],
        "T",
    );
    let text = source::documented(source::TYPE_DOCS, &generic);
    let tokens = Lexer::tokenize(&text).expect("generic type must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 1);
    match &ast[0] {
        Declaration::Type {
            docs,
            name,
            generics: Some(generics),
            ty: (Type::Simple(SimpleType::Identifier(identifier, arguments)), _),
            ..
        } => {
            assert_eq!(*docs, Some(source::TYPE_DOCS));
            assert_eq!(name.v, source::TYPE_NAME);
            assert_eq!(identifier.v, "T");
            assert!(arguments.is_none());
            assert_eq!(generics.len(), 2);
            assert_eq!(generics[0].0.v, "T");
            assert!(generics[0].1.is_none());
            assert_eq!(generics[1].0.v, "U");
            assert_eq!(
                generics[1].1.expect("U has a constraint").v,
                source::IDENTIFIER
            );
        }
        declaration => panic!("unexpected declaration: {declaration:?}"),
    }
}

#[test]
fn parses_every_simple_type_and_literal_case() {
    let text = source::all_simple_types();
    let tokens = Lexer::tokenize(&text).expect("simple types must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 10);
    assert!(matches!(
        type_declaration(&ast[0]).1,
        Type::Simple(SimpleType::Identifier(_, _))
    ));
    assert!(matches!(
        type_declaration(&ast[1]).1,
        Type::Simple(SimpleType::Optional(inner)) if matches!(**inner, SimpleType::Identifier(_, _))
    ));
    assert!(matches!(
        type_declaration(&ast[2]).1,
        Type::Simple(SimpleType::Array(_, _))
    ));
    assert!(matches!(
        type_declaration(&ast[3]).1,
        Type::Simple(SimpleType::Optional(inner)) if matches!(**inner, SimpleType::Array(_, _))
    ));
    assert!(matches!(
        type_declaration(&ast[4]).1,
        Type::Simple(SimpleType::Literal(Literal::Int(_)))
    ));
    assert!(matches!(
        type_declaration(&ast[5]).1,
        Type::Simple(SimpleType::Optional(inner))
            if matches!(**inner, SimpleType::Literal(Literal::Int(_)))
    ));
    assert!(matches!(
        type_declaration(&ast[6]).1,
        Type::Simple(SimpleType::Literal(Literal::Float(_)))
    ));
    assert!(matches!(
        type_declaration(&ast[7]).1,
        Type::Simple(SimpleType::Literal(Literal::String(_, _)))
    ));
    assert!(matches!(
        type_declaration(&ast[8]).1,
        Type::Simple(SimpleType::Literal(Literal::Boolean(true, _)))
    ));
    assert!(matches!(
        type_declaration(&ast[9]).1,
        Type::Simple(SimpleType::Literal(Literal::Boolean(false, _)))
    ));
}

#[test]
fn numeric_literals_infer_minimum_representations() {
    let text = "type Zero = 0; type Positive = 255; type Negative = -129; type SmallFloat = 1.5; type Precise = 1.234567890123456789;";
    let tokens = Lexer::tokenize(text).expect("numeric literals must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    let literal_at = |index| match type_declaration(&ast[index]).1 {
        Type::Simple(SimpleType::Literal(literal)) => literal,
        ty => panic!("expected literal, got {ty:?}"),
    };

    assert!(matches!(literal_at(0), Literal::Int(value)
        if !value.representation.signed && value.representation.size == IntegerSize::Bits(1)));
    assert!(matches!(literal_at(1), Literal::Int(value)
        if !value.representation.signed && value.representation.size == IntegerSize::Bits(8)));
    assert!(matches!(literal_at(2), Literal::Int(value)
        if value.representation.signed && value.representation.size == IntegerSize::Bits(9)));
    assert!(matches!(literal_at(3), Literal::Float(value)
        if value.representation.precision == 2
            && value.representation.scale == 1
            && value.representation.size == FloatSize::F32));
    assert!(matches!(literal_at(4), Literal::Float(value)
        if value.representation.size == FloatSize::Decimal));
}

#[test]
fn parses_explicit_numeric_literal_representations() {
    let text = "type Signed = 1 as i32; type Unsigned = 1 as u64; type Float = 1.5 as f64; type Exact = 1.25 as decimal; type Big = 1 as bigint;";
    let tokens = Lexer::tokenize(text).expect("numeric casts must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert!(matches!(type_declaration(&ast[0]).1,
        Type::Simple(SimpleType::Literal(Literal::Int(value)))
            if value.representation.signed
                && value.representation.size == IntegerSize::Bits(32)
                && value.cast.is_some_and(|cast| cast.v == "i32")));
    assert!(matches!(type_declaration(&ast[1]).1,
        Type::Simple(SimpleType::Literal(Literal::Int(value)))
            if !value.representation.signed
                && value.representation.size == IntegerSize::Bits(64)
                && value.cast.is_some_and(|cast| cast.v == "u64")));
    assert!(matches!(type_declaration(&ast[2]).1,
        Type::Simple(SimpleType::Literal(Literal::Float(value)))
            if value.representation.size == FloatSize::F64));
    assert!(matches!(type_declaration(&ast[3]).1,
        Type::Simple(SimpleType::Literal(Literal::Float(value)))
            if value.representation.size == FloatSize::Decimal));
    assert!(matches!(type_declaration(&ast[4]).1,
        Type::Simple(SimpleType::Literal(Literal::Int(value)))
            if value.representation.size == IntegerSize::Arbitrary));
}

#[test]
fn parses_numeric_casts_in_annotation_arguments() {
    let text = "type Limited = u64 @min(1 as u64);";
    let tokens = Lexer::tokenize(text).expect("numeric cast must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    let Declaration::Type {
        ty: (_, annotations),
        ..
    } = &ast[0]
    else {
        panic!("expected type declaration");
    };
    assert!(matches!(
        &annotations[0].params[0],
        Expr::Type(Type::Simple(SimpleType::Literal(Literal::Int(integer))))
            if integer.cast.is_some_and(|cast| cast.v == "u64")
    ));
}

#[test]
fn rejects_invalid_or_lossy_numeric_literal_casts() {
    for (text, expected) in [
        ("type Bad = 256 as u8;", "outside the range of u8"),
        ("type Bad = -1 as u8;", "outside the range of u8"),
        ("type Bad = 1 as string;", "Cannot cast integer literal"),
        (
            "type Bad = 1.23456789 as f32;",
            "cannot be represented by f32",
        ),
    ] {
        let tokens = Lexer::tokenize(text).expect("invalid cast still lexes");
        let (_, diagnostics) = parse(&tokens);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "expected diagnostic containing {expected:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn postfix_arrays_do_not_conflict_with_tuples() {
    let text = source::array_and_tuple();
    let tokens = Lexer::tokenize(&text).expect("array and tuple types must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert!(matches!(
        type_declaration(&ast[0]).1,
        Type::Simple(SimpleType::Array(identifier, arguments))
            if identifier.v == source::IDENTIFIER && arguments.is_none()
    ));
    assert!(matches!(
        type_declaration(&ast[1]).1,
        Type::Simple(SimpleType::Optional(inner))
            if matches!(&**inner, SimpleType::Array(identifier, arguments)
                if identifier.v == source::IDENTIFIER && arguments.is_none())
    ));
    assert!(matches!(type_declaration(&ast[2]).1, Type::Tuple(items) if items.len() == 1));
}

#[test]
fn parses_nested_generic_specializations_as_simple_types() {
    let text = "type Box<T> = T; type Example = Box<Dict<string, Box<u8>>>; type Fields = { value: Box<string>, values: Box<u8>[], };";
    let tokens = Lexer::tokenize(text).expect("generic specializations must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 3);
    assert!(matches!(
        type_declaration(&ast[1]).1,
        Type::Simple(SimpleType::Identifier(name, Some(arguments)))
            if name.v == "Box"
                && matches!(&arguments[0], SimpleType::Identifier(name, Some(arguments))
                    if name.v == "Dict" && arguments.len() == 2)
    ));
    assert!(matches!(
        type_declaration(&ast[2]).1,
        Type::Struct(fields)
            if matches!(&fields[0].1, SimpleType::Identifier(name, Some(arguments))
                    if name.v == "Box" && arguments.len() == 1)
                && matches!(&fields[1].1, SimpleType::Array(name, Some(arguments))
                    if name.v == "Box" && arguments.len() == 1)
    ));
}

#[test]
fn parses_every_composite_type_case() {
    let text = source::all_composite_types();
    let tokens = Lexer::tokenize(&text).expect("composite types must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 8);
    assert!(matches!(type_declaration(&ast[0]).1, Type::Tuple(items) if items.is_empty()));
    assert!(matches!(type_declaration(&ast[1]).1, Type::Tuple(items) if items.len() == 2));
    assert!(matches!(
        type_declaration(&ast[2]).1,
        Type::Set(set)
            if set.element_type.is_none()
                && set.values.as_ref().is_some_and(|values| values.len() == 2)
    ));
    assert!(matches!(type_declaration(&ast[3]).1, Type::Struct(fields) if fields.is_empty()));
    assert!(matches!(
        type_declaration(&ast[4]).1,
        Type::Struct(fields)
            if fields.len() == 2
                && fields[0].2.is_some()
                && fields[0].2.expect("field docs").v.contains(source::FIELD_DOCS)
    ));
    assert!(matches!(type_declaration(&ast[5]).1, Type::Enum(items) if items.len() == 2));
    assert!(matches!(type_declaration(&ast[6]).1, Type::Sum(items) if items.len() == 2));
    assert!(matches!(type_declaration(&ast[7]).1, Type::Intersection(items) if items.len() == 2));
}

#[test]
fn parses_typed_inferred_and_prefilled_sets() {
    let text = "type Open = set<string>; type Inferred = set [\"a\", \"b\"]; type Typed = set<string> [\"a\", \"b\"]; type Empty = set<string> [];";
    let tokens = Lexer::tokenize(text).expect("set types must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 4);
    assert!(matches!(type_declaration(&ast[0]).1, Type::Set(set)
        if matches!(&set.element_type, Some(SimpleType::Identifier(name, None)) if name.v == "string")
            && set.values.is_none()));
    assert!(matches!(type_declaration(&ast[1]).1, Type::Set(set)
        if set.element_type.is_none()
            && set.values.as_ref().is_some_and(|values| values.len() == 2)));
    assert!(matches!(type_declaration(&ast[2]).1, Type::Set(set)
        if set.element_type.is_some()
            && set.values.as_ref().is_some_and(|values| values.len() == 2)));
    assert!(matches!(type_declaration(&ast[3]).1, Type::Set(set)
        if set.element_type.is_some()
            && set.values.as_ref().is_some_and(Vec::is_empty)));
}

#[test]
fn set_prefills_reject_non_literals() {
    let text = "type Bad = set<string> [\"a\", string, \"b\"]; type After = string;";
    let tokens = Lexer::tokenize(text).expect("invalid set prefill must still lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 1, "the following declaration must survive");
    assert_eq!(type_declaration(&ast[0]).0, "After");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == XenoDiagSeverity::Err
            && diagnostic.message.contains("only literals")
            && diagnostic.location.v == "string"
    }));
}

#[test]
fn rejects_bare_set_and_recovers_at_the_declaration_boundary() {
    let text = "type Bad = set; type After = string;";
    let tokens = Lexer::tokenize(text).expect("bare set must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 1);
    assert_eq!(type_declaration(&ast[0]).0, "After");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("requires an element type")));
}

#[test]
fn parses_every_annotation_expression_case() {
    let text = source::all_annotation_expressions();
    let tokens = Lexer::tokenize(&text).expect("annotations must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    let Declaration::Type {
        ty: (_, annotations),
        ..
    } = &ast[0]
    else {
        panic!("expected type declaration");
    };
    assert_eq!(annotations.len(), 3);
    assert!(annotations[0].params.is_empty());
    assert!(annotations[1].params.is_empty());
    assert!(matches!(annotations[2].params[0], Expr::Regex(_)));
    assert!(matches!(annotations[2].params[1], Expr::Annotation(_)));
    assert!(matches!(
        annotations[2].params[2],
        Expr::Type(Type::Tuple(_))
    ));
    assert!(matches!(
        annotations[2].params[3],
        Expr::Type(Type::Simple(SimpleType::Literal(Literal::Boolean(true, _))))
    ));
}

#[test]
fn accepts_trailing_separators_in_closed_lists_and_structs() {
    let text = source::trailing_separators();
    let tokens = Lexer::tokenize(&text).expect("trailing separators must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert_eq!(ast.len(), 3);
    assert!(matches!(type_declaration(&ast[0]).1, Type::Tuple(items) if items.len() == 1));
    assert!(matches!(type_declaration(&ast[1]).1, Type::Struct(fields) if fields.len() == 1));
    let Declaration::Type {
        ty: (_, annotations),
        ..
    } = &ast[2]
    else {
        panic!("expected annotated type");
    };
    assert_eq!(annotations[0].params.len(), 1);
}

#[test]
fn struct_field_failures_recover_at_commas_and_closing_braces() {
    let text = source::recoverable_struct();
    let tokens = Lexer::tokenize(&text).expect("recoverable struct must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 2, "the following declaration must survive");
    assert!(matches!(
        type_declaration(&ast[0]).1,
        Type::Struct(fields)
            if fields.iter().map(|(key, _, _)| key.v).collect::<Vec<_>>()
                == ["first", "second", "third", "fourth"]
    ));
    assert_eq!(type_declaration(&ast[1]).0, source::SECOND_TYPE_NAME);
    assert!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err)
            .count()
            >= 4
    );
}

#[test]
fn warning_only_struct_still_returns_its_declaration() {
    let text = source::struct_with_dangling_docs();
    let tokens = Lexer::tokenize(&text).expect("documented struct must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 1);
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == XenoDiagSeverity::Err));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == XenoDiagSeverity::Warn
            && diagnostic.message.contains(source::DANGLING_DOCS_WARNING)
    }));
}

#[test]
fn quoted_strings_are_normalized_to_struct_field_identifiers() {
    let text = "type Payload = { \"type\": string, \"ecu.test\": bool };";
    let tokens = Lexer::tokenize(text).expect("quoted fields must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_no_errors(&diagnostics);
    assert!(matches!(
        type_declaration(&ast[0]).1,
        Type::Struct(fields)
            if fields.len() == 2
                && fields[0].0.v == "type"
                && fields[1].0.v == "ecu.test"
    ));
}

#[test]
fn unquoted_type_keyword_is_not_a_struct_field_identifier() {
    let text = "type Payload = { type: string };";
    let tokens = Lexer::tokenize(text).expect("keyword-shaped field must lex");
    let (_, diagnostics) = parse(&tokens);

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == XenoDiagSeverity::Err
            && diagnostic.message.contains("Expected Identifier")
    }));
}

#[test]
fn declaration_recovery_continues_after_semicolon() {
    let text = source::declaration_recovery();
    let tokens = Lexer::tokenize(&text).expect("declarations must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert_eq!(ast.len(), 1);
    assert_eq!(type_declaration(&ast[0]).0, source::SECOND_TYPE_NAME);
    assert!(diagnostics.iter().any(|diagnostic| diagnostic
        .message
        .contains(source::UNKNOWN_DECLARATION_ERROR)));
}

#[test]
fn missing_semicolon_reports_unexpected_eof() {
    let text = source::type_decl_without_semicolon(source::TYPE_NAME, source::IDENTIFIER);
    let tokens = Lexer::tokenize(&text).expect("declaration must lex");
    let (ast, diagnostics) = parse(&tokens);

    assert!(ast.is_empty());
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains(source::EOF_ERROR)));
}
