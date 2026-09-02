- This is just a concern: Make sure not to recompute the global types/traits hierarchy to keep the code performant, and keep a per-module local hierarchy that joins the global one. And importing modules can rely on other modules' local hierarchy. Also place declaration local hierarchies below this one (generics). This also makes validation easier.

- Add gh actions on main tagging (manually enter version and replace all package.json & Cargo.toml versions with that, generate/build exe/dll/so/dylib/other executables and vsce package, publish the vscode extension to marketplace and individual binaries to npm with the given version) e.g. @xenomorph/cli @xenomorph/lsp @xenomorph/typescript @xenomorph/json-schema @xenomorph/java-dto

- Lombok "types" to annotations
    - [ ] Add a bool field to "hide" some types from being recommended (e.g. lombok decorator typenames), only show them if there is some constraint (not Any type) it matches, e.g. the LombokDecorator trait.
    - [ ] Only warn if there is a missing type IN a missing annotation's arguments.
    - [ ] Rework lombok "type" identifiers to annotations.

- Prevent &ing very disjoint types (e.g. u8 and string)

- Utility to find duplicate types (ones which semantically map to the same thing after complete resolution (recursion!!!))

- Decorator "result/return" types

- Benchmarks and optimizations
- !T for Required<T>
- Deep literals (for structs & arrays)
