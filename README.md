# Xenomorph (eXtensible ENtity & Object MOdel Relation PHrocessor)

## What is this?

Xenomorph is meant to be a universal schema descriptor. It is a front-end for plugins that can be used to generate validators, serializers, ORMs, and other data-shape and relation related code in a language and framework agnostic way.

## Language Documentation

- [Examples](docs/EXAMPLES.md)
- [Grammar](docs/GRAMMAR.md)

## Config (`.xenomorphrc`)

## Parser

## LSP

## Development

- Install [Rust](https://rust-lang.org/learn/get-started/) (Recommended 1.94)
- Install [Node.js](https://nodejs.org/en/download) (Recommended 24.11)

- Run `npm run install:once` or install these manually:
    - Install [pnpm](https://pnpm.io/) because it's nicer than npm
    - Install [@antfu/ni](https://github.com/antfu-collective/ni) for npm run scripts to work

- Run `nr install:deps` to install some dependencies
- Run `nr build` to run compile/build all sub-projects and extensions.
