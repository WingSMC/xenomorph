# Language reference

This document describes the implemented language. Declarations end with `;`.
Use commas between struct fields, enum variants, tuple members, set prefill
values, type arguments, and annotation arguments. Closed lists accept a trailing
comma. Newlines do not replace commas or semicolons.

## Modules, imports, and exports

Declarations are private to their module by default. Prefix a declaration with
`pub` to export it. Visibility belongs to every declaration kind.

```xen
// folder/models.xen
type InternalId = string;
pub type User = { id: InternalId };
pub type Status = enum { active: true, inactive: false };

// index.xen
import folder::models::{User, Status};
pub type Api = { user: User, status: Status };
```

Import paths start at the configured entry file's directory. Use `::` between
directories, the module filename without `.xen`, and the selected declarations.
`import folder::models::User;` selects one declaration. Braced lists accept a
trailing comma and can span multiple lines. Only named public declarations are
visible to an importer; importing a module does not expose its other names.

`pub import folder::models::{User, Status};` imports and re-exports those names.
Consumers can import them through this module. Re-exports retain the original
declaration's identity for navigation and semantic resolution.

The configured entry module's public declarations and public imports are the
bundle roots. Generators and plugin analyzers receive those roots and their
transitive dependencies, including private helper declarations. Merely importing
a declaration or exporting one from another module does not include it in the
bundle. Dependencies include type arguments, generic constraints, and type
references in annotation arguments. The core analyzer marks declarations outside
this bundle as unnecessary, and the LSP uses those diagnostics to fade them.

The analyzer warns about each imported name that no local declaration references.
References in private declarations count as usage, even if those declarations
are outside the entry bundle. Public imports are exempt because they re-export
their selections. In a partially used braced list, only the unused selections
receive warnings.

## Types

Plugin types use `namespace::Type`, plugin annotations use
`@namespace::Annotation`, and plugin trait constraints use `namespace::Trait`.
Each reference has one namespace segment and contains no whitespace around
`::`. Core names such as `string`, `Numeric`, and `@min` remain unqualified.

- Bool: `bool`
    - literals: `true`, `false`
- Number `number`:
    - Integers: `i4`, `i8`, `i16`, `i32`, `i64`, `i128`, `u4`, `u8`, `u16`, `u32`, `u64`, `u128`, `integer`, and `bigint`
    - Floating point: `f32`, `f64`, `number`, and `decimal`
    - Integer literal: an optional `-` followed by decimal digits, for example `0`, `42`, or `-129`
    - Floating-point literal: an optional `-`, decimal digits, `.`, and one or more fractional digits, for example `3.14`
    - Optional representation: append `as` and a compatible numeric type, for example `1 as i32`, `1 as u64`, `3.14 as f64`, or `0.1 as decimal`
- String:
    - Unicode: `string`

## Numeric literal representation and casts

Numeric values are parsed without loss: integer values use arbitrary-precision integers and decimal values use arbitrary-precision decimals. The AST also records the representation required by each literal.

`as` is a literal type ascription, not a truncating runtime conversion. It is valid only directly after a numeric literal. The parser reports an error when the literal does not fit the requested integer range or when an `f32`/`f64` value cannot preserve the literal's decimal value after a shortest-string round trip.

```xen
type RetryCount = 3 as u8;
type StatusCode = 200 as i32;
type ObjectId = 1 as u64;
type Ratio = 1.5 as f32;
type ExactRate = 1.234567890123456789 as decimal;
```

Integer casts select signedness and width. The width is in bits, so `u32` requires 32 bits (4 bytes). `bigint` selects arbitrary precision. Without a cast, the AST stores the smallest exact bit count: non-negative values are unsigned, negative values are signed using the minimum two's-complement width, and zero requires one bit.

Floating-point literals store their decimal precision and scale. Without a cast, representation is inferred in this order:

1. `f32` when converting to `f32` and back produces the same decimal value.
2. `f64` when the equivalent `f64` round trip succeeds.
3. `decimal` when neither binary representation round-trips.

An explicit `as f32` or `as f64` must pass the corresponding round-trip check. `as decimal` always preserves a successfully parsed finite decimal literal.

### Generator behavior

- **TypeScript:** integer literals within the safe integer range use `number`; wider inferred integers and explicit `i64`, `u64`, `i128`, `u128`, or `bigint` literals use `bigint`. Floating-point literals use `number`. Generation fails if a floating-point literal cannot round-trip through an IEEE-754 64-bit `number`.
- **Java:** literals use the smallest lossless boxed primitive (`Byte`, `Short`, `Integer`, `Long`, `Float`, or `Double`) and fall back to `BigInteger` or `BigDecimal`. Generation reports an error instead of emitting a value that violates its recorded representation.
- **JSON Schema:** numeric `const`, `minimum`, and `maximum` values are emitted as exact JSON numbers, including 64-bit and 128-bit integer bounds. JSON Schema has no standard integer-width or decimal-precision type, so explicit casts add the available numeric type and range constraints; downstream JSON parsers can still impose their own precision limits.

## Custom code blocks

A custom declaration starts and ends with matching runs of `#` and requires a
trailing semicolon. Its first word is a handler identifier, using the language's
usual identifier rules. Whitespace before that identifier is allowed.

```xen
#java class Example {}#;
##sql SELECT '#', id FROM examples;##;
pub #java class Exported {}#;
```

The lexer treats the entire fenced block as one opaque token. A maximal run of
`#` closes the block only when its length matches the opening run. Other runs,
quotes, comments, punctuation, and line breaks remain raw content. Use a longer
fence when the body contains a run that would close it. There is no escape syntax.

Plugins register handler identifiers such as `java` or `sql`. These identifiers
are independent of declaration namespaces, and duplicate registrations are load
errors. Unknown handlers produce warnings. A handler consumes the body and can
store parsed data in its plugin state for generation. Parsing returns a unit
result or a diagnostic. A `pub` prefix is passed to the handler as a flag; the
plugin determines its meaning.
Formatting preserves the fenced content verbatim.

## Complex types

| Form                 | Example right-hand side          |
| -------------------- | -------------------------------- |
| Struct               | `{ name: string, active: bool }` |
| Enum                 | `enum { Ready: 1, Failed: 2 }`   |
| Tuple                | `[string, i32, ?bool]`           |
| Array                | `string[]` or `User[]`           |
| Typed set            | `set<string>`                    |
| Prefilled set        | `set ["red", "green"]`           |
| Typed, prefilled set | `set<u8> [1, 2, 3]`              |
| Sum / union          | `\|string\|u32`                  |
| Intersection         | `&First&Second`                  |

Struct fields, enum variants, tuple members, and type arguments contain simple
types: a literal or type identifier, an optional `?` prefix, or an identifier's
array form. Type identifiers can include type arguments. Nested composites need
named declarations:

```xen
type Address = { street: string };
type Row = string[];
type Matrix = Row[];
type User = { address: Address, names: ?Row };
```

`?Row` makes the entire value optional. The array suffix applies to identifiers,
including generic references such as `Box<string>[]`; literals cannot take it.
Use aliases for additional array dimensions rather than `string[][]`.

### Sets

`set` requires an element type in `<...>`, a literal prefill in `[...]`, or both.
`set TypeName` and bare `set` are invalid. Prefills contain comma-separated
literals; type names cannot appear as prefill values. Duplicate values are
semantic errors, including equal numeric literals with different casts.

```xen
type Names = set<string>;
type Colors = set ["red", "green"];
type Sizes = set<u16> [1, 300,];
type EmptyNames = set<string> [];
```

Without an explicit element type, a nonempty prefill uses its first value's type
as the element type. Later values must fit that type. Use an explicit element
type when later values need a wider range. Collection operations such as `+`,
`*`, `\`, and `<>` are not implemented.

### Generics

Generic parameters follow the declaration name. A parameter can have one type or
trait constraint after `:`. Separate parameters and arguments with commas:

```xen
type Box<T> = { value: T };
type Increment<T: Numeric> = T @multipleof(0.1);
type Label<T: HasLength> = T @minlen(1);
type TextBox = Box<string>;
type Tenths = Increment<decimal>;
type NonemptyText = Label<string>;
```

Built-in utilities include `Pick<T, Keys>`, `Omit<T, Keys>`, `Keyof<T>`,
`Required<T>`, `Partial<T>`, and `Complete<T>`. `Pick`, `Omit`, `Keyof`,
`Partial`, and `Complete` require a struct. `Pick` and `Omit` select keys from a
sum of string literals. `Required<T>` removes outer optionality from any type.

```xen
type Profile = { name: string, enabled: ?bool };
type NameKeys =
    | "name";
type NameOnly = Pick<Profile, NameKeys>;
type WithoutName = Omit<Profile, NameKeys>;
type ProfileKeys = Keyof<Profile>;
type OptionalProfile = Partial<Profile>;
type CompleteProfile = Complete<Profile>;
type OptionalText = ?string;
type PresentText = Required<OptionalText>;
```

### Union, intersection, and bottom types

A sum type starts with `|` to make parsing obvious. A value of `|A|B` can be either `A` or `B`, so only constraints and ancestors guaranteed by **both** members are available on the sum. A constraint implemented by only one member is optional and cannot be
applied to the sum as a whole.

An intersection type starts with `&`. A value of `&A&B` must be both `A` and
`B`, so it inherits the union of the constraints and ancestors provided by its
members. A constraint guaranteed by either member is therefore available on
the intersection.

```xen
type Alphanumeric = string
    @match(/[0-9a-f]+/);
// both members derive from string, the decorator is applied to the whole type.
type Text =
    |uuid
    |Alphanumeric
    @deprecated;
type Mixed =
    | uuid
    | u8;
type NumericText =
    & f32
    & integer; // inherits constraints from both
```

`NEVER` is the bottom type. It has no possible values and is a subtype of every
type, including source-declared types. Its universal parentage is resolved by
the source type hierarchy; it is not stored as a generated static parent list.

The analyzer proves incompatibility conservatively. A provably empty
intersection such as `& u8 & string` resolves to `NEVER` and produces an error at
the source intersection. Intersections that might overlap are retained when
the hierarchy cannot prove that they are empty.

### Quoted field names

Struct and enum keys can be quoted when their wire name is not a Xenomorph
identifier or conflicts with a Xenomorph keyword:

```xen
type Automation = { enabled: bool };
type ExecutionMode = {
        "ecu.test": ?Automation,
        "type": string,
};
```

The parser normalizes quoted keys to their unquoted value. Targets retain that
value as the serialized property name:

- JSON Schema uses the exact key in `properties`.
- TypeScript emits an identifier property when legal and a quoted property
  otherwise.
- Java replaces characters that are not legal in a Java member name with `_`
  and adds Gson `@SerializedName` with the exact wire key. For example,
  `"ecu.test"` becomes `@SerializedName("ecu.test")` on `ecu_test`.

Language targets report an error when a schema identifier conflicts with a
native reserved keyword in a context that requires a native identifier. Java
also reports keyword clashes for fields and enum variants. Quoted wire keys do
not bypass these checks when the target must emit them as native identifiers.

## Annotation placement

Annotations belong to the full type expression on the right-hand side of a
top-level named `type` declaration. Simple types used in struct and enum
fields, tuple elements, and type arguments cannot carry annotations. They
contain identifiers or literals, optionality, and array forms only.

Declare an annotated type first, then reference it where a simple type is
required:

```xen
type Milliseconds = u32 @multipleof(10);
type Tenths = decimal @multipleof(0.1);
type Timeline = { elapsed_ms: Milliseconds, elapsed_seconds: ?Tenths };
```

Writing `elapsed_ms: u32 @multipleof(10)` directly inside the struct is invalid.
This is a deliberate language restriction that keeps annotation applicability
and plugin interpretation easier to verify.

## Deprecation metadata

`@deprecated` marks a named type declaration as deprecated. It accepts an optional
string literal reason and can apply to any type. It does not change type
compatibility, validate runtime values, or emit compiler warnings when the type
is used. You can also provide replacement advice in the declaration's documentation:

```xen
/** Use Timeline instead. */
type LegacyTimeline = { elapsed: u32 } @deprecated("Use Timeline instead.");
```

Plugins preserve the marker using their target's standard representation:

| Target      | Generated metadata                                                                 |
| ----------- | ---------------------------------------------------------------------------------- |
| JSON Schema | `"deprecated": true` on the type's schema                                          |
| TypeScript  | JSDoc `@deprecated` on each generated declaration                                  |
| Java        | `@Deprecated` and Javadoc `@deprecated` on generated classes, enums, and constants |

TypeScript and Java include the reason in the documentation tag. JSON Schema
uses the boolean keyword; put additional prose in the declaration's documentation
to include it in the schema's `description`. Both `@deprecated` and
`@deprecated()` omit the reason.

Java scalar aliases are expanded into their underlying types and have no separate
declaration to mark. Fields that reference a deprecated type are not themselves
marked deprecated. The annotation placement rules above also apply to
`@deprecated`.

## Built-in annotations

Annotations carry metadata for plugins to interpret. The compiler checks
their target and argument constraints; runtime data validation is generated or
performed by a plugin. Annotation order is retained in the AST.

| Annotation                                        | Applicable target            | Explicit arguments           |
| ------------------------------------------------- | ---------------------------- | ---------------------------- |
| `@min(value)`, `@max(value)`                      | `Numeric`                    | One numeric literal          |
| `@gt(value)`, `@lt(value)`                        | `Numeric`                    | One numeric literal          |
| `@multipleof(value)`                              | `Numeric`                    | One positive numeric literal |
| `@len(value)`, `@minlen(value)`, `@maxlen(value)` | `HasLength`                  | One integer literal          |
| `@match(pattern)`                                 | `string` and its descendants | One regex literal            |
| `@deprecated(reason)`                             | Any type                     | Optional string literal      |
| `@if(condition, expression)`                      | Any type                     | Two expressions              |
| `@elseif(condition, expression)`                  | Any type                     | Two expressions              |
| `@else(expression)`                               | Any type                     | One expression               |

Targets are checked against the registered type or trait constraint. For example,
`@minlen` applies to values implementing `HasLength`, including strings and
arrays. Numeric annotations accept integer and decimal literals, including
compatible numeric casts.

`@multipleof(0.1)` describes increments of one tenth; `@multipleof(10)` describes
increments of ten. The argument must be greater than zero and is stored exactly.
Plugins that generate floating-point validators must account for the target's
precision. The JSON Schema plugin emits the `multipleOf` keyword.

Regex literals are annotation arguments, for example
`type Code = string @match(/^[A-Z]+$/);`. A bare `/regex/` after a type is invalid.

Annotation expressions can contain types, literals, regexes, and nested
annotations. `@elseif` must immediately follow `@if` or another `@elseif`, and
`@else` must follow `@if` or `@elseif`. Nested annotations do not interrupt the
outer chain. The compiler checks these signatures and the chain order; plugins
determine how to interpret the conditional metadata.

```xen
type ConditionalNumber = number
    @if(true, @min(1))
    @elseif(false, @min(2))
    @else(@min(0));
```

Runtime field references such as `$field`, validator operators such as `not`
and `xor`, and built-ins such as `eq`, `in`, `empty`, `only`, and `exclude` are
not implemented. Unknown annotation names produce warnings rather than adding
core behavior. Draft ideas are recorded separately in
[Unimplemented design sketches](NOT_SURE_YET.md).
