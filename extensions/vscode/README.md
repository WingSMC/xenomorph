# Xenomorph for Visual Studio Code

Language support for Xenomorph (`.xen`) files, including syntax highlighting,
snippets, diagnostics, formatting, parsing tools, and AST visualization.

Syntax highlighting in `.xen` editors is supplied by `xenomorph_lsp` semantic
tokens generated from the Rust lexer and parser. The bundled TextMate grammar
provides fallback highlighting while the server starts and colors Xenomorph
code blocks embedded in hover and completion documentation, where semantic
tokens are not available.

## What is Xenomorph?

It is a schema description language written for polyglot projects, like a microservice architecture. It lets you generate language and library specific codes for DTOs, serializers, database schemas or form validators from a **_single source of TRUTH_**.

Find out more at [https://github.com/WingSMC/xenomorph](https://github.com/WingSMC/xenomorph).

The repo is also the place where you can download the required binaries for your operating system. Make sure to place them on the `$PATH`/`$env:PATH`/`%PATH%`.

## Requirements

The extension runs the globally installed executables directly from `PATH`:

- `xenomorph_lsp` provides language-server features.
- `xeno` provides parse, lexer-debug, AST-inspection, and module-graph features.

The extension does not package either executable. If you use different names
or absolute paths, configure `xenomorph.lsp.executable` and
`xenomorph.parser.executable` in VS Code settings.

After changing the LSP executable setting, reload the VS Code window. Use
**Xenomorph: Restart LSP Server** to restart the current executable manually.
Changes to the workspace's discovered `xenomorph.toml` restart the language
server automatically.

## Commands

Open the Command Palette with `Ctrl+Shift+P` and search for **Xenomorph**:

- **Xenomorph: Parse Current Document** parses the active document and writes
  its AST and diagnostics to the **Xenomorph** Output channel.
- **Xenomorph: Debug Current Document (Tokens + AST)** writes the complete
  lexer token stream, AST, and parser diagnostics to the Output channel.
- **Xenomorph: Show AST Visualization** opens an interactive AST tab beside
  the editor.
- **Xenomorph: Show Module Graph** opens the configured workspace's interactive
  module dependency graph.
- **Xenomorph: Show Module Graph JSON** opens the CLI graph protocol as a JSON
  document.
- **Xenomorph: Restart LSP Server** restarts the language-server process and
  reloads workspace configuration, plugins, and modules.

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
active Xenomorph document and a references icon that opens the module graph.

## Module graph visualization

The module graph runs `xeno graph --json` in the active workspace. Arrows point
from an importer to the module it imports. Entry, error, and warning nodes are
highlighted; click any module to open its source file. The graph supports pan,
zoom, and fit controls.

## Parser inspection protocol

The editor tools send source text to `xeno inspect` over standard input. The
command emits JSON containing `tokens`, `ast`, and `diagnostics`. Inspection is
syntax-only; workspace import resolution and semantic analysis continue to be
provided by the LSP.
