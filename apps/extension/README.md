# Flow-Like for VS Code

Language support for **FlowScript** — the textual representation of Flow-Like boards.

## Features

Supports two file types:

| File | Language | Description |
| --- | --- | --- |
| `*.flow` | FlowScript | A board: variables, functions and event handlers that call catalog nodes. |
| `*.flow.d` | FlowScript Declaration | Generated node declarations (one `declare function` per catalog node). |

### `.flow` files

- **Syntax highlighting** for keywords, interface structs, types, decorators, calls, strings and
  anchor comments.
- **Function result caching** support for bare `@cache`, empty `@cache({})`, and configured
  `@cache({ namespace: "pricing", ttlSeconds: 0, scope: "user" })` decorators, including
  completion, hover documentation and linting. Bare and empty forms use the `"global"` namespace,
  a 300-second lifetime, and app scope; explicit `ttlSeconds: 0` keeps entries until invalidated.
  Cache hits skip the entire function body, so only cache functions whose outputs are determined
  by their inputs.
- **Auto-completion** of every catalog node found in the workspace `.flow.d` files, plus locally
  declared interfaces, variables, functions and event handlers. Node completions insert a
  fully-named argument snippet (`floatAdd({ float1: \${1}, float2: \${2} })`).
- **Hover** documentation showing the signature, parameters, returns and purity of a node. Struct
  variables typed with a local `interface` show their fields, including array element fields.
- **Signature help** while typing arguments.
- **Outline / breadcrumbs** listing interfaces, variables, functions and event handlers.
- **Linting**: unbalanced brackets, unterminated strings, and unknown function calls (functions not
  declared in any `.flow.d` file and not declared locally).

### `.flow.d` files

- **Syntax highlighting** for `declare function`, JSDoc tags, types and optional parameters.
- **Outline** of all declared nodes.

## Configuration

| Setting | Default | Description |
| --- | --- | --- |
| `flowLike.lint.enable` | `true` | Enable linting of `.flow` files. |
| `flowLike.lint.unknownFunctions` | `true` | Warn on calls to functions not declared anywhere. |

## Commands

- **FlowScript: Reload Node Declarations** — re-scan the workspace for `.flow.d` files.

The extension automatically discovers and watches every `*.flow.d` file in the workspace, so the
catalog of available nodes stays current as declarations change.
