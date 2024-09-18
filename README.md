# Xenomorph (eXtensible ENtity & Object MOdel Relation PHrocessor)

## Examples

```ts
// ----------------- lists.xen
type VafActuatorInterface set [
	"axles"
	"driver-brake-request"
	"vehicle"
	"vehicle-and-wheel-individual"
	"vehicle-and-service-request"
	"wheel-individual"
]

type AllColumns set [
	"ServiceRequests"
	"VehiclePressure"
	"FrontAxlePressure"
	"RearAxlePressure"
	"FrontLeftWheelPressure"
	"FrontRightWheelPressure"
	"RearLeftWheelPressure"
	"RearRightWheelPressure"
	"CSD_Mode"
	"NPuMoMin"
	"NPuMoMax"
	"PSBC"
	"DriverRequest"
]

type ActuatorVehicleColumns set AllColumns.values[] [
	"VehiclePressure"
	"CSD_Mode"
	"NPuMoMin"
	"NPuMoMax"
	"PSBC"
	"DriverRequest"
]

type ActuatorAxlesColumns = set AllColumns.values[] [
	"FrontAxlePressure"
	"RearAxlePressure"
	"CSD_Mode"
	"NPuMoMin"
	"NPuMoMax"
	"PSBC"
	"DriverRequest"
]

// ----------------- validators/user.xen
import lists { VafActuatorInterface }

validator InterfaceToColumn({
	ActuatorInterface VafActuatorInterface
}) {
	match @ActuatorInterface {
		"axles" => @[]
	}
}

// ----------------- user.dto.xen
import validators/Interface { InterfaceToColumn }
import SQL { primary, foreign }

type ReviewStatus = {
	Draft "draft"
	InReview "in-review"
	Approved "approved"
	Invalid "invalid"
	Outdated "outdated"
}

type EventTuple = tuple [
	EventList
	string? /* Reason */ len(15..300)
	number // Price
]

type User = struct {
	_id string /^[a-f0-9]{24}$/ primary
	name string /^[A-Z]{3,5}_[0-9]{1,3}$/
	
	age Age
	utype UserType
	languageFilter bool (@age gt(12), false)
	events EventTuple[]
}

type Admin = struct User {
	username string @AdminName
	actions struct {
		// ...
	}
}

```

## Descriptors

### Types

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

### Complex types

- Struct: `{...}` or with [generics](#Generics)  `<Generics>` `{}`
- Enum object `enum {...}` or with types or literals
- Enum lists: `[a, b, c]` with literals or with types `(<Types>)[]` or `[string, i32]`

<!-- ### Generics -->

## Builtin validators:

### Validation operators

- `(...)` for grouping validators
- `not` inverts the result e.g. `not empty`
- `or` or `,`
- `xor` or `^`

### Single field

#### Common validators

- `<literal>` are they equal e.g. `string "exact"`
- `in(<enum, list>)` is in enumeration/list
- `empty` for strings and arrays
- Range: `a..b` or `a.<b` or `a<.b` or `a<.<b` where `a` and `b` are `<number> or <string>`

#### Number / Integer / BigInt / Float / Decimal validation

- `min(<number>)`
- `max(<number>)`
- `gt(<number>)`
- `lt(<number>)`
- `multipleof(<integer>)` you can test precision with this e.g. `0.031 multipleof(0.001)` is true.<br>*might break down if you write something too small*

#### String

- `/regex/flags`
- `minlen(<number>)`
- `maxlen(<number>)`
- `len(<number>)`
- `len(<range>)` is in range
