# Java plugin

The Java plugin generates one Lombok DTO per Xenomorph struct or tuple and one
Java enum per Xenomorph enum, literal set, or literal sum. Sums of struct
declarations generate marker interfaces. Homogeneous literal tuples and boolean
literal sets produce array constants in `Xenomorph.java` instead of tuple DTOs.

## Configure the generator

Enable the plugin and configure its output package in `xenomorph.toml`:

```toml
[plugins]
plugins = ["xenomorph_java"]

[plugins.java]
output = "./generated/java/"
package = "com.example.model"
data = true
builder = true
```

The available Java plugin settings are:

| Setting   | Type    | Purpose                                                                                                                                                                                                      |
| --------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `output`  | string  | Target directory for generated `.java` files. Relative paths start at the workspace root. Package subdirectories are created below this directory. If omitted, output is written next to each source module. |
| `package` | string  | Package declaration used by generated Java files, such as `com.example.model`.                                                                                                                               |
| `value`   | boolean | Add Lombok `@Value` to every generated class.                                                                                                                                                                |
| `builder` | boolean | Add Lombok `@Builder` to every generated class.                                                                                                                                                              |
| `data`    | boolean | Add Lombok `@Data` to every generated class.                                                                                                                                                                 |

Generated DTOs require Lombok. Generated tuple support also requires Gson.

## Use tuples with Gson

For a tuple declaration such as:

```xen
type SomethingTuple = [string, i32];
```

the plugin generates a positional DTO marked with `@JsonTuple`. It also emits these support files in the configured output package:

- `JsonTuple.java` — marks generated tuple DTOs.
- `TupleTypeAdapterFactory.java` — converts tuple DTOs to and from JSON arrays.

Register the generated factory on **every Gson instance** that reads or writes generated tuples:

```java
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.example.model.TupleTypeAdapterFactory;

Gson gson = new GsonBuilder()
        .registerTypeAdapterFactory(new TupleTypeAdapterFactory())
        .create();
```

The factory is not registered automatically. Without this registration, Gson treats a tuple as a regular object instead of its positional JSON-array representation.

After registration, tuple JSON is positional:

```java
SomethingTuple tuple = gson.fromJson(
        "[\"asd\", 10]",
        SomethingTuple.class
);

String json = gson.toJson(tuple); // ["asd",10]
```

Nested generated tuples use the same factory and require no additional adapters. The reader rejects arrays with missing or additional elements.

## Generated tuple shape

Tuple positions use zero-based Java fields named `item0`, `item1`, and so on. For example, the declaration above produces a class shaped like this:

```java
@JsonTuple
public class SomethingTuple {
    private String item0;
    private Integer item1;
}
```

The exact Lombok annotations and accessors depend on the plugin configuration and any Xenomorph `@java::Lombok(java::Value, ...)` annotations.

## Struct utilities and intersections

Java has no structural types, so struct-producing utilities (`Pick`, `Omit`,
`Partial`, `Complete`) and intersections of struct-shaped members (`&A&B`) each
generate their own flattened class. An intersection's class carries every
member's fields in member order. A field present in several members is
required unless every member marks it optional. The class does not extend its
members, so a `B` value is not assignable to an `A` reference in Java.

## Literal sets and literal sums

A prefilled set (`type Kind = set ["a", "b"];`, `type Small = set<u8> [1, 2];`)
or a sum of literals of one kind (`type Mode = |"on"|"off";`) is a value type.
It generates a Java enum, and fields of that type use the enum (`Kind`,
`List<Kind>`):

```java
public enum Kind {
    @SerializedName("a")
    A("a"),
    @SerializedName("Decision Table Testing")
    DECISION_TABLE_TESTING("Decision Table Testing");

    private final String value;
    // getValue(), toString(), fromValue(String), contains(String)
}
```

- Constant names are UPPER_SNAKE_CASE. Numbers become `VALUE_1`,
  `VALUE_MINUS_1`, or `VALUE_0_5`. When two literals produce the same name,
  the later constant gets a `_2`, `_3` suffix and the plugin reports a warning.
- `fromValue` returns the matching constant or throws
  `IllegalArgumentException`. `contains` checks a raw value, for example for
  validation or UI selections.
- Numeric enums store `int`, `long`, `float`, `double`, `BigInteger`, or
  `BigDecimal`. Gson always writes enums as strings, so numeric enums carry a
  nested `@JsonAdapter` `GsonAdapter` that reads and writes JSON numbers.
- Boolean literal sets keep an array constant in `Xenomorph.java`, and fields
  use `Boolean`. Literal sums of mixed kinds map to `Object`.
- A sum whose members are literals or other literal sets and literal sums
  (`type Stored = |Technique|Placeholder;`) generates one enum that holds
  all their values, without duplicates. Java enums cannot extend each other,
  so `Stored` and `Technique` are unrelated Java types.
- Sets without a prefill, such as `set<string>`, stay `Set<String>`.

## Sums of structs

A named sum whose members are all class-backed declarations (structs,
struct-shaped intersections, or struct utilities) generates a marker interface:

```xen
type Msg = |SaveMsg|ProblemMsg;
```

```java
public interface Msg {}
public class SaveMsg implements Msg { ... }
```

Fields of type `Msg` use the interface. A nested sum of such sums extends the
inner interfaces. A class implements every sum that lists it, including sums in
other modules, as long as those modules are reachable from the parser entry.
Gson cannot deserialize an interface by itself; a discriminator-aware
`TypeAdapter` is still to be defined (TBD). Sums with non-class members, such as
`|string|i32`, generate no interface and fields use `Object`.

## Java keyword names

Struct fields whose names are Java keywords, including quoted keys such as
`"class"`, get a trailing `_` (`protected_`) and
`@SerializedName("protected")`. A field that is already named `protected_`
collides with the escaped name and is reported as an error. Declaration
(type) names that are Java keywords are still errors.

## Deprecation metadata

`@deprecated` or `@deprecated("Use NewType instead.")` on a named declaration
emits Java `@Deprecated` and a Javadoc `@deprecated` tag on the generated class,
enum, or constant. The optional reason appears in the tag. Scalar aliases are
expanded into their underlying types and have no separate declaration to mark.

## Plugin lifecycle

The Java generator uses the [versioned plugin SDK](../../docs/PLUGINS.md).
Output paths use the configuration supplied by the host at initialization.
The LSP retains Java/Lombok semantic metadata but releases the generator
library; it does not generate Java files.
