# Pydantic plugin

The Pydantic plugin generates one Python module of
[Pydantic v2](https://docs.pydantic.dev/) models and type aliases per
Xenomorph module. The generated models enforce Xenomorph's native validators,
such as `@min`, `@maxlen`, and `@match`, and the bounds of sized builtins, such
as `u8`. JSON accepted by the generated models therefore matches JSON accepted
by the Xenomorph schema.

## Configure the generator

Enable the plugin in `xenomorph.toml`:

```toml
[plugins]
plugins = ["xenomorph_pydantic"]

[plugins.pydantic]
output = "./generated/models/"
package = "models"
extra = "forbid"
```

| Setting     | Type   | Purpose                                                                                                                                                                                                                       |
| ----------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `output`    | string | Target directory for generated `.py` files. Relative paths start at the workspace root. Module subdirectories are created below this directory. If omitted, output is written next to each source module.                     |
| `package`   | string | Dotted Python package that contains `output`, such as `app.models`. If set, cross-module imports are absolute (`from app.models.shared.ids import UserId`). If omitted, they are relative (`from .shared.ids import UserId`). |
| `extra`     | string | Default `extra` behaviour of every generated model: `"allow"`, `"ignore"`, or `"forbid"`. `@pydantic::extra` overrides it per model. If omitted, Pydantic's default (`"ignore"`) applies.                                     |
| `file_type` | table  | Uploaded-file mapping with required `module` and `name` strings. Required only when generated value types use `file`.                                                                                                         |

### Uploaded files

`file` represents an uploaded file, not raw `binary` data or a filesystem path.
Choose the Python type explicitly:

```toml
[plugins.pydantic.file_type]
module = "app.uploads"
name = "UploadFile"
```

The generator imports `UploadFile` from `app.uploads` and uses it in aliases,
fields, collections, and generic arguments. Relative modules such as
`.uploads` are supported. Import-name collisions are resolved with an alias.
The imported type must be available in your Python environment and support
Pydantic validation. For arbitrary classes, configure the containing model
with `@pydantic::config("arbitrary_types_allowed", true)` or implement Pydantic
schema support on the class.

Missing or invalid settings produce an error diagnostic at the source type
reference and stop generation; there is no `Any` fallback. Schemas without
`file` do not require this setting. The plugin does not select a web framework
or generate multipart parsing or file serialization.

### Requirements

- Python 3.12 or later. The output uses PEP 695 `type` aliases and generic
  classes.
- Pydantic 2.x.
- [`email-validator`](https://pypi.org/project/email-validator/) if the schema
  uses `email` (`pip install "pydantic[email]"`).

## Generated code

The plugin generates these Python constructs:

| Xenomorph declaration                                          | Python output                                                 |
| -------------------------------------------------------------- | ------------------------------------------------------------- |
| Struct `{ ... }`                                               | `class Name(BaseModel)`                                       |
| Intersection of structs, `Pick`, `Omit`, `Partial`, `Complete` | `class Name(BaseModel)` with the resolved fields              |
| Generic struct `type Page<T> = { ... }`                        | `class Page[T](BaseModel)`                                    |
| Enum of integer values                                         | `class Name(IntEnum)`                                         |
| Enum with a float value                                        | `class Name(Enum)`                                            |
| Other enum                                                     | A discriminated union of variant models (see [Enums](#enums)) |
| Any other type                                                 | `type Name = ...` alias                                       |

Each module also has an `__all__` list of its `pub` declarations. The
documentation comment of a declaration becomes a docstring (models) or a
comment (aliases). Field documentation becomes `Field(description=...)`.
`@deprecated` becomes `@typing_extensions.deprecated` on models, a
`# Deprecated:` comment on aliases, and `Field(deprecated=...)` on fields that
use a deprecated alias.

### Optional values

Without a non-null `@default`, `?T` becomes `T | None`. A struct field whose type is optional gets a `None`
default. The field can then be absent from the JSON input:

```xen
type Nickname = ?string @minlen(1);
type User = { name: string, nickname: Nickname, age: ?u8 };
```

```python
type Nickname = Annotated[str, Field(min_length=1)] | None


class User(BaseModel):
    name: str
    nickname: Nickname = None
    age: Annotated[int, Field(ge=0, le=255)] | None = None
```

A field is optional if its type is `?T`, `null`, a union with an optional or
`null` member, or an alias of such a type. Constraints apply to `T`, never to
`None`.

`model_dump()` keeps absent optional fields as `None`. Use
`model_dump(exclude_none=True)` to omit them again.

### Default values

`@default(value)` on an alias sets the default of every struct field of that
type, also through optional types and alias chains. It replaces the `None`
default of optional fields. The alias carries the default in its `Annotated`
metadata:

```xen
type Port = u16 @default(8080);
type Host = ?string @default("localhost");
type Server = { port: Port, host: Host };
```

```python
type Port = Annotated[int, Field(ge=0, le=65535, default=8080)]
type Host = Annotated[str, Field(default="localhost")]


class Server(BaseModel):
    port: Port = 8080
    host: Host = "localhost"
```

The generator also keeps defaults on model fields: Pydantic does not reliably
propagate field metadata through PEP 695 named aliases. Some Pydantic versions
warn about `default` metadata inside named aliases; the field-level default
ensures that generated models still behave correctly. For nullable aliases,
non-null defaults remove outer `None`: omitted fields receive the default,
but explicit `None` is rejected. For example, `?bool @default(true)` becomes
`Annotated[bool, Field(default=True)]`. `@default(null)` is rejected even for
nullable types. This is target-specific behavior: `@default` does not change
Xenomorph's type compatibility, and TypeScript retains source optionality.

Use `@defaultIfMissing(value)` to preserve nullability while supplying a
default on omission. For example, `type A = ?bool @defaultIfMissing(true);`
generates `Annotated[bool | None, Field(default=True)]`. Fields of type `A`
default to `True` but accept explicit `None` and preserve it. Validation
constraints still apply only to the non-null member. `@defaultIfMissing(null)`
is valid for nullable types.

On a non-nullable type, `@defaultIfMissing` produces a warning suggesting
`@default`, but still applies the default normally; it does not add `None`.
Both annotations support alias chains, generic specializations, and structured
defaults. The nearest default annotation wins (the first matching annotation
within a declaration).

Lists and dicts render as Python literals; Pydantic copies them per instance.
A struct default on a model type becomes a factory:

```xen
type Options = { debug: ?bool } @default({});
type Named = { name: ?string } @default({ name: "x" });
```

```python
    options: Options = Field(default_factory=Options)
    named: Named = Field(default_factory=lambda: Named.model_validate({"name": "x"}))
```

### Field names

A field name that is not a valid Python attribute name is renamed. The
original name is kept as the JSON alias:

| Xenomorph field        | Python field                                    |
| ---------------------- | ----------------------------------------------- |
| `"first-name": string` | `first_name: str = Field(alias="first-name")`   |
| `class: string`        | `class_: str = Field(alias="class")`            |
| `_id: uuid`            | `id: UUID = Field(alias="_id")`                 |
| `model_type: string`   | `f_model_type: str = Field(alias="model_type")` |

The plugin also renames fields that shadow `BaseModel` attributes, such as
`json` or `copy`. A model with a renamed field gets `populate_by_name=True`.
Serialize with `model_dump(by_alias=True)` to write the original names.

### Explicit compatibility aliases

`@pydantic::alias("name")` on a type adds an alternative name to every model
field of that type, including through optional types, alias chains, and
imported or flattened fields:

```xen
pub type SignalType = set ["input", "output", "other", "keyword"]
    @default("input")
    @pydantic::alias("type");
pub type SignalInformation = { var_type: SignalType };
```

The generated model keeps `var_type` as its stored field and accepts either
`var_type` or `type` as input (`AliasChoices("var_type", "type")`). If both
are supplied, `var_type` wins. A read-only `.type` property returns the current
`.var_type` value. The property is decorated with `@computed_field` above
`@property`: only `var_type` is stored, and every access or serialization reads
its current value. There is no cached copy to synchronize.

`model_dump()` and `model_dump_json()` emit both names with the same value,
also with `by_alias=True`. The serialization JSON Schema marks `type` as
read-only. Existing model validators still run. Computed fields follow
Pydantic's native include/exclude behavior: use
`exclude={"var_type", "type"}` to omit both names, or exclude either separately.

The closest alias decorator in a type's alias chain wins. An alias that is
not a Python identifier, or is a Python keyword, still works in input and
output but does not get a dot-access property. Aliases that collide with
another field, another compatibility alias, or reserved model attributes
produce a generation error instead of silently replacing a field.

### Builtin types

| Xenomorph                               | Python                                           |
| --------------------------------------- | ------------------------------------------------ |
| `string`                                | `str`                                            |
| `char`                                  | `str` with length 1                              |
| `hostname`, `semver`                    | `str` with a pattern                             |
| `bool`                                  | `bool`                                           |
| `number`, `f32`, `f64`                  | `float`                                          |
| `integer`, `bigint`                     | `int`                                            |
| `u8` … `u128`, `i8` … `i128`            | `int` with `ge` and `le` bounds                  |
| `decimal`                               | `Decimal`                                        |
| `uuid`                                  | `UUID`                                           |
| `email`                                 | `EmailStr`                                       |
| `url`                                   | `AnyUrl`                                         |
| `ip`, `ipv4`, `ipv6`                    | `IPvAnyAddress`, `IPv4Address`, `IPv6Address`    |
| `date`, `datetime`, `duration`          | `date`, `datetime`, `timedelta`                  |
| `binary`                                | `Base64Bytes`                                    |
| `file`                                  | Imported type configured by `file_type`          |
| `regex`                                 | `re.Pattern[str]`                                |
| `any`                                   | `Any`                                            |
| `null`                                  | `None`                                           |
| `T[]`, `set<T>`, `[A, B]`, `Dict<K, V>` | `list[T]`, `set[T]`, `tuple[A, B]`, `dict[K, V]` |
| Literals and `set ["a", "b"]`           | `Literal[...]`                                   |
| `\|A\|B`                                | `A \| B`                                         |

### Native validators

| Xenomorph        | Pydantic `Field` constraint  |
| ---------------- | ---------------------------- |
| `@min(x)`        | `ge=x`                       |
| `@max(x)`        | `le=x`                       |
| `@gt(x)`         | `gt=x`                       |
| `@lt(x)`         | `lt=x`                       |
| `@multipleof(x)` | `multiple_of=x`              |
| `@len(n)`        | `min_length=n, max_length=n` |
| `@minlen(n)`     | `min_length=n`               |
| `@maxlen(n)`     | `max_length=n`               |
| `@match(/re/)`   | `pattern="re"`               |

A validator cannot loosen a builtin bound. For example, `u8 @min(18)` becomes
`Annotated[int, Field(ge=18, le=255)]`, and `i8 @min(-500)` keeps `ge=-128`.

The plugin does not enforce conditional validators (`@if`, `@elseif`,
`@else`). It reports a warning for each one. Use a
[Pydantic validator](#pydantic-annotations) to enforce such rules.

Pydantic's default regex engine (Rust `regex`) does not support lookarounds or
backreferences. To use them, set
`@pydantic::config("regex_engine", "python-re")` on the model.

### Enums

An enum with only integer values becomes an `IntEnum`:

```xen
type Status = enum { Ready: 1, Failed: 2 };
```

```python
class Status(IntEnum):
    Ready = 1
    Failed = 2
```

Every other enum becomes one model per variant and a union discriminated by
`kind`. This matches the `{ "kind": ..., "value": ... }` wire format of the
other generators:

```xen
type Shape = enum { Circle: f64, Square: ?u8 };
```

```python
class Shape_Circle(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Literal["Circle"] = "Circle"
    value: float


class Shape_Square(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Literal["Square"] = "Square"
    value: Annotated[int, Field(ge=0, le=255)] | None = None


type Shape = Annotated[Shape_Circle | Shape_Square, Field(discriminator="kind")]
```

## Pydantic annotations

The plugin adds these annotations in the `pydantic` namespace.

### Model configuration

These annotations apply only to struct-like declarations. They set entries in
the model's `model_config = ConfigDict(...)`.

| Annotation                            | `ConfigDict` entry                                        |
| ------------------------------------- | --------------------------------------------------------- |
| `@pydantic::extra("allow")`           | `extra="allow"`. Also `"ignore"` or `"forbid"`.           |
| `@pydantic::frozen`                   | `frozen=True`                                             |
| `@pydantic::populate_by_name`         | `populate_by_name=True`                                   |
| `@pydantic::validate_assignment`      | `validate_assignment=True`                                |
| `@pydantic::use_enum_values`          | `use_enum_values=True`                                    |
| `@pydantic::str_strip_whitespace`     | `str_strip_whitespace=True`                               |
| `@pydantic::alias_generator("camel")` | `alias_generator=to_camel`. Also `"pascal"` or `"snake"`. |
| `@pydantic::config("key", value)`     | `key=value` for any other `ConfigDict` key                |

```xen
pub type User = { name: string, "first-name": string }
    @pydantic::extra("allow")
    @pydantic::frozen
    @pydantic::config("ser_json_timedelta", "float");
```

### Models and type aliases

| Annotation                          | Effect                                                          |
| ----------------------------------- | --------------------------------------------------------------- |
| `@pydantic::strict`                 | Disables lax type coercion (`strict=True`).                     |
| `@pydantic::title("Text")`          | Sets the JSON Schema title.                                     |
| `@pydantic::discriminator("field")` | Validates a union of models as a tagged union keyed by `field`. |

### Custom validators

These annotations take the dotted import path of a Python function. A path
that starts with `.` is relative to the generated module.

| Annotation                           | On a type alias      | On a model                                                |
| ------------------------------------ | -------------------- | --------------------------------------------------------- |
| `@pydantic::after_validator("m.f")`  | `AfterValidator(f)`  | `@model_validator(mode="after")`, calls `f(self)`         |
| `@pydantic::before_validator("m.f")` | `BeforeValidator(f)` | `@model_validator(mode="before")`, calls `f(data)`        |
| `@pydantic::wrap_validator("m.f")`   | `WrapValidator(f)`   | `@model_validator(mode="wrap")`, calls `f(data, handler)` |
| `@pydantic::plain_validator("m.f")`  | `PlainValidator(f)`  | Not supported (error)                                     |

```xen
pub type Slug = string @pydantic::after_validator("app.checks.slugify");
pub type Range = { start: u32, end: u32 } @pydantic::after_validator("app.checks.ordered");
```

```python
# app/checks.py
def ordered(model):
    if model.start > model.end:
        raise ValueError("start must not be greater than end")
    return model
```

## Embed Python code

A `#pydantic ... #;` block is copied into the generated module at its source
position. The plugin removes the common indentation of the block:

```xen
pub #pydantic
    def display_name(user: "User") -> str:
        return user.name.title()
#;
```

Generation keeps `pub` custom blocks in the entry module and its imported
modules. Private blocks are omitted. A module containing only public Python
helpers can be included with `pub import helpers::*;`; validators can then
refer to functions in that generated module. An empty block causes a warning.

## Limitations

- Inline structs and enums inside other types are not supported. Declare them
  as named types. The plugin reports an error and generates `dict[str, Any]`.
- An intersection of non-struct types is not supported. The plugin reports a
  warning and uses the first member.
- A Python keyword or builtin, such as `list` or `type`, cannot be a type or
  generic parameter name.
