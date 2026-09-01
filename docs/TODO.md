- [ ] This is just a concern: Make sure not to recompute the global types/traits hierarchy to keep the code performant, and keep a per-module local hierarchy that joins the global one. And importing modules can rely on other modules' local hierarchy. Also place declaration local hierarchies below this one (generics). This also makes validation easier. Although I think there is a notification feature missing from the LSP that tells importing modules that the current module changed and they need to recompute their own type constraints (and other validation). (if you implement this make sure not to run into recursive imports, modules can be exported by modules which are imported by a module)

- Make sure sets actually mean a list of unique constants or types in the language so the same string literal twice in a set should error. Although it should be possible to make a set type via `set<T>` (no [...] here) without the prefill content, but `set (<T>)? [...SimpleTypeExpr]` should just yield a list/array of unique items where each item should be bound by the T constraint if `<T>` is defined, e.g. `set<string>["a", "b"]` -> fine `set<string>["a", "a", 123]` -> fails because duplicate "a" and 123 fails string/string literal constraint.

- check if &A&B implies constraints are inherited from both (union) while |A|B implies only common types/traits are applicable at all times (intersecion).

- Add gh actions on main tagging (manually enter version and replace all package.json & Cargo.toml versions with that, generate/build exe/dll/so/dylib/other executables and vsce package, publish the vscode extension to marketplace and individual binaries to npm with the given version) e.g. @xenomorph/cli @xenomorph/lsp @xenomorph/typescript @xenomorph/json-schema @xenomorph/java-dto

- Lombok "types" to annotations
    - [ ] Add a bool field to "hide" some types from being recommended (e.g. lombok decorator typenames), only show them if there is some constraint (not Any type) it matches, e.g. the LombokDecorator trait.
    - [ ] Only warn if there is a missing type IN a missing annotation's arguments.
    - [ ] Rework lombok "type" identifiers to annotations.

- Prevent &ing very disjoint types (e.g. u8 and string)

- Utility to find duplicate types (ones which semantically map to the same thing after complete resolution (recursion!!!))
