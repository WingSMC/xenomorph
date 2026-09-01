# Java plugin

The Java plugin generates one Lombok DTO per Xenomorph struct or tuple and one Java enum per Xenomorph enum.

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

The exact Lombok annotations and accessors depend on the plugin configuration and any Xenomorph `@Lombok(...)` annotations.
