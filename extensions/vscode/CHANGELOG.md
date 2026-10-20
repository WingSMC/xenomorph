# Change Log

## [Unreleased]

- Rewrite the AST and module graph webviews as Svelte components that use D3.
- Lay out the module graph in layers: modules that import nothing are in the
  rightmost column, and edge crossings are reduced. Import cycles are dashed.
- Highlight the imports and importers of the hovered module, and list them
  for the selected module.
- Show the AST left to right, and collapse declarations in large documents.

## [0.1.2]

- Add a manual LSP restart command and automatically restart after the active
  workspace `xenomorph.toml` changes.
- Add module graph preview and JSON commands backed by `xeno graph --json`.
- Use semantic tokens from the Rust lexer and parser for syntax highlighting;
  retain TextMate fallback rules for hover and completion documentation.
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
