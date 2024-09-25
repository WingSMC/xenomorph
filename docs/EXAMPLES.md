# Examples

## Lists

Lists can be made of literals and types.
If a list has a type in it you can no longer use it as a value.

Lets say we have two lists, one of string literals (literals can be considered sub-types) and one of types.
Usually in place of the types you can use the right side of the `=` operators.

```ts
type ActuatorInterface = ["axles" "vehicle" "wheel-individual"]
type Tuple = [
	ActuatorInterface
	string
	number
	number
]

// You can validate tuple fields
type EventTuple = [
	EventList
	(string @len(15..300) | null)
	number // Price
]
```

You can constrain lists and do operations on them.

- `set` enforces unique values
	- `*` intersection
	- `\` difference
	- `<>` symmetric difference: `(a \ b) + (b \ a)` same as `(a + b) \ (a * b)`
- `+` concatenation (union for sets)

The `set` keyword produces and error in declarations but when used on pre-existing types it just creates a new types without the duplicates.

```ts
type SomeIncorrectSet = set ["axles" "service-request" "axles"] // -> Error
type SomeList = ["axles" "service-request" "axles"]
type SomeSet = set SomeList // -> ["axles" "service-request"]
```

```ts
type SomeOtherInterface = set ["axles" "service-request"]
type AllInterfaces = SomeOtherInterface + set ActuatorInterface
// -> ["axles" "service-request" "vehicle" "wheel-individual"]
```

## Structs & Enums

```ts
type UserType = enum {
	Admin: -1
	Basic: 1
	Premium: 2
}

type User = {
	_id: string /^[a-f0-9]{24}$/ @SQL:primary
	name: string /^[A-Z]{3,5}_[0-9]{1,3}$/
	
	age: u8 @max(125)
	utype: UserType
	languageFilter: bool @if($age +@min(16)) @else($age +@min(12))
}

type Admin = User + {
	actions struct {
		// ...
	}
}

```
