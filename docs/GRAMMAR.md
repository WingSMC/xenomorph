# Descriptors

## Types

- Bool: `bool`
	- literals: `true`, `false`
- Number `number`:
	- Integers: e.g `i8`, `u64`, ...
		- Literal: any number that doesn't fit the criteria of the other number types
	- BigInt: `bigint`
		- Literal: number ending with `b`
	- Float: `f32`, `f64`
		- Literal: `f64: 3.141592653`, `f32: 2.71f` notice the `f`
- String:
	- Unicode: `string`

## Complex types

- Struct: `{...}` or with [generics](#Generics)  `<Generics>` `{}`
- Enum object `enum {...}` or with types or literals
- Enum lists: `[a, b, c]` with literals or with types `(<Types>)[]` or `[string, i32]`

## Builtin validators:

The validators that are function-like that have the signiture `name(...args)` need a `@` prefix in the schema.

Validators are executed in the order they are written in most contexts, but the plugins can alter this behavior.

## Validation operators

- `(...)` for grouping validators
- `not` inverts the result e.g. `not empty`
- `or` or `|`
- `xor` or `^`

## Single field

### Common validators

- `<literal>` are they equal e.g. `string "exact"`
- `in(<enum, list>)` is in enumeration/list
- `empty` for strings and arrays
- Range: `a..b` or `a.<b` or `a<.b` or `a<.<b` where `a` and `b` are `<number> or <string>`
- `only(<list>)` restrict fields to plugins
- `exclude(<list>)` opposite of `only`

### Number / Integer / BigInt / Float / Decimal validation

- `min(<number>)` use this as lte
- `max(<number>)` use this as gte
- `gt(<number>)`
- `lt(<number>)`
- `multipleof(<integer>)` you can test precision with this e.g. `0.031 multipleof(0.001)` is true.<br>*might break down if you write something too small*

### String

- `/regex/flags`
- `minlen(<number>)`
- `maxlen(<number>)`
- `len(<number>)`
- `len(<range>)` is in range

## Multi field

Other fields are referenced by their name after a `$` prefix.

- `=(<$field>)` equal to another field
- `!=(<$field>)` not equal to another field
- `>(<$field>)` greater than another field
- `<($field>)` less than another field
- `>=(<$field>)` greater than or equal to another field
- `<=(<$field>)` less than or equal to another field
- `in(<$field>)` is in another field
- `not in(<$field>)` is not in another field
- `if(<$field> ([+-]<validator>)*)`	can be used on bool fields
- `if(<validator>*, <$field> ([+-]<validator>)*)` can be used on any field with the appropriate validators
- `elseif(<$field> ([+-]<validator>)*)` can be used right after `if` to specify the else if condition
- `else(<$field> ([+-]<validator>)*)` can be used right after `if` to specify the else condition
