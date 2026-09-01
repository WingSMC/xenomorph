# Examples

## Numeric literals

Use `as` when a literal needs a specific signedness, width, or floating-point representation. Without `as`, Xenomorph keeps the exact arbitrary-precision value and records the smallest suitable representation.

```xen
type InferredSmallInteger = 1;
type SignedStatus = 1 as i32;
type UnsignedId = 1 as u64;
type SinglePrecisionRatio = 1.5 as f32;
type ExactDecimal = 1.234567890123456789 as decimal;

type Limits = {
	retries: 3 as u8,
	max_id: 18446744073709551615 as u64,
	ratio: 1.5 as f32,
};

type PositiveId = u64 @min(1 as u64);
```

Casts are checked. For example, `256 as u8`, `-1 as u8`, and a decimal that loses precision as `f32` are errors. See [Grammar: Numeric literal representation and casts](GRAMMAR.md#numeric-literal-representation-and-casts) for inference and generator rules.

## Lists

Lists can be made of literals and types.
If a list has a type in it you can no longer use it as a value.

Lets say we have two lists, one of string literals (literals can be considered sub-types) and one of types.
Usually in place of the types you can use the right side of the `=` operators.

```ts
type ActuatorInterface = ["axles", "vehicle", "wheel-individual"]
type Tuple = [
	ActuatorInterface,
	string,
	number,
	number,
]

// You can validate tuple fields
type EventTuple = [
	EventList,
	string @len(15..300) | null,
	number, // Price
]
```

You can constrain lists and do operations on them.

- `set` enforces unique values
	- `*` intersection
	- `\` difference
	- `<>` symmetric difference: `(a \ b) + (b \ a)` same as `(a + b) \ (a * b)`
- `+` concatenation (union for sets)

The `set` keyword produces and error in declarations with duplicates, but when used on pre-existing types it just creates new types without the duplicates.

```ts
type SomeIncorrectSet = set ["axles" "service-request" "axles"] // -> Error
type SomeList = ["axles" "service-request" "axles"]
type SomeSet = set SomeList // -> ["axles" "service-request"]
```

```ts
type SomeOtherInterface = set ["axles", "service-request"]
type AllInterfaces = SomeOtherInterface + set ActuatorInterface
// -> ["axles" "service-request" "vehicle" "wheel-individual"]
```

## Structs & Enums

```ts

type User = {
	// by default it will use string for other targets
	// if it is generating a mongo orm it will use string
	// if it is an SQL ORM it will use u64
	_id: 
		| string
		| @Mongo:id
		| @SQL:primary(u64),
	name: string /^[A-Z]{3,5}_[0-9]{1,3}$/ @len(5..20),
	age: u8 @min(13) @max(127) @if(lt(18), $adultContent +false),
	utype: UserType,
	// if tests the languageFilter boolean value implicitly
	// so a validator expression is not needed
	languageFilter: bool @if(_, $age +@min(16)) @else($age +@min(12)),
	adultContent: bool,
}

// Action user contains algebraic data just like rust enums
type UserType = enum {
	Admin: -1,
	Basic: 1,
	Premium: 2,
}
type Action = enum {
	// DeleteUser contains same type as User._id, it's
	// own type is an auto generated integer much like in rust
	DeleteUser: User._id, // in memory [0, User._id]
	EditUser: User._id,   // in memory [1, User._id]
	QueryLogs: {from: Date, to: Date},  // in memory [2, Date, Date]
}

type Admin = User & {
	actions: ActionEntry[] @minlen(1) @maxlen(10),
}

type ActionEntry = [Date, Action]

```
