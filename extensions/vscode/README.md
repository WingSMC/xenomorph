# Xenomorph for Visual Studio Code

Language support for Xenomorph (`.xen`) files, including syntax highlighting,
snippets, diagnostics, formatting, parsing tools, and AST visualization.

## Requirements

The extension runs the globally installed executables directly from `PATH`:

- `xenomorph_lsp` provides language-server features.
- `xeno` provides parse, lexer-debug, and AST-inspection features.

The extension does not package either executable. If you use different names
or absolute paths, configure `xenomorph.lsp.executable` and
`xenomorph.parser.executable` in VS Code settings.

After changing the LSP executable, reload the VS Code window.

## Commands

Open the Command Palette with `Ctrl+Shift+P` and search for **Xenomorph**:

- **Xenomorph: Parse Current Document** parses the active document and writes
  its AST and diagnostics to the **Xenomorph** Output channel.
- **Xenomorph: Debug Current Document (Tokens + AST)** writes the complete
  lexer token stream, AST, and parser diagnostics to the Output channel.
- **Xenomorph: Show AST Visualization** opens an interactive AST tab beside
  the editor.

The commands inspect the current in-memory editor text, so saving first is not
required.

## Declaration actions

Each parsed declaration has CodeLens actions above it:

- **Parse**
- **Debug tokens + AST**
- **View AST**

These actions inspect only that declaration. Disable them with
`xenomorph.codeLens.enabled` if you prefer a quieter editor.

## AST visualization

The AST tab renders the parser's structured output as a tree. You can:

- pan by dragging and zoom with the mouse wheel or toolbar buttons;
- fit, expand, or collapse the tree;
- double-click parent nodes to expand or collapse them;
- click ranged nodes to select and reveal their source in the editor;
- review parser diagnostics in the side panel.

The editor title bar also contains a tree icon that opens the AST for the
active Xenomorph document.

## Parser inspection protocol

The editor tools send source text to `xeno inspect` over standard input. The
command emits JSON containing `tokens`, `ast`, and `diagnostics`. Inspection is
syntax-only; workspace import resolution and semantic analysis continue to be
provided by the LSP.
