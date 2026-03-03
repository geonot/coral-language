# Coral Language — VS Code Extension

Syntax highlighting and language support for the [Coral programming language](https://github.com/user/coral).

## Features

- **Syntax highlighting** for all Coral constructs — functions, types, enums, stores, actors, traits, errors, taxonomies, template strings with interpolation, and more
- **Bracket matching** and auto-closing pairs
- **Indentation rules** — auto-indent after function defs, if/else, while, for, match, type/enum/store/actor/trait/err declarations
- **Code folding** via indentation (off-side rule)
- **Comment toggling** with `#`

## Supported Syntax

| Feature                  | Example                              |
|--------------------------|--------------------------------------|
| Function definitions     | `*greet(name)`                       |
| Bindings                 | `x is 42`                            |
| Template strings         | `'Hello, {name}!'`                   |
| Match expressions        | `match x` with `Pattern ? body`      |
| Type/Enum/Store/Actor    | `type Point`, `enum Shape`           |
| Error definitions        | `err NotFound`                       |
| Taxonomy                 | `!!Category:Subcategory`             |
| Pipeline operator        | `data ~ transform ~ output`          |
| Ternary                  | `cond ? yes ! no`                    |
| Placeholders             | `$`, `$1`                            |

## Installation

### From source (development)

1. Clone or copy the `vscode-coral` directory
2. Open VS Code and press `F5` to launch the Extension Development Host
3. Open any `.coral` file to see syntax highlighting

### Package as VSIX

```bash
npm install -g @vscode/vsce
cd vscode-coral
vsce package
code --install-extension coral-lang-0.1.0.vsix
```

## Tree-sitter Grammar

A complete tree-sitter grammar for Coral is available in the sibling `tree-sitter-coral/` directory. This can be used with tree-sitter compatible editors (Neovim, Helix, Zed, etc.) for more advanced syntax features including:

- Incremental parsing
- Structural code navigation
- Syntax-aware highlighting via `queries/highlights.scm`
