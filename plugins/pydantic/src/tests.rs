use super::*;
use xenomorph_common::semantic::{TypeDeclarationInfo, BUILTIN_TYPES};
use xenomorph_common::{lexer::Lexer, parser::Parser};

fn generator_for(source: &str) -> (PydanticGenerator, Vec<Declaration>) {
    let tokens = Lexer::tokenize(source).unwrap();
    let (ast, diagnostics) = Parser::parse(&tokens, source);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let mut generator = PydanticGenerator::new();
    generator.set_source(source);
    generator.module_path = "test".into();
    generator.type_hierarchy.set_current_module("test");
    generator.type_hierarchy.register_module("test", Vec::new());
    for semantic_type in BUILTIN_TYPES {
        generator.type_hierarchy.insert_semantic_type(semantic_type);
    }
    for declaration in &ast {
        if let Declaration::Type {
            name, generics, ty, ..
        } = declaration
        {
            generator.type_hierarchy.insert_declaration(
                "test",
                name.text(source),
                TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
            );
        }
    }
    (generator, ast)
}

fn generate_with(
    source: &str,
    configure: impl FnOnce(&mut PydanticGenerator),
) -> (String, Vec<XenoDiagnostic>) {
    let (mut generator, ast) = generator_for(source);
    configure(&mut generator);
    let mut errors = Vec::new();
    generator.on_before_ast(&ast, &mut errors);
    (generator.render_file(), errors)
}

fn generate(source: &str) -> String {
    let (output, errors) = generate_with(source, |_| {});
    assert!(errors.is_empty(), "{errors:#?}\n{output}");
    output
}

fn analyze(source: &str) -> Vec<XenoDiagnostic> {
    let tokens = Lexer::tokenize(source).unwrap();
    let (ast, _) = Parser::parse(&tokens, source);
    let mut analyzer = PydanticAnalyzer::default();
    analyzer.set_source(source);
    let mut errors = Vec::new();
    for declaration in &ast {
        analyzer.on_before_decl(declaration, &mut errors);
    }
    errors
}

#[test]
fn optional_alias_is_nullable_and_defaults_fields_to_none() {
    let output = generate(
        "type A = ?string @minlen(1); type B = ?u8; type M = { a: A, b: ?bool, c: string, d: D }; type D = |string|null|u8;",
    );
    assert!(
        output.contains("type A = Annotated[str, Field(min_length=1)] | None\n"),
        "{output}"
    );
    assert!(
        output.contains("type B = Annotated[int, Field(ge=0, le=255)] | None\n"),
        "{output}"
    );
    assert!(output.contains("    a: A = None\n"), "{output}");
    assert!(output.contains("    b: bool | None = None\n"), "{output}");
    assert!(output.contains("    c: str\n"), "{output}");
    assert!(output.contains("    d: D = None\n"), "{output}");
}

#[test]
fn native_validators_become_field_constraints() {
    let output = generate(concat!(
        "type Age = u8 @min(18) @max(300); ",
        "type Low = i8 @min(-500); ",
        "type Ratio = f64 @gt(0) @lt(1.5) @multipleof(0.25); ",
        "type Code = string @len(3) @match(/^[A-Z]x$/); ",
        "type Tags = string[] @minlen(1) @maxlen(5);",
    ));
    assert!(
        output.contains("type Age = Annotated[int, Field(ge=18, le=255)]\n"),
        "{output}"
    );
    assert!(
        output.contains("type Low = Annotated[int, Field(ge=-128, le=127)]\n"),
        "{output}"
    );
    assert!(
        output.contains("type Ratio = Annotated[float, Field(gt=0, lt=1.5, multiple_of=0.25)]\n"),
        "{output}"
    );
    assert!(
        output.contains(
            r#"type Code = Annotated[str, Field(min_length=3, max_length=3, pattern="^[A-Z]x$")]"#
        ),
        "{output}"
    );
    assert!(
        output.contains("type Tags = Annotated[list[str], Field(min_length=1, max_length=5)]\n"),
        "{output}"
    );
}

#[test]
fn model_config_annotations_and_field_aliases() {
    let output = generate(concat!(
        "/** A user. */ pub type User = { ",
        "/** Display name. */ name: string, \"first-name\": string, class: u32, _id: uuid, json: email ",
        "} @pydantic::extra(\"allow\") @pydantic::frozen @pydantic::alias_generator(\"camel\") ",
        "@pydantic::config(\"regex_engine\", \"python-re\");",
    ));
    assert!(
        output.contains("class User(BaseModel):\n    \"\"\"A user.\"\"\"\n"),
        "{output}"
    );
    assert!(
        output.contains("    model_config = ConfigDict(extra=\"allow\", frozen=True, alias_generator=to_camel, regex_engine=\"python-re\", populate_by_name=True)\n"),
        "{output}"
    );
    assert!(
        output.contains("    name: str = Field(description=\"Display name.\")\n"),
        "{output}"
    );
    assert!(
        output.contains("    first_name: str = Field(alias=\"first-name\")\n"),
        "{output}"
    );
    assert!(
        output.contains(
            "    class_: Annotated[int, Field(ge=0, le=4294967295)] = Field(alias=\"class\")\n"
        ),
        "{output}"
    );
    assert!(
        output.contains("    id: UUID = Field(alias=\"_id\")\n"),
        "{output}"
    );
    assert!(
        output.contains("    json_: EmailStr = Field(alias=\"json\")\n"),
        "{output}"
    );
    assert!(
        output.contains("from pydantic.alias_generators import to_camel\n"),
        "{output}"
    );
    assert!(
        output.contains("__all__ = [\n    \"User\",\n]\n"),
        "{output}"
    );
}

#[test]
fn default_extra_applies_unless_overridden() {
    let source = "type A = { a: string }; type B = { b: string } @pydantic::extra(\"ignore\");";
    let (output, _) = generate_with(source, |generator| {
        generator.default_extra = Some("forbid".into())
    });
    assert!(
        output.contains("class A(BaseModel):\n    model_config = ConfigDict(extra=\"forbid\")\n"),
        "{output}"
    );
    assert!(
        output.contains("class B(BaseModel):\n    model_config = ConfigDict(extra=\"ignore\")\n"),
        "{output}"
    );
}

#[test]
fn enums_follow_the_xenomorph_wire_format() {
    let output = generate(
        "type Status = enum { Ready: 1, Failed: 2 }; type Shape = enum { Circle: f64, Square: ?u8 };",
    );
    assert!(
        output.contains("class Status(IntEnum):\n    Ready = 1\n    Failed = 2\n"),
        "{output}"
    );
    assert!(output.contains(
        "class Shape_Circle(BaseModel):\n    model_config = ConfigDict(extra=\"forbid\")\n    kind: Literal[\"Circle\"] = \"Circle\"\n    value: float\n"
    ), "{output}");
    assert!(
        output.contains("    value: Annotated[int, Field(ge=0, le=255)] | None = None\n"),
        "{output}"
    );
    assert!(
        output.contains(
            "type Shape = Annotated[Shape_Circle | Shape_Square, Field(discriminator=\"kind\")]\n"
        ),
        "{output}"
    );
}

#[test]
fn literals_sets_tuples_and_generics() {
    let output = generate(concat!(
        "type Color = set [\"red\", \"green\"]; ",
        "type Mixed = |\"a\"|1|?true|string; ",
        "type Pair = [string, ?i32]; ",
        "type Ids = set<uuid>; ",
        "type Map = Dict<string, u16>; ",
        "type Page<T> = { items: T[], next: ?string }; ",
        "type Users = Page<string>;",
    ));
    assert!(
        output.contains("type Color = Literal[\"red\", \"green\"]\n"),
        "{output}"
    );
    assert!(
        output.contains("type Mixed = Literal[\"a\", 1, True] | str | None\n"),
        "{output}"
    );
    assert!(
        output.contains(
            "type Pair = tuple[str, Annotated[int, Field(ge=-2147483648, le=2147483647)] | None]\n"
        ),
        "{output}"
    );
    assert!(output.contains("type Ids = set[UUID]\n"), "{output}");
    assert!(
        output.contains("type Map = dict[str, Annotated[int, Field(ge=0, le=65535)]]\n"),
        "{output}"
    );
    assert!(
        output.contains(
            "class Page[T](BaseModel):\n    items: list[T]\n    next: str | None = None\n"
        ),
        "{output}"
    );
    assert!(output.contains("type Users = Page[str]\n"), "{output}");
}

#[test]
fn struct_utilities_and_intersections_become_models() {
    let output = generate(concat!(
        "type A = { a: string, b: ?u8 }; type B = { c: bool }; ",
        "type AB = &A&B; type P = Partial<A>; type Keys = Keyof<A>;",
    ));
    assert!(output.contains("class AB(BaseModel):\n    a: str\n    b: Annotated[int, Field(ge=0, le=255)] | None = None\n    c: bool\n"), "{output}");
    assert!(
        output.contains("class P(BaseModel):\n    a: str | None = None\n"),
        "{output}"
    );
    assert!(
        output.contains("type Keys = Literal[\"a\", \"b\"]\n"),
        "{output}"
    );
}

#[test]
fn pydantic_validators_and_deprecation() {
    let output = generate(concat!(
        "type Name = string @pydantic::after_validator(\"app.checks.title\") @pydantic::strict @deprecated(\"Use Text.\"); ",
        "type U = { name: Name } @pydantic::before_validator(\"app.checks.prepare\") @deprecated;",
    ));
    assert!(output.contains("# Deprecated: Use Text.\ntype Name = Annotated[str, Field(strict=True), AfterValidator(title)]\n"), "{output}");
    assert!(
        output.contains("from app.checks import prepare, title\n"),
        "{output}"
    );
    assert!(
        output.contains("@deprecated(\"Deprecated.\")\nclass U(BaseModel):\n"),
        "{output}"
    );
    assert!(
        output.contains("    name: Name = Field(deprecated=\"Use Text.\")\n"),
        "{output}"
    );
    assert!(output.contains(
        "    @model_validator(mode=\"before\")\n    @classmethod\n    def _xeno_before_validator_0(cls, data):\n        return prepare(data)\n"
    ), "{output}");
}

#[test]
fn imported_symbols_are_aliased_when_user_types_collide() {
    let output = generate("type Field = string; type M = { a: Field, b: ?string };");
    assert!(output.contains("type Field = str\n"), "{output}");
    let output = generate("type Field = string; type M = { a: Field } @pydantic::title(\"x\"); type N = { \"a-b\": string };");
    assert!(
        output.contains("from pydantic import BaseModel, ConfigDict, Field as Field_2\n"),
        "{output}"
    );
    assert!(
        output.contains("    a_b: str = Field_2(alias=\"a-b\")\n"),
        "{output}"
    );
}

#[test]
fn custom_blocks_are_emitted_verbatim_and_dedented() {
    let output = generate(
        "type A = string;\n#pydantic\n    def helper() -> int:\n        return 1\n#;\ntype B = A;",
    );
    assert!(
        output.contains("type A = str\n\n\ndef helper() -> int:\n    return 1\n\n\ntype B = A\n"),
        "{output}"
    );
}

#[test]
fn import_paths_are_relative_or_package_absolute() {
    assert_eq!(python_import_path("a/b", "a/c", None), ".c");
    assert_eq!(python_import_path("a/b/c", "d", None), "...d");
    assert_eq!(python_import_path("x", "y/z", None), ".y.z");
    assert_eq!(
        python_import_path("x", "y/z", Some("app.models")),
        "app.models.y.z"
    );
}

#[test]
fn analyzer_rejects_invalid_annotation_arguments_and_python_names() {
    let errors = analyze(concat!(
        "type A = { a: string } @pydantic::extra(\"sometimes\") @pydantic::alias_generator(\"kebab\"); ",
        "type B = string @pydantic::after_validator(\"nomodule\"); ",
        "type list = string; ",
        "type C = { c: string } @pydantic::discriminator(\"kind\");",
    ));
    let messages: Vec<_> = errors.iter().map(|error| error.message.as_str()).collect();
    assert_eq!(errors.len(), 5, "{messages:#?}");
    assert!(messages
        .iter()
        .any(|message| message.contains("\"sometimes\"")));
    assert!(messages.iter().any(|message| message.contains("\"kebab\"")));
    assert!(messages
        .iter()
        .any(|message| message.contains("\"nomodule\"")));
    assert!(messages.iter().any(|message| message.contains("'list'")));
    assert!(messages
        .iter()
        .any(|message| message.contains("unions of models")));
    assert!(analyze("type A = { a: string } @pydantic::extra(\"allow\") @pydantic::after_validator(\".checks.run\");").is_empty());
}

#[test]
fn conditional_validation_is_reported_as_not_enforced() {
    let (_, errors) = generate_with("type A = u8 @if(true, @min(1));", |_| {});
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("does not enforce conditional @if")),
        "{errors:#?}"
    );
}
