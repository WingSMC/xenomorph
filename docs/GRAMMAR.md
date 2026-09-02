# Descriptors

## Types

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

## Complex types

- Structs/Enums: `{...}` or with [generics](#Generics) `<Generics>` `{}`
- Lists/Tuples: `[a, b, c]` with literals or types, for example `[string, i32]`
- Arrays use postfix syntax: `Type[]`, for example `string[]` or `User[]`

### Quoted field names

Struct and enum keys can be quoted when their wire name is not a Xenomorph
identifier or conflicts with a Xenomorph keyword:

```xen
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

## Builtin validators:

The validators that are function-like that have the signiture `name(...args)` need a `@` prefix in the schema.

Validators are executed in the order they are written in most contexts, but the plugins can alter this behavior.

## Validation operators

- `( )` for grouping validators
- `not` inverts the result e.g. `not empty`
- `or` or `|`
- `xor` or `^`

## Single field

### Common validators

- `<literal>` are they equal e.g. `string "exact"`
- `in(<enum, list>)` is in enumeration/list
- `empty` for strings and arrays
- Range: `a..b` or `a.<b` or `a<.b` or `a<.<b` where `a` and `b` are `<number>`
- `only(<plugin_list>)` restrict fields to plugins
- `exclude(<plugin_list>)` opposite of `only`

### Number / Integer / BigInt / Float / Decimal validation

- `min(<number>)` use this as lte
- `max(<number>)` use this as gte
- `gt(<number>)`
- `lt(<number>)`
- `multipleof(<number>)` you can test precision with this e.g. `0.031 multipleof(0.001)` is true.<br>_might break down if you write something too small_

### String

- `/regex/`
- `match(<regex>)` requires a string or a type derived from `string` to match the regular expression
- `minlen(<number>)`
- `maxlen(<number>)`
- `len(<number>)`
- `len(<range>)` is in range

## Multi field

Other fields are referenced by their name after a `$` prefix.

- `eq(<$field>)` equal to another field
- `neq(<$field>)` not equal to another field
- `gt(<$field>)` greater than another field
- `lt($field>)` less than another field
- `min(<$field>)` greater than or equal to another field
- `max(<$field>)` less than or equal to another field
- `in(<$field>)` is in another field (list)
- `not in(<$field>)` is not in another field
- `if(_ , (<$field> ([+-]<validator>)*)*)` can be used on bool fields
- `if(<validator>*, (<$field> ([+-]<validator>)*)*)` can be used on any field with the appropriate validators
- `elseif(<validator>*, <$field> (([+-]<validator>)*)*)` can be used right after `if` to specify the else if condition
- `else(<$field> ([+-]<validator>)*)` can be used right after `if` to specify the else condition
