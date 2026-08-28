# Change Log

## [0.1.2]

- Add module graph preview and JSON commands backed by `xeno graph --json`.
- Use semantic tokens from the Rust lexer and parser for syntax highlighting;
  retain only comment fallback rules in the TextMate grammar.
- Resolve the `xenomorph_lsp` and `xeno` executables from `PATH` instead of
  packaging a platform-specific LSP binary.
- Add Command Palette actions for parsing, lexer/AST debugging, and interactive
  AST visualization.
- Add per-declaration Parse, Debug, and View AST CodeLens actions.
- Log parser results to the Xenomorph Output channel.

## [0.1.1]

- Renamed `.xenomorphrc` config file to `xenomorph.toml` and added schema support
    - The schema can be generated via the `xeno schema` command (only needs to be run when you add/remove a plugin)

## [0.1.0]

- Basic syntax highlighting
- LSP+Client
- Snippets
- File icons
