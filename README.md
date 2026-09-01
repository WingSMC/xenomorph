# Xenomorph (eXtensible ENtity & Object MOdel Relation PHrocessor)

## What is this?

Xenomorph is meant to be a universal schema descriptor. It is a front-end for plugins that can be used to generate validators, serializers, ORMs, and other data-shape and relation related code in a language and framework agnostic way.

## Language Documentation

- [Examples](docs/EXAMPLES.md)
- [Grammar](docs/GRAMMAR.md)

## Plugin Documentation

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

Tables are merged recursively from the extended config into the consuming
config. The consuming config replaces colliding scalar and array values while
retaining non-colliding keys and subkeys. Paths used at runtime remain relative
to the authoritative consuming config's directory. `xeno schema` writes
`.xenomorph/xenomorph.schema.json` beside the deepest extended config (or beside
the authoritative config when it does not extend another file).

Unknown annotations in the selected plugin visibility scope are warnings, so a
TypeScript-only view can tolerate Java-only annotations such as `@Lombok`.
Unknown types remain errors.

Use `xeno config --inspect` to print the deeply merged TOML configuration to
standard output. The output omits the consumed `abstract` and `extends` keys.

## Parser

`xeno` parses the configured workspace. For editor and tooling integrations,
`xeno inspect` reads standalone Xenomorph source from standard input and emits
structured JSON containing the lexer token stream, syntax AST, and parser
diagnostics. Standalone inspection does not resolve imports or run workspace
semantic analysis.

`xeno graph` prints the configured workspace's module graph in a readable
`importer -> imported` format. Use `xeno graph --json` for the versioned JSON
representation consumed by editor and other tooling integrations.

## LSP

`xenomorph_lsp` communicates over standard input/output and can be launched by
the VS Code extension when the executable is available on `PATH`. It provides
Rust-derived semantic tokens for context-aware syntax highlighting, alongside
diagnostics, completion, hover, formatting, navigation, and rename support.

## Development

- Install [Rust](https://rust-lang.org/learn/get-started/) (Recommended 1.94)
- Install [Node.js](https://nodejs.org/en/download) (Recommended 24.11)

- Run `npm run install:once` or install these manually:
    - Install [pnpm](https://pnpm.io/) because it's nicer than npm
    - Install [@antfu/ni](https://github.com/antfu-collective/ni) for npm run scripts to work

- Run `nr install:deps` to install some dependencies
- Run `nr build` to run compile/build all sub-projects and extensions.
