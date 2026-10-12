# Xenomorph (eXtensible ENtity & Object MOdel Relation PHrocessor)

## What is this?

Xenomorph aims to be a universal schema descriptor. It is a front end for
plugins. Plugins generate validators, serializers, ORMs, and other code for
data shapes and relations, independent of language or framework.

## Language Documentation

- [Examples](docs/EXAMPLES.md)
- [Grammar](docs/GRAMMAR.md)

## Plugin Documentation

- [Plugin API, ownership, and reload](docs/PLUGINS.md)
- [Java generator](plugins/java/README.md)

## Config (`xenomorph.toml`)

Xenomorph discovers `xenomorph.toml` from the current directory upward. A
reusable schema project can mark its local config as abstract:

```toml
# /shared-schemas/xenomorph.toml
#:schema ./.xenomorph/xenomorph.schema.json

abstract = true

[formatter]
indent_kind = "space"
indent_width = 4
line_ending = "lf"

[debug]
loglevel = "info"
# tokens = true
# ast = true
# plugins = true
```

`abstract = true` makes discovery continue upward.

```toml
# consuming-repository/xenomorph.toml from ui project
#:schema ./shared-schemas/.xenomorph/xenomorph.schema.json

extends = "./reusable-project/xenomorph.toml"

[parser]
entry = "reusable-project/ui"

[plugins]
path = "./node_modules"
plugins = ["xenomorph_typescript"]
```

Tables merge recursively from the extended config into the consuming config.
The consuming config replaces colliding scalar and array values. It keeps
non-colliding keys and subkeys. Paths used at runtime remain relative to the
authoritative consuming config's directory. `xeno schema` writes
`.xenomorph/xenomorph.schema.json` beside the deepest extended config. If the
authoritative config does not extend another file, `xeno schema` writes the
file beside that config instead.

The host reports unknown annotations in the selected plugin visibility scope
as warnings. A TypeScript-only view can therefore tolerate Java-only
annotations such as `@Lombok`. Unknown types remain errors.

Use `xeno config --inspect` to print the deeply merged TOML configuration to
standard output. The output omits the consumed `abstract` and `extends` keys.

## Parser

`xeno` parses the configured workspace. For editor and tooling integrations,
`xeno inspect` reads standalone Xenomorph source from standard input. It emits
structured JSON containing the lexer token stream, syntax AST, and parser
diagnostics. Standalone inspection does not resolve imports or run workspace
semantic analysis.

`xeno graph` prints the configured workspace's module graph in a readable
`importer -> imported` format. Use `xeno graph --json` for the versioned JSON
representation consumed by editor and other tooling integrations.

## LSP

The VS Code extension can launch `xenomorph_lsp` when the executable is
available on `PATH`. `xenomorph_lsp` communicates over standard input/output.
It provides:

- Rust-derived semantic tokens for context-aware syntax highlighting
- Diagnostics
- Completion
- Hover
- Formatting
- Navigation
- Rename support

Configuration changes reload the workspace and plugins in process.
The VS Code **Reload Plugins** command reloads rebuilt libraries without restarting the LSP.
The host releases generator-only libraries after it copies their semantic metadata.

## Development

- Install [Rust through rustup](https://rust-lang.org/tools/install/). The workspace selects the pinned nightly toolchain in `rust-toolchain.toml`, including rustfmt and Clippy.
Nightly does not yet provide a stable native Rust ABI. Plugins must use the C ABI for compatibility.
- Install [Node.js](https://nodejs.org/en/download) (Recommended 24.11)

- Run `npm run install:once` or install these manually:
    - Install [pnpm](https://pnpm.io/). It is nicer than npm
    - Install [@antfu/ni](https://github.com/antfu-collective/ni) to make `npm run` scripts work

- Run `nr install:deps` to install some dependencies
- Run `nr build` to compile and build all sub-projects and extensions.

## Versions and CI

Run `pnpm run update-versions 1.2.3` from the workspace to set all `package.json`
and Cargo package versions, including the test fixture, and synchronize local
package entries in Cargo lockfiles. Dependency versions are preserved. The
script requires Node.js and accepts a SemVer version without a `v` prefix.
Use `pnpm run check-versions 1.2.3` to validate versions without changing files.

Testing and linting run on pull requests (opened, updated, reopened, or marked
ready for review), branch pushes, and manual workflow dispatches. Testing
includes the Rust workspace and plugin ABI/lifecycle tests on Windows, Linux,
and macOS, plus the workspace script and VS Code extension tests. Linting checks
Rust formatting, Clippy warnings, and extension TypeScript types.

To build release artifacts, commit the updated manifests and lockfile, then
push an exact version tag such as `1.2.3`. The build workflow rejects tags that
do not match every package version. Successful builds upload these artifacts
to the workflow run:

- A `.tar.gz` for each of Windows x64, Linux x64, and macOS ARM64 containing
  `xeno`, `xenomorph_lsp`, and the Java, JSON Schema, and TypeScript plugin
  libraries (`.dll`, `.so`, or `.dylib`). Archives preserve executable permissions.
- The platform-independent `xenomorph-<version>.vsix` VS Code extension.
