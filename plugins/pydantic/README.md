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

| Setting   | Type   | Purpose                                                                                                                                                                                     |
| --------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `output`  | string | Target directory for generated `.py` files. Relative paths start at the workspace root. Module subdirectories are created below this directory. If omitted, output is written next to each source module. |
| `package` | string | Dotted Python package that contains `output`, such as `app.models`. If set, cross-module imports are absolute (`from app.models.shared.ids import UserId`). If omitted, they are relative (`from .shared.ids import UserId`). |
| `extra`   | string | Default `extra` behaviour of every generated model: `"allow"`, `"ignore"`, or `"forbid"`. `@pydantic::extra` overrides it per model. If omitted, Pydantic's default (`"ignore"`) applies.   |

### Requirements

- Python 3.12 or later. The output uses PEP 695 `type` aliases and generic
  classes.
- Pydantic 2.x.
- [`email-validator`](https://pypi.org/project/email-validator/) if the schema
  uses `email` (`pip install "pydantic[email]"`).

## Generated code

The plugin generates these Python constructs:

| Xenomorph declaration                               | Python output                                                          |
| --------------------------------------------------- | ---------------------------------------------------------------------- |
| Struct `{ ... }`                                    | `class Name(BaseModel)`                                                |
| Intersection of structs, `Pick`, `Omit`, `Partial`, `Complete` | `class Name(BaseModel)` with the resolved fields             |
| Generic struct `type Page<T> = { ... }`             | `class Page[T](BaseModel)`                                             |
| Enum of integer values                              | `class Name(IntEnum)`                                                  |
| Enum with a float value                             | `class Name(Enum)`                                                     |
| Other enum                                          | A discriminated union of variant models (see [Enums](#enums))          |
| Any other type                                      | `type Name = ...` alias                                                |

Each module also has an `__all__` list of its `pub` declarations. The
documentation comment of a declaration becomes a docstring (models) or a
comment (aliases). Field documentation becomes `Field(description=...)`.
`@deprecated` becomes `@typing_extensions.deprecated` on models, a
`# Deprecated:` comment on aliases, and `Field(deprecated=...)` on fields that
use a deprecated alias.

### Optional values

`?T` becomes `T | None`. A struct field whose type is optional gets a `None`
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

### Field names

A field name that is not a valid Python attribute name is renamed. The
original name is kept as the JSON alias:

| Xenomorph field          | Python field                               |
| ------------------------ | ------------------------------------------ |
| `"first-name": string`   | `first_name: str = Field(alias="first-name")` |
| `class: string`          | `class_: str = Field(alias="class")`       |
| `_id: uuid`              | `id: UUID = Field(alias="_id")`            |
| `model_type: string`     | `f_model_type: str = Field(alias="model_type")` |

The plugin also renames fields that shadow `BaseModel` attributes, such as
`json` or `copy`. A model with a renamed field gets `populate_by_name=True`.
Serialize with `model_dump(by_alias=True)` to write the original names.

### Builtin types

| Xenomorph                                   | Python                                    |
| ------------------------------------------- | ----------------------------------------- |
| `string`                                    | `str`                                     |
| `char`                                      | `str` with length 1                       |
| `hostname`, `semver`                        | `str` with a pattern                      |
| `bool`                                      | `bool`                                    |
| `number`, `f32`, `f64`                      | `float`                                   |
| `integer`, `bigint`                         | `int`                                     |
| `u8` … `u128`, `i8` … `i128`                | `int` with `ge` and `le` bounds           |
| `decimal`                                   | `Decimal`                                 |
| `uuid`                                      | `UUID`                                    |
| `email`                                     | `EmailStr`                                |
| `url`                                       | `AnyUrl`                                  |
| `ip`, `ipv4`, `ipv6`                        | `IPvAnyAddress`, `IPv4Address`, `IPv6Address` |
| `date`, `datetime`, `duration`              | `date`, `datetime`, `timedelta`           |
| `binary`                                    | `Base64Bytes`                             |
| `regex`                                     | `re.Pattern[str]`                         |
| `any`                                       | `Any`                                     |
| `null`                                      | `None`                                    |
| `T[]`, `set<T>`, `[A, B]`, `Dict<K, V>`     | `list[T]`, `set[T]`, `tuple[A, B]`, `dict[K, V]` |
| Literals and `set ["a", "b"]`               | `Literal[...]`                            |
| `\|A\|B`                                    | `A \| B`                                  |

### Native validators

| Xenomorph       | Pydantic `Field` constraint     |
| --------------- | ------------------------------- |
| `@min(x)`       | `ge=x`                          |
| `@max(x)`       | `le=x`                          |
| `@gt(x)`        | `gt=x`                          |
| `@lt(x)`        | `lt=x`                          |
| `@multipleof(x)`| `multiple_of=x`                 |
| `@len(n)`       | `min_length=n, max_length=n`    |
| `@minlen(n)`    | `min_length=n`                  |
| `@maxlen(n)`    | `max_length=n`                  |
| `@match(/re/)`  | `pattern="re"`                  |

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

| Annotation                              | `ConfigDict` entry                                     |
| --------------------------------------- | ------------------------------------------------------ |
| `@pydantic::extra("allow")`             | `extra="allow"`. Also `"ignore"` or `"forbid"`.        |
| `@pydantic::frozen`                     | `frozen=True`                                          |
| `@pydantic::populate_by_name`           | `populate_by_name=True`                                |
| `@pydantic::validate_assignment`        | `validate_assignment=True`                             |
| `@pydantic::use_enum_values`            | `use_enum_values=True`                                 |
| `@pydantic::str_strip_whitespace`       | `str_strip_whitespace=True`                            |
| `@pydantic::alias_generator("camel")`   | `alias_generator=to_camel`. Also `"pascal"` or `"snake"`. |
| `@pydantic::config("key", value)`       | `key=value` for any other `ConfigDict` key             |

```xen
pub type User = { name: string, "first-name": string }
    @pydantic::extra("allow")
    @pydantic::frozen
    @pydantic::config("ser_json_timedelta", "float");
```

### Models and type aliases

| Annotation                           | Effect                                                                 |
| ------------------------------------ | ---------------------------------------------------------------------- |
| `@pydantic::strict`                  | Disables lax type coercion (`strict=True`).                            |
| `@pydantic::title("Text")`           | Sets the JSON Schema title.                                            |
| `@pydantic::discriminator("field")`  | Validates a union of models as a tagged union keyed by `field`.        |

### Custom validators

These annotations take the dotted import path of a Python function. A path
that starts with `.` is relative to the generated module.

| Annotation                              | On a type alias               | On a model                                         |
| --------------------------------------- | ----------------------------- | -------------------------------------------------- |
| `@pydantic::after_validator("m.f")`     | `AfterValidator(f)`           | `@model_validator(mode="after")`, calls `f(self)`  |
| `@pydantic::before_validator("m.f")`    | `BeforeValidator(f)`          | `@model_validator(mode="before")`, calls `f(data)` |
| `@pydantic::wrap_validator("m.f")`      | `WrapValidator(f)`            | `@model_validator(mode="wrap")`, calls `f(data, handler)` |
| `@pydantic::plain_validator("m.f")`     | `PlainValidator(f)`           | Not supported (error)                              |

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

Generation keeps only `pub` custom blocks of the entry module, like it does for
the other plugins. An empty block causes a warning.

## Limitations

- Inline structs and enums inside other types are not supported. Declare them
  as named types. The plugin reports an error and generates `dict[str, Any]`.
- An intersection of non-struct types is not supported. The plugin reports a
  warning and uses the first member.
- A Python keyword or builtin, such as `list` or `type`, cannot be a type or
  generic parameter name.
