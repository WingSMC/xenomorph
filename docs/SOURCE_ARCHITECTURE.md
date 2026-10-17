# Source indexing and coordinated updates

Each registry module owns one current `SourceDocument`, its tokens, its AST,
declaration indexes, and diagnostics under one `RwLock`. Tokens and AST nodes
store `TextRange` values: half-open UTF-8 byte offsets, with no source pointers
or lifetimes. Resolve those ranges against the source while holding the module
guard. A copied range has no revision identifier; it must not be resolved after
the guard is released and the document changes. Registry summaries contain
owned names and semantic metadata, never borrowed source text.

The source stays contiguous, with a line-start index. The lexer counts bytes
only. Position conversion uses that index, and editor output uses UTF-16
columns. Strings, regexes, documentation, and comments can span lines, so a
line is not a safe independent lexing unit.

An update holds the registry coordinator for the entire synchronous pass. A
single module writer replaces the source, tokens, AST, and declaration indexes
and publishes canonical semantic metadata. It releases that writer before
loading another module. No source history or document revision is retained.
After ingestion, each affected module is locked once for collision checks,
semantic scanning, and diagnostic publication. There is no background queue.

Incremental lexing finds the changed byte interval, restarts at a neutral
checkpoint before it, and stops when unchanged text converges at an old
checkpoint. Whole multiline lexemes and import/path pairs lie between
checkpoints. Error recovery falls back to a complete scan when necessary.

Incremental parsing verifies unchanged top-level declarations by token kinds,
relative ranges, and exact source text, then reuses and relocates their owned
ASTs. It reparses changed declarations. A previous parse with diagnostics uses
a complete parse to preserve recovery behavior. Unchanged declarations retain
their semantic metadata and canonical hierarchy definitions through `Arc`
sharing. Scope construction shares the hierarchy instead of rebuilding it.

The change pass marks edited modules, modules referring to changed declaration
names (including unresolved names), and their transitive importers. It scans
each marked module once in deterministic dependency order. Name-based
invalidation is conservative: it can rescan an unrelated module with the same
identifier. Kahn's algorithm defers import cycles and modules depending on
them, with diagnostics until the cycle is removed. Recursive type references
are a separate graph and remain legal, including:

```xen
type TreeNode = { children: Tree[] };
type Tree =
    | TreeNode
    | number;
```

The implementation reduces lexer, parser, and semantic conversion work; it
does not guarantee sublinear edit cost. Source comparison, line-index building,
and relocating the flat token/AST arrays still take linear work. Structural
edits and ambiguous recovery can require parsing the rest of the file.

The host and plugin SDK share owned syntax and exchange Postcard buffers.
Only the C function table, opaque handles, and byte buffers cross the native
ABI. Fieldless enums can use explicit integer representations; payload enums
remain serialized Rust values.
