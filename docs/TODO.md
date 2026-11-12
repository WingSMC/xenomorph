- This is just a concern: Make sure not to recompute the global types/traits hierarchy to keep the code performant, and keep a per-module local hierarchy that joins the global one. And importing modules can rely on other modules' local hierarchy. Also place declaration local hierarchies below this one (generics). This also makes validation easier.

- [~] Add gh actions on main tagging (manually enter version and replace all package.json & Cargo.toml versions with that, generate/build exe/dll/so/dylib/other executables and vsce package, publish the vscode extension to marketplace and individual binaries to npm with the given version) e.g. @xenomorph/cli @xenomorph/lsp @xenomorph/typescript @xenomorph/json-schema @xenomorph/java-dto

- Utility to find duplicate types (ones which semantically map to the same thing after complete resolution (recursion!!!))

- Decorator "result/return" types

- Benchmarks and optimizations
- !T for Required<T>
- Deep literals (for structs & arrays)
