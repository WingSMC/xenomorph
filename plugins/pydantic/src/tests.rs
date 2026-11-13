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
            generator.type_hierarchy.set_declaration_annotations(
                "test",
                name.text(source),
                xenomorph_common::semantic::declaration_annotations(&ty.1, source),
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

/// Exercise the actual generated source when Python 3.12+ and Pydantic v2
/// are installed. Generator-only tests remain usable without Python.
fn assert_python(output: &str, assertions: &str) {
    assert_python_with_setup(output, "", assertions);
}

fn assert_python_with_setup(output: &str, setup: &str, assertions: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let python = if cfg!(windows) { "py" } else { "python3" };
    let available = Command::new(python)
        .args([
            "-c",
            "import sys, pydantic; assert sys.version_info >= (3, 12); assert pydantic.__version__.startswith('2.')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        eprintln!("Skipping Python runtime assertions: Python 3.12+ and Pydantic v2 are required.");
        return;
    }
    let mut child = Command::new(python)
        .args([
            "-c",
            &format!("{setup}\nimport sys; exec(compile(sys.stdin.read(), '<generated>', 'exec'))"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let code = format!("{output}\n{assertions}\n");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(code.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}\n{code}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
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

fn file_config(value: &str) -> Config {
    let mut config = Config::default();
    config.plugins.config.insert(
        NAME.into(),
        format!("{{ {value} }}").parse::<ConfigValue>().unwrap(),
    );
    config
}

#[test]
fn configured_file_import_is_used_in_aliases_fields_collections_and_generics() {
    let config = file_config("file_type = { module = 'pydantic', name = 'Base64Bytes' }");
    let source = concat!(
        "type Upload = file; type Uploads = file[]; ",
        "type Box<T> = { value: T }; type UploadBox = Box<file>; ",
        "type Request = { upload: file, optional: ?file, uploads: Uploads, alias: Upload };",
    );
    let (output, errors) = generate_with(source, |generator| {
        generator.file_type = configured_generator(&config).file_type;
    });
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(
        output.contains("from pydantic import Base64Bytes, BaseModel\n"),
        "{output}"
    );
    assert!(output.contains("type Upload = Base64Bytes\n"), "{output}");
    assert!(
        output.contains("type Uploads = list[Base64Bytes]\n"),
        "{output}"
    );
    assert!(
        output.contains("class UploadBox(Box[Base64Bytes]):"),
        "{output}"
    );
    assert!(output.contains("    upload: Base64Bytes\n"), "{output}");
    assert!(
        output.contains("    optional: Base64Bytes | None = None\n"),
        "{output}"
    );
    assert_python(
        &output,
        r#"
from pydantic import ValidationError
request = Request(upload=b'aGVsbG8=', uploads=[b'd29ybGQ='], alias=b'aGVsbG8=')
assert request.upload == request.alias == b'hello'
assert request.uploads == [b'world']
assert request.optional is None
assert UploadBox(value=b'aGVsbG8=').value == b'hello'
assert Request.model_json_schema()['properties']['upload']['format'] == 'base64'
try:
    Request(upload=42, uploads=[], alias=b'aGVsbG8=')
except ValidationError:
    pass
else:
    raise AssertionError('the configured file type must validate input')
"#,
    );
}

#[test]
fn configured_file_import_handles_relative_paths_and_name_collisions() {
    let config = file_config("file_type = { module = '.uploads', name = 'Upload' }");
    let (output, errors) = generate_with(
        "type Upload = string; type Request = { value: file };",
        |generator| {
            generator.file_type = configured_file_type(&config);
        },
    );
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(
        output.contains("from .uploads import Upload as Upload_2\n"),
        "{output}"
    );
    assert!(output.contains("    value: Upload_2\n"), "{output}");
    let (output, _) = generate_with("type Request = { value: file };", |generator| {
        generator.file_type = configured_file_type(&config);
    });
    assert!(output.contains("from .uploads import Upload\n"), "{output}");
    assert!(!output.contains("Upload_2"), "{output}");
    let generator = configured_generator(&config);
    assert_eq!(
        generator.render(&OwnedType::Qualified {
            module_path: None,
            name: "file".into(),
            arguments: Vec::new(),
        }),
        "Upload"
    );
}

#[test]
fn missing_or_invalid_file_config_reports_errors_at_value_references() {
    let source = "type Alias = file; type Uploads = Alias[]; type Box<T> = { value: T }; type Request = { value: Box<file> };";
    let (mut generator, _) = generator_for(source);
    let reference = SimpleType::Identifier(
        TokenData::new(
            source.find("Alias[]").unwrap(),
            source.find("Alias[]").unwrap() + 5,
        ),
        None,
    );
    for config in [
        Config::default(),
        file_config("file_type = 'Upload'"),
        file_config("file_type = { module = 'uploads' }"),
        file_config("file_type = { module = 'uploads', name = 'class' }"),
        file_config("file_type = { module = 'bad..module', name = 'Upload' }"),
        file_config("file_type = { module = 'uploads', name = 'Upload', typo = true }"),
    ] {
        generator.file_type = configured_file_type(&config);
        assert!(generator.file_type.is_err());
        let mut errors = Vec::new();
        generator.on_simple_type(&reference, &mut errors);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert_eq!(errors[0].severity, XenoDiagSeverity::Err);
        assert_eq!(errors[0].location.text(source), "Alias");
        assert!(
            errors[0].message.contains("[plugins.pydantic.file_type]"),
            "{errors:#?}"
        );
    }
    generator.file_type = configured_file_type(&file_config(
        "file_type = { module = 'app.uploads', name = 'Upload' }",
    ));
    let mut errors = Vec::new();
    generator.on_simple_type(&reference, &mut errors);
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        configured_file_type(&file_config(
            "file_type = { module = '..', name = 'Upload' }"
        ))
        .unwrap(),
        ("..".into(), "Upload".into())
    );
}

#[test]
fn file_mapping_is_not_required_for_binary_or_unrelated_types() {
    let (mut generator, _) = generator_for("type Data = binary;");
    let mut errors = Vec::new();
    generator.on_simple_type(
        &SimpleType::Identifier(TokenData::new(12, 18), None),
        &mut errors,
    );
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(generate("type Data = binary;").contains("type Data = Base64Bytes\n"));
    let schema: serde_json::Value = serde_json::from_str(provide_config_schema()).unwrap();
    assert_eq!(
        schema["properties"]["file_type"]["required"],
        serde_json::json!(["module", "name"])
    );
}

#[test]
fn dict_members_allow_extra_keys_and_usetype_overrides() {
    let output = generate(concat!(
        "type Base = { id: string }; ",
        "type Open = &Base&Dict<string, i32>; ",
        "type Picked = &Partial<Base>&Dict<string, bool>; ",
        "type Raw = &Base&Dict<string, any> @pydantic::usetype(\"dict[str, object]\");",
    ));
    assert!(output.contains("class Open(BaseModel):\n    model_config = ConfigDict(extra=\"allow\")\n\n    id: str\n"), "{output}");
    assert!(
        output
            .contains("class Picked(BaseModel):\n    model_config = ConfigDict(extra=\"allow\")\n"),
        "{output}"
    );
    assert!(!output.contains("__pydantic_extra__"), "{output}");
    assert!(
        output.contains("type Raw = dict[str, object]\n"),
        "{output}"
    );
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
fn flattened_fields_import_their_qualified_dependencies() {
    let (output, errors) = generate_with(
        "type Extra = { enabled: bool }; type Result = &Base & Extra;",
        |generator| {
            let source = "pub type Tag = { name: string }; pub type Base = { tags: ?Tag[] };";
            let tokens = Lexer::tokenize(source).unwrap();
            let (ast, diagnostics) = Parser::parse(&tokens, source);
            assert!(diagnostics.is_empty(), "{diagnostics:#?}");
            generator
                .type_hierarchy
                .register_module("test", vec!["other".into()]);
            generator.type_hierarchy.register_module("other", vec![]);
            for declaration in ast {
                if let Declaration::Type {
                    name, generics, ty, ..
                } = declaration
                {
                    generator.type_hierarchy.insert_declaration(
                        "other",
                        name.text(source),
                        TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
                    );
                }
            }
        },
    );
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(output.contains("from .other import Tag\n"), "{output}");
    assert!(
        output.contains("    tags: list[Tag] | None = None\n"),
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
    assert!(
        output.contains("class Users(Page[str]):\n    items: list[str]\n"),
        "{output}"
    );
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
fn defaults_are_preserved_in_reusable_type_annotations() {
    let output = generate(concat!(
        "pub type DefaultFalse = bool @default(false); ",
        "pub type DefaultTrue = bool @default(true); ",
        "type Flags = { describe_images: DefaultFalse, some_flag: DefaultTrue };",
    ));
    assert!(
        output.contains("type DefaultFalse = Annotated[bool, Field(default=False)]\n"),
        "{output}"
    );
    assert!(
        output.contains("type DefaultTrue = Annotated[bool, Field(default=True)]\n"),
        "{output}"
    );
    assert_python(
        &output,
        r#"
assert DefaultFalse.__value__.__metadata__[0].default is False
assert DefaultTrue.__value__.__metadata__[0].default is True
flags = Flags()
assert flags.describe_images is False
assert flags.some_flag is True
assert Flags(describe_images=True, some_flag=False).model_dump() == {
    'describe_images': True, 'some_flag': False,
}
"#,
    );
}

#[test]
fn defaulted_optional_aliases_accept_omission_but_reject_none() {
    let output = generate(concat!(
        "pub type DefaultFalse = ?bool @default(false); ",
        "pub type A = ?bool @default(true); ",
        "type OptBool = ?bool; ",
        "type CsvOptionsOf<Enabled: OptBool> = { csv_enable: Enabled }; ",
        "type CsvOptions = CsvOptionsOf<DefaultFalse>; ",
        "type Flags = { enabled: DefaultFalse, explicit: ?DefaultFalse, affirmative: A, plain: OptBool };",
    ));
    assert!(
        output.contains("type DefaultFalse = Annotated[bool, Field(default=False)]\n"),
        "{output}"
    );
    assert!(
        output.contains("type A = Annotated[bool, Field(default=True)]\n"),
        "{output}"
    );
    assert_python(
        &output,
        r#"
from pydantic import TypeAdapter, ValidationError
assert CsvOptions().csv_enable is False
assert Flags().enabled is False
assert Flags().explicit is False
assert Flags().affirmative is True
assert Flags().plain is None
assert Flags(plain=None).plain is None
assert CsvOptions(csv_enable=True).csv_enable is True
assert TypeAdapter(A).validate_python(False) is False
try:
    TypeAdapter(A).validate_python(None)
except ValidationError:
    pass
else:
    raise AssertionError('defaulted alias accepted None')
for model, payload in [(CsvOptions, {'csv_enable': None}), (Flags, {'enabled': None}), (Flags, {'explicit': None}), (Flags, {'affirmative': None})]:
    try:
        model.model_validate(payload)
    except ValidationError:
        pass
    else:
        raise AssertionError(f'accepted None: {payload!r}')
"#,
    );
}

#[test]
fn default_if_missing_preserves_none_and_applies_on_omission() {
    for bound in ["?bool", "OptBool"] {
        let output = generate(&format!(
            "type OptBool = ?bool; \
             type Missing = ?bool @defaultIfMissing(true); \
             type Copy = Missing; \
             type Plain = bool @defaultIfMissing(false); \
             type NullDefault = ?bool @defaultIfMissing(null); \
             type CsvOptionsOf<Enabled: {bound}> = {{ csv_enable: Enabled }}; \
             type CsvOptions = CsvOptionsOf<Copy>; \
             type Flags = {{ enabled: Copy, explicit: ?Missing, plain: Plain, empty: NullDefault }};"
        ));
        assert!(
            output.contains("type Missing = Annotated[bool | None, Field(default=True)]\n"),
            "{output}"
        );
        assert!(output.contains("    enabled: Copy = True\n"), "{output}");
        assert!(
            output.contains("    explicit: Missing | None = True\n"),
            "{output}"
        );
        assert_python(
            &output,
            r#"
from pydantic import TypeAdapter, ValidationError
assert Flags().model_dump() == {'enabled': True, 'explicit': True, 'plain': False, 'empty': None}
assert Flags(enabled=None, explicit=None).enabled is None
assert Flags(enabled=None, explicit=None).explicit is None
assert Flags(enabled=False).enabled is False
assert Flags(empty=None).empty is None
assert CsvOptions().csv_enable is True
assert CsvOptions(csv_enable=None).csv_enable is None
assert CsvOptions(csv_enable=False).csv_enable is False
assert TypeAdapter(Missing).validate_python(None) is None
try:
    Flags(plain=None)
except ValidationError:
    pass
else:
    raise AssertionError('defaultIfMissing must not make a required type nullable')
"#,
        );
    }
}

#[test]
fn nearest_default_annotation_wins_across_both_kinds() {
    let output = generate(concat!(
        "type Base = bool @default(false); ",
        "type Missing = ?Base @defaultIfMissing(true); ",
        "type Copy = Missing; ",
        "type FirstMissing = ?bool @defaultIfMissing(false) @default(true); ",
        "type FirstDefault = ?bool @default(true) @defaultIfMissing(false); ",
        "type Model = { value: Copy, first_missing: FirstMissing, first_default: FirstDefault };",
    ));
    assert_python(
        &output,
        r#"
from pydantic import ValidationError
assert Model().value is True
assert Model(value=None).value is None
assert Model(value=False).value is False
assert Model().first_missing is False
assert Model(first_missing=None).first_missing is None
assert Model().first_default is True
try:
    Model(first_default=None)
except ValidationError:
    pass
else:
    raise AssertionError('the first default annotation must determine nullability')
"#,
    );
}

#[test]
fn default_if_missing_copies_mutable_values_and_preserves_optional_models() {
    let output = generate(concat!(
        "type Tags = ?string[] @defaultIfMissing([\"a\"]); ",
        "type Options = { debug: ?bool } @defaultIfMissing({ debug: true }); ",
        "type Model = { tags: Tags, options: ?Options };",
    ));
    assert_python(
        &output,
        r#"
first, second = Model(), Model()
assert first.tags == ['a']
assert first.options.debug is True
first.tags.append('b')
first.options.debug = False
assert second.tags == ['a']
assert second.options.debug is True
assert Model(tags=None, options=None).model_dump() == {'tags': None, 'options': None}
"#,
    );
}

#[test]
fn explicit_aliases_accept_both_names_and_expose_a_serialized_property() {
    let output = generate(concat!(
        "pub type SignalType = set [\"input\", \"output\", \"other\", \"keyword\"] ",
        "@default(\"input\") @pydantic::alias(\"type\"); ",
        "type Classification = SignalType; ",
        "pub type SignalInformation = { var_type: Classification } ",
        "@pydantic::before_validator(\"..shared.pydantic_validators.signal_input\");",
    ));
    assert!(
        provide_annotations()
            .iter()
            .any(|annotation| annotation.name == "alias"),
        "@pydantic::alias must be registered"
    );
    assert!(
        output.contains("AliasChoices(\"var_type\", \"type\")"),
        "{output}"
    );
    assert!(
        output.contains("@computed_field(alias=\"type\")\n    @property\n"),
        "{output}"
    );
    assert!(!output.contains("model_serializer"), "{output}");
    assert!(
        output.contains("def type(self) -> Classification:"),
        "{output}"
    );
    assert!(output.contains("return self.var_type"), "{output}");
    assert!(output.contains("return signal_input(data)"), "{output}");
}

#[test]
fn alias_runtime_preserves_source_fields_validators_and_serialization() {
    let output = generate(concat!(
        "type SignalType = set [\"input\", \"output\"] @default(\"input\") @pydantic::alias(\"type\"); ",
        "type Classification = SignalType; ",
        "type Signal = { var_type: Classification, other: ?string } ",
        "@pydantic::extra(\"forbid\") @pydantic::alias_generator(\"camel\") ",
        "@pydantic::before_validator(\"__main__.signal_input\");",
    ));
    assert_python_with_setup(
        &output,
        r#"
def signal_input(data):
    if isinstance(data, dict) and 'var_type' in data:
        return {**data, 'type': data['var_type']}
    return data
"#,
        r#"
import json
from pydantic import ValidationError

assert Signal().var_type == Signal().type == 'input'
assert 'type' in Signal.model_computed_fields
assert 'type' not in Signal.model_fields
assert Signal.model_json_schema(mode='serialization')['properties']['type']['readOnly'] is True
for payload in ({'type': 'output'}, {'var_type': 'output'},
                {'var_type': 'output', 'type': 'input'}):
    original = dict(payload)
    signal = Signal.model_validate(payload)
    assert payload == original
    assert signal.var_type == signal.type == 'output'
    expected = {'var_type': 'output', 'type': 'output', 'other': None}
    assert signal.model_dump() == expected
    assert signal.model_dump(by_alias=True) == expected
    assert json.loads(signal.model_dump_json()) == expected
    assert Signal.model_validate(expected).model_dump() == expected
    assert signal.model_dump(include={'var_type'}) == {'var_type': 'output'}
    assert signal.model_dump(include={'type'}) == {'type': 'output'}
    assert signal.model_dump(exclude={'var_type', 'type'}) == {'other': None}
    assert signal.model_dump(exclude={'type'}) == {'var_type': 'output', 'other': None}
    assert 'type' not in signal.__dict__
    signal.var_type = 'input'
    assert signal.type == 'input'
    assert signal.model_dump()['type'] == 'input'
    assert json.loads(signal.model_dump_json())['type'] == 'input'
    try:
        signal.type = 'output'
    except AttributeError:
        pass
    else:
        raise AssertionError('computed alias must be read-only')

for payload in ({'type': 'invalid'}, {'var_type': 'invalid'}, {'unexpected': 1}):
    try:
        Signal.model_validate(payload)
    except ValidationError:
        pass
    else:
        raise AssertionError(f'accepted invalid input: {payload!r}')
"#,
    );
}

#[test]
fn computed_aliases_keep_explicit_names_and_serialize_current_values() {
    let output = generate(concat!(
        "type Timestamp = datetime @pydantic::alias(\"time_stamp\"); ",
        "type Label = string @pydantic::alias(\"wire-name\"); ",
        "type Event = { created: Timestamp, label: Label } @pydantic::alias_generator(\"camel\");",
    ));
    assert!(
        output.contains("@computed_field(alias=\"time_stamp\")\n    @property\n"),
        "{output}"
    );
    assert_python(
        &output,
        r#"
import json
from datetime import datetime

timestamp = datetime(2026, 11, 12, 8, 30)
event = Event(time_stamp=timestamp, **{'wire-name': 'example'})
assert event.created == event.time_stamp == timestamp
assert event.model_dump() == {
    'created': timestamp, 'time_stamp': timestamp, 'label': 'example', 'wire-name': 'example',
}
assert event.model_dump(by_alias=True) == event.model_dump()
serialized = json.loads(event.model_dump_json(by_alias=True))
assert serialized['created'] == serialized['time_stamp'] == '2026-11-12T08:30:00'
assert 'timeStamp' not in serialized
event.created = datetime(2026, 11, 13, 9, 0)
serialized = json.loads(event.model_dump_json())
assert serialized['created'] == serialized['time_stamp'] == '2026-11-13T09:00:00'
assert Event.model_validate_json(event.model_dump_json()).created == event.created
assert event.model_dump(include={'time_stamp'}) == {'time_stamp': event.created}
assert event.model_dump(exclude={'label'}) == {'created': event.created, 'time_stamp': event.created}
"#,
    );
}

#[test]
fn nullable_and_structured_defaults_remain_in_alias_metadata() {
    let output = generate(concat!(
        "type MaybePort = ?u16 @defaultIfMissing(null); ",
        "type Host = ?string @minlen(1) @default(\"localhost\"); ",
        "type Tags = string[] @default([\"a\"]); ",
        "type Env = Dict<string, integer> @default({ x: 1 }); ",
        "type Fallback = Host; ",
        "type Model = { port: MaybePort, host: Fallback, tags: Tags, env: Env };",
    ));
    assert!(output.contains("type MaybePort = Annotated[Annotated[int, Field(ge=0, le=65535)] | None, Field(default=None)]\n"), "{output}");
    assert!(
        output.contains("type Host = Annotated[str, Field(min_length=1, default=\"localhost\")]\n"),
        "{output}"
    );
    assert!(
        output.contains("type Tags = Annotated[list[str], Field(default=[\"a\"])]\n"),
        "{output}"
    );
    assert!(
        output.contains("type Env = Annotated[dict[str, int], Field(default={\"x\": 1})]\n"),
        "{output}"
    );
    assert_python(
        &output,
        r#"
assert MaybePort.__value__.__metadata__[0].default is None
assert Host.__value__.__metadata__[0].default == 'localhost'
first, second = Model(), Model()
assert first.port is None
assert Model(port=None).port is None
assert first.host == 'localhost'
assert first.tags == ['a']
assert first.env == {'x': 1}
first.tags.append('b')
first.env['x'] = 2
assert second.tags == ['a']
assert second.env == {'x': 1}
from pydantic import ValidationError
try:
    Model(host=None)
except ValidationError:
    pass
else:
    raise AssertionError('non-null default must not admit None')
"#,
    );
}

#[test]
fn alias_metadata_is_scoped_in_imports_and_flattened_intersections() {
    let (output, errors) = generate_with(
        "type Extra = { enabled: bool }; type Copy = &Base&Extra;",
        |generator| {
            generator
                .type_hierarchy
                .register_module("test", vec!["other".into()]);
            generator
                .type_hierarchy
                .register_module("other", vec!["shared".into()]);
            generator.type_hierarchy.register_module("shared", vec![]);
            for (module, source) in [
                (
                    "shared",
                    "type SignalType = string @default(\"input\") @pydantic::alias(\"type\");",
                ),
                (
                    "other",
                    "type Classification = ?SignalType; type Base = { var_type: Classification };",
                ),
            ] {
                let tokens = Lexer::tokenize(source).unwrap();
                let (ast, diagnostics) = Parser::parse(&tokens, source);
                assert!(diagnostics.is_empty(), "{diagnostics:#?}");
                for declaration in ast {
                    if let Declaration::Type {
                        name, generics, ty, ..
                    } = declaration
                    {
                        generator.type_hierarchy.insert_declaration(
                            module,
                            name.text(source),
                            TypeDeclarationInfo::from_ast(generics.as_deref(), &ty.0, source),
                        );
                        generator.type_hierarchy.set_declaration_annotations(
                            module,
                            name.text(source),
                            xenomorph_common::semantic::declaration_annotations(&ty.1, source),
                        );
                    }
                }
            }
        },
    );
    assert!(errors.is_empty(), "{errors:#?}\n{output}");
    assert!(
        output.contains("from .other import Classification\n"),
        "{output}"
    );
    assert!(
        output.contains("default=\"input\", validation_alias=AliasChoices(\"var_type\", \"type\")"),
        "{output}"
    );
    assert!(
        output.contains("def type(self) -> Classification:"),
        "{output}"
    );
}

#[test]
fn nearer_alias_decorators_override_inherited_names() {
    let output = generate(concat!(
        "type Base = string @pydantic::alias(\"first\"); ",
        "type Override = Base @pydantic::alias(\"second\"); ",
        "type Model = { field: ?Override };",
    ));
    assert!(
        output.contains("AliasChoices(\"field\", \"second\")"),
        "{output}"
    );
    assert!(!output.contains("def first("), "{output}");
    assert_python(
        &output,
        r#"
assert Model().second is None
assert Model(second='x').field == 'x'
assert Model(field='x').model_dump() == {'field': 'x', 'second': 'x'}
"#,
    );
}

#[test]
fn conflicting_aliases_are_reported_instead_of_shadowing_fields() {
    for source in [
        "type T = string @pydantic::alias(\"other\"); type M = { field: T, other: string };",
        "type T = string @pydantic::alias(\"duplicate\"); type M = { first: T, second: T };",
        "type T = string @pydantic::alias(\"model_dump\"); type M = { field: T };",
    ] {
        let (_, errors) = generate_with(source, |_| {});
        assert!(
            errors
                .iter()
                .any(|error| error.severity == XenoDiagSeverity::Err
                    && error.message.contains("conflicts")),
            "{errors:#?}"
        );
    }
}

#[test]
fn default_annotation_sets_field_defaults() {
    let output = generate(concat!(
        "type Port = u16 @default(8080); ",
        "type Host = string @default(\"localhost\"); ",
        "type Ratio = f64 @default(0.5); ",
        "type Debug = bool @default(false); ",
        "type MaybePort = ?u16 @defaultIfMissing(null); ",
        "type ServerPort = Port; ",
        "type Name = string @default(\"x\") @deprecated(\"Use Host.\"); ",
        "type S = { port: Port, host: ?Host, ratio: Ratio, debug: Debug, other: MaybePort, server: ServerPort, name: Name, plain: ?string };",
    ));
    assert!(
        output.contains("type Port = Annotated[int, Field(ge=0, le=65535, default=8080)]\n"),
        "{output}"
    );
    assert!(output.contains("    port: Port = 8080\n"), "{output}");
    assert!(
        output.contains("    host: Host = \"localhost\"\n"),
        "{output}"
    );
    assert!(output.contains("    ratio: Ratio = 0.5\n"), "{output}");
    assert!(output.contains("    debug: Debug = False\n"), "{output}");
    assert!(output.contains("    other: MaybePort = None\n"), "{output}");
    assert!(
        output.contains("    server: ServerPort = 8080\n"),
        "{output}"
    );
    assert!(
        output.contains("    name: Name = Field(default=\"x\", deprecated=\"Use Host.\")\n"),
        "{output}"
    );
    assert!(
        output.contains("    plain: str | None = None\n"),
        "{output}"
    );
}

#[test]
fn structured_defaults_render_lists_dicts_and_model_factories() {
    let output = generate(concat!(
        "type Tags = string[] @default([\"a\", \"b\"]); ",
        "type Empty = string[] @default([]); ",
        "type Env = Dict<string, i32> @default({ \"x\": 1 }); ",
        "type Options = { debug: ?bool } @default({}); ",
        "type Named = { name: ?string } @default({ name: \"x\" }); ",
        "type S = { tags: Tags, empty: Empty, env: Env, options: Options, named: ?Named };",
    ));
    assert!(
        output.contains("    tags: Tags = [\"a\", \"b\"]\n"),
        "{output}"
    );
    assert!(output.contains("    empty: Empty = []\n"), "{output}");
    assert!(output.contains("    env: Env = {\"x\": 1}\n"), "{output}");
    assert!(
        output.contains("    options: Options = Field(default_factory=Options)\n"),
        "{output}"
    );
    assert!(
        output.contains(
            "    named: Named = Field(default_factory=lambda: Named.model_validate({\"name\": \"x\"}))\n"
        ),
        "{output}"
    );
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
