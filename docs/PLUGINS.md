# Plugin API reference

This reference is for Rust plugin authors migrating to the versioned Xenomorph
plugin interface.

## Compatibility

Plugins are `cdylib` libraries. Their exported entry point is
`xenomorph_plugin_v1`, using the C calling convention. The host checks the ABI
magic, major version, function-table size, and payload protocol version before
using callbacks. ABI major 1 and payload protocol 1 are required.

`rust-toolchain.toml` selects a pinned nightly compiler.

The plugin and the host must use the same operating system and architecture,
and a compatible ABI and payload protocol.
Native Rust types never cross this interface, even when both binaries use the same compiler.

## Rust SDK

Depend on `xenomorph_common` and define an internal `XenoPlugin` descriptor.
Export it with `xenomorph_common::export_plugin!(PLUGIN)`.
The plugins in the repo demonstrate this pattern.

The descriptor is an SDK-local Rust value, not the exported ABI. Its factories
receive `&Config` and return local `AnalyzerListener` instances. The SDK sends
only C-compatible function tables, opaque handles, and byte buffers to the host.

| Callback                | Purpose                                                                                                  |
| ----------------------- | -------------------------------------------------------------------------------------------------------- |
| `initialize`            | Receive the host's resolved configuration before runtime calls. Return an error if initialization fails. |
| `shutdown`              | Release initialized plugin resources before the instance and library are destroyed.                      |
| `provide_types`         | Describe semantic types and their dependencies.                                                          |
| `provide_annotations`   | Describe annotations, parameters, and constraints.                                                       |
| `provide_config_schema` | Describe the plugin's configuration section as JSON Schema.                                              |
| `register_generator`    | Create a fresh local generator listener for a module.                                                    |
| `register_analyzer`     | Create a fresh local analysis listener for a module.                                                     |

Keep metadata providers side-effect-free. Metadata and schema discovery does not
initialize generator state.

## Configuration

The host resolves `xenomorph.toml`, its inheritance chain, and the authoritative
workspace directory. It passes an immutable snapshot to initialization.
Factories use that snapshot for the lifetime of their plugin instance.

Use `config.workdir` to resolve output directories. Use
`config.plugins.config` to read plugin options. Do not call `Config::get()`,
rediscover configuration, or derive workspace paths from the plugin's current
directory. A plugin can receive a workspace different from the process's
current directory.

Reload initializes a new instance with a new snapshot. It does not mutate
configuration underneath existing calls.

## Wire representation

`xenomorph_plugin_api` defines the C-compatible ABI declarations and syntax
payload. The function table provides describe, initialize, run, destroy, and
buffer-release operations.

Payloads use Postcard binary serialization. The ABI header uses protocol 1.
Module requests contain the current source, the shared owned AST, and the host's semantic scope.
AST tokens contain half-open UTF-8 byte ranges into that source. The host
serializes borrowed syntax directly, without constructing another AST.
Integers use a sign and little-endian magnitude; decimals use an integer
coefficient and signed scale. Configuration uses typed binary values. JSON
Schema remains JSON text inside the description payload.
Native paths use Windows UTF-16 units or Unix bytes. They do not need a lossy UTF-8 conversion.

The SDK deserializes the shared AST and runs local listener callbacks
in their normal order. It does not reparse source or rerun workspace resolution.
Protocol 1 does not support custom opaque Rust declaration values.

Diagnostic severity values are `0` for errors, `1` for warnings, and `2` for
information. Diagnostic byte ranges must fit within a token in the submitted AST. The host
rejects malformed payloads instead of accepting incomplete results.

## Ownership and threading

Input spans are borrowed only for the duration of a callback. Returned buffers
belong to the plugin. The host copies them and invokes the originating plugin's
release callback before unloading. Neither side frees allocations using the
other side's allocator.

The SDK creates and destroys opaque handles. The SDK serializes calls on one
instance. An instance may move between host threads. Do not require
thread-local initialization state. Listener instances remain local to a
module call. The SDK destroys them before the call returns.

Do not start detached threads, retain host input pointers, or register
untracked callbacks. Protocol 1 does not support background work. Native
plugins are trusted code, not isolated processes.

The SDK catches unwind-mode panics inside exported operations and returns an
explicit error. The failed instance cannot be called again until reload. The
host cannot recover from abort-mode panics or memory corruption in process.

## Loading and unloading policies

| Host operation       | Executable capabilities retained |
| -------------------- | -------------------------------- |
| Generation           | Generation and analysis          |
| LSP / analysis       | Analysis only                    |
| Configuration schema | None                             |

The host copies semantic metadata and configuration schemas before releasing
library handles. This preserves TypeScript's `symbol` and Java's Lombok
definitions in the LSP without retaining their generator code.

The host loads libraries from unique temporary shadow copies. This allows
rebuilt plugin artifacts to replace the original files while the host is
running, including on Windows. Plugins must not depend on being loaded from
their original build directory.

Old runtime snapshots keep library owners alive until their last in-flight
request finishes. Teardown destroys plugin state and releases buffers before
closing the native handle. The host reports close errors. Some platforms can
keep a library mapped after the host releases the native handle. Physical
unmapping is not a portable guarantee.
If plugin shutdown fails, the host reports the failure. It pins the library
and its shadow copy instead of unloading code that may still run. Native
close failures also retain the shadow copy for diagnosis.

## LSP reload

Configuration changes reconcile the workspace in process. **Xenomorph: Reload
Plugins** requests `workspace/executeCommand` with command
`xenomorph.reloadPlugins`. It also rereads unchanged plugin filenames.

Reload constructs a candidate configuration, plugins, metadata, module state,
and config watcher before publishing them. Invalid configuration or plugin
initialization leaves the last valid runtime active. The host retains open
unsaved document text. The LSP never invokes generators.

There is no rebuilt-library watcher. Use the reload command after rebuilding
plugins. **Xenomorph: Restart LSP Server** remains available for full process
recovery.

## Developer validation

Build dynamic libraries before running the explicitly ignored DLL integration
tests. On Windows, from the repository root:

```powershell
cargo build --workspace
cargo +stable build -p xenomorph_test_plugin --features alternate-version --target-dir target\plugin-fixture-stable
cargo build -p xenomorph_test_plugin --features bad-abi --target-dir target\plugin-fixture-bad
cargo test --workspace
cargo test -p xenomorph_common plugins::tests -- --ignored
cargo test -p xenomorph_lsp --test reload -- --ignored
cargo test -p xenomorph_parser --test plugin_cli -- --ignored
```

The integration tests cover:

- Authoritative config paths
- Metadata retention
- In-flight ownership
- Initialization failure
- Panic handling
- Independently compiled stable/nightly compatibility
- Incompatible DLL rejection

The `plugin-abi` workflow runs these checks on Windows, Linux, and macOS.
