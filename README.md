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
