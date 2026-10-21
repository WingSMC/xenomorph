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

## Tuples and arrays

Simple types in fields, tuples, and type arguments cannot carry annotations.
Apply annotations to a top-level named type, then reference that type. See
[Annotation placement](GRAMMAR.md#annotation-placement) for the design rationale.

Tuples have fixed positions containing simple types or literals. Separate members
with commas and end every declaration with a semicolon. Arrays use an identifier
followed by `[]`.

```xen
type ActuatorInterface = ["axles", "vehicle", "wheel-individual"];
type Tuple = [
	ActuatorInterface,
	string,
	number,
	number,
];

// Declare an annotated type before using it in a tuple.
type EventLabel = string @minlen(15) @maxlen(300);
type EventTuple = [
	datetime,
	?EventLabel,
	number, // Price
];
type Events = EventTuple[] @minlen(1);
```

## Sets

A set declaration needs an element type in angle brackets, a literal prefill in
square brackets, or both. Prefill members require commas. A trailing comma is
allowed. Duplicate literal values produce semantic errors; they are not silently
removed.

```xen
type InterfaceName = string;
type Interfaces = set<InterfaceName>;
type KnownInterfaces = set ["axles", "service-request"];
type SmallIntegers = set<u8> [1, 2, 3,];
type EmptyInterfaces = set<InterfaceName> [];
```

A prefill accepts literals rather than type identifiers.

<!-- TODO: set operations -->

## Structs & Enums

Quote serialized keys that contain punctuation or overlap with Xenomorph
keywords. The Java target preserves the wire key with Gson metadata while
using a legal Java member name.

```xen
type Automation = { enabled: bool };
type ExecutionMode = {
	"ecu.test": ?Automation,
	"type": string,
};
```

```xen
type UserId = u64;
type UserName = string @match(/^[A-Z]{3,5}_[0-9]{1,3}$/) @minlen(5) @maxlen(20);
type UserAge = u8 @min(13) @max(127);
type User = {
	_id: UserId,
	name: UserName,
	age: UserAge,
	utype: UserType,
	languageFilter: bool,
	adultContent: bool,
};

// Action user contains algebraic data just like rust enums
type UserType = enum {
	Admin: -1,
	Basic: 1,
	Premium: 2,
};
type QueryLogRange = { from: datetime, to: datetime };
type Action = enum {
	DeleteUser: UserId,
	EditUser: UserId,
	QueryLogs: QueryLogRange,
};

type ActionEntry = [datetime, Action];
type ActionEntries = ActionEntry[] @minlen(1) @maxlen(10);
type AdminActions = { actions: ActionEntries };
type Admin = &User&AdminActions;

type TimelineTick = decimal @multipleof(0.1);
type Timeline = { position: TimelineTick };
```

## Deprecation

Mark a named declaration as deprecated metadata, with an optional reason:

```xen
type LegacyId = u32 @deprecated;
type LegacyTimeline = { elapsed: u32 } @deprecated("Use Timeline instead.");
type Tick = decimal @multipleof(0.1);
type Timeline = { position: Tick };
```
