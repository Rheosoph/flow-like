---
title: FlowScript
description: The text projection of a board — syntax, calls, namespaces, and the declaration files behind it
sidebar:
  order: 18
---

**FlowScript** is a TypeScript-flavoured rendering of a board. Every board can be shown as
FlowScript, edited as text, and applied back onto the graph; it is also the language FlowPilot
reads and writes when it changes a workflow. This page is the language reference. The examples are
copied from real rendered boards, so what you see here is exactly what the editor renders.

## File layout

A FlowScript document has five parts, always in this order:

1. `use` lines that open namespaces (see [`use`](#use)).
2. `interface` declarations — the schemas of struct-typed values.
3. Top-level variables (`const` / `let`, optionally decorated with `@category("…")` or
   `@secret`).
4. `function` declarations — each becomes a Function layer whose parameters are input pins and
   whose returns are output pins.
5. Events — the entry nodes, written as `<eventType> <name>(params) { … }`.

```ts
use db::*
use string::*
use ui::*

interface ReportEntry {
    title: string;
    uri: string;
    summary: string;
    date: string;
    interest?: string;
    report_id: string;
}

@category("Report")
const reportCreated = false
@category("Report")
const reportEntry: ReportEntry[] = []
@category("Report")
const reportID = ""

function saveConfig(config: Struct) {
    files::userDir({ nodeScope: false }).child("config.json").writeString({ content: json::stringify({ value: config, pretty: true }) })
}

eventsSimple clickLatestOverview() {
    const { database } = open({ name: "report_overview", userScoped: true, batchSize: 1000 })
    const { session } = df::createSession({ sessionName: "default" })
    session.registerLance({ database: database, tableName: "reports" })
    const { rows } = session.sqlQuery({ query: "SELECT report_id FROM reports ORDER BY created DESC LIMIT 1;" })
    navigateTo({ route: `/briefing?report_id=${rows[0].report_id}` })
}
```

A top-level `const` whose initializer is a scalar literal infers its type (`string`, `int`,
`float`, `bool`); everything else keeps an explicit `: Type`. A secret variable renders its
`@secret` decorator but never its value.

## Calling nodes

Every catalog node has a **namespace** and an **alias**. A call is the qualified path followed by
an object whose keys are the node's exact input pin names:

```ts
const { hash } = hash::md5({ input: item.content })
log::info({ message: source, toast: false })
const { markdown } = md::fromHtml({ html: text, skippedTags: ["script", "style"] })
```

`::` separates namespace segments; `.` is field or method access on a *value*. The same node can
be spelled three ways, and all three resolve to the same node:

| Spelling | Example | When |
| --- | --- | --- |
| qualified | `string::trim({ string: s })` | always works; the canonical static form |
| method | `s.trim()` | nodes whose declaration has a `this:` parameter |
| legacy flat | `stringTrim({ string: s })` | the camelCase node type; accepted forever, never rendered |

A node is a **method** when it has a receiver pin — by default the first input when its type is
the namespace's own value type (`string`, `int`, `float`, `bool`, `array`, `map`, `set`, `struct`,
`bytes`, `path`, `datetime`). The receiver binds that pin, so it is not repeated inside `{ … }`;
when exactly one input remains it may be passed positionally:

```ts
const { arrayOut } = elements.push(item)
const label = "Topic {label}\nGoal: {Goal}".format({ goal: source.goal, label: source.label })
setElementText({ elementRef: "page/generated-date", text: date.format("%A, %B %-d, %Y") })
if (values.get(0).success) { … }
```

Numeric literal receivers need parentheses (`(5).abs()`), exactly as in JavaScript.

### Outputs

A call expression evaluates to the node's default output. Pick other outputs by pin name, either
through destructuring or field access:

```ts
const { hash: hash2 } = md5({ input: item3.content })
const { rows, rowCount } = sqlQuery({ session: session3, query: "SELECT …" })
const { value, exists } = getQueryParams({ paramName: "report_id" })
const aPICall = fetch({ request: makeRequest({ method: "GET", url: url }) })
const { text } = responseToText({ response: aPICall.response })
```

Destructuring renames with `pin: local` (`{ hash: hash2 }`); a binding used as a whole value
(`aPICall`) is the node's default output and still exposes every pin as a field.

## `use`

`use ns::*` opens a namespace so its members can be called bare. It is only valid at the top of
the file. The Rust forms are all accepted:

```ts
use string::*                      // glob: trim({ string: s }) / s.trim()
use ai::response                   // brings `response` into scope: response::make()
use github::copilot as copilot     // alias: copilot::createSession({ … })
use ui::{ setElementText, navigateTo }
```

Rendered boards open every namespace with two or more static call sites (alphabetically) and keep
single calls qualified; `use` lines are derived from the board, never stored on it. Two opened
namespaces that export the same member (`string::length` and `array::length`) are disambiguated
by the argument shape; when that is not enough the compiler reports the qualified alternatives
instead of guessing.

## Operators and sugar

| Sugar | Lowers to |
| --- | --- |
| `a + b`, `a - b`, `a * b`, `a / b`, `a % b`, `a ** b` | `int::add` / `float::multiply` / … by operand type |
| `"a" + b` | `string::concat` |
| `-x` | `0 - x` |
| `x += v`, `-=`, `*=`, `/=` | `x = x + v` |
| `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `\|\|`, `!` | the comparison / boolean nodes |
| `c ? a : b` | `types::select` |
| `` `Topic ${label}` `` | `string::format` with one pin per placeholder |
| `row.status = "done"` | `struct::set` on a mutable binding |
| `let record = …` / `record.title = title` | `struct::set` chain, rebinding `record` |
| `'single quotes'`, trailing `;` | accepted; rendered as `"…"` without `;` |

Template literal placeholders are named after the expression (`${label}` → `label`,
`${source.goal}` → `goal`, anything else → `argN`); static text that already contains `{name}`
is rejected because the format node would read it as a placeholder.

## Control flow

```ts
if (reportCreated) {
    return `Already created a report`
} else {
    reportCreated = true
}

for (const item of userConfiguration.sources) { … }          // control::forEach, item = value
for (const [index, item2] of rows3) { … }                     // + index
@parallel for (const item of items) { … }                     // control::parallelForEach
while (i < 3) { … }                                           // control::whileLoop
for (const parallelForEach of control::parallelForEach({ array: userConfiguration.sources, maxConcurrent: 15 })) {
    perpareNews({ source: parallelForEach.value })           // explicit handle form, still accepted
}
```

Nodes with several execution outputs open a block per arm; the arm labels are the node's exact
execution pin names. A trailing comment on an `if` whose condition is itself a node call names the
execution pin the branch follows:

```ts
aPICall {
    execSuccess: {
        const { text } = responseToText({ response: aPICall.response })
        return text
    }
    execError: {
        return `FAILED TO FETCH WEBSITE - SKIP`
    }
}

if (pathExists({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })) { // exec_out_exists
    const { content } = readToString({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })
    userConfiguration = fromString({ string: content })
} else { // exec_out_missing
    userConfiguration = { general: { arXiv: false, hn: false, news: false }, sources: [] }
    saveConfig({ config: userConfiguration })
}
```

## Functions and handlers

```ts
function filterSources(hash: string) {
    let elements: Elements[] = []
    for (const item3 of userConfiguration.sources) {
        const { hash: hash2 } = md5({ input: item3.content })
        if (hash != hash2) {
            const { arrayOut: arrayOut3 } = elements.push(item3)
            elements = arrayOut3
        }
    }
    userConfiguration = { general: userConfiguration.general, sources: elements }
}

eventsGeneric fetchPage(url: string, payload: Struct) {
    chatPushStep({ title: "Reading Website", description: url })
    …
}
```

A `function` is a Function layer; a handler block such as `eventsGeneric fetchPage(…)` declared
inside another function is an entry node that agents can invoke (`tools: [fetchPage]`). A function
that returns a value declares its return pins: `function double(n: int): (out: int) { return n * 2 }`.
User functions join the method tables too — the first parameter is the receiver, so
`fullName.parseName()` calls `function parseName(name: string)`.

## Anchors and editing

In the editable view every statement carries its node id as a trailing `//@n:<id>` comment
(variables `//@v:`, layers `//@l:`). Keep the anchor and you are editing that node; drop it and you
are deleting it — and deletions are blocked unless explicitly allowed. See the
[FlowScript blog post](https://flow-like.com/blog/flowscript/) for the reasoning behind the
round-trip invariants.

## Declarations (`.flow.d`)

The catalog is described by generated declaration files in `packages/ast/flow.d/` (one per
top-level category, plus one per package under `flow.d/packages/`). They are the "types" behind
editor completion, hover, the VS Code extension, and FlowPilot's `get_declarations` tool. Each
node is one `function` inside its `declare namespace` block:

```ts
declare namespace string {
    // === Utils/String ===

    /**
     * Checks if a string contains a substring
     * @node string_contains @receiver string @alias stringContains
     * @param string — Input String (receiver: `this` in `x.contains(...)`)
     * @param substring — Substring to search for
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns contains — Does the string contain the substring?
     */
    function contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;
}

declare namespace ai {
    namespace ml {
        /** … @node ai_ml_model_read @alias aiMlModelRead @impure has side effects / drives control flow */
        function read({ path: string }): Struct;
    }
}
```

- The `{ … }` object is the complete static call shape, receiver included.
- `this: T` marks the receiver pin (the node is a method on `T`); `T` is the pin type, or the
  schema title for a titled struct.
- `@node` is the catalog node type, `@receiver` the receiver pin, `@alias` the legacy camelCase
  spelling, `@impure` marks nodes with execution pins.
- `flow.d/names.json` maps every node type to `{ qualified, namespace, alias, flat, receiver,
  class, category }`; `node.flow.schemas.json` carries the JSON schemas of struct-typed pins, keyed
  by node type.

Regenerate them with `cargo test -p flow-like-catalog --test generate_signatures`.

### `get_declarations`

FlowPilot looks nodes up with `get_declarations`, which searches the embedded declaration index
by intent (`"string contains substring"`, `"gmail imap fetch mail"`) or by any exact spelling
(`string::contains`, `stringContains`, `string_contains`). The answer prints the `use` lines for
every namespace in the result, then one compact signature per node and, for nodes with a
receiver, both call forms:

```ts
// use string::*

1. string::contains — Checks if a string contains a substring [utils.flow.d :: Utils/String]
   function string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;
   // string::contains({ string: string, substring: substring, ignoreCase: ignoreCase })  or  string.contains({ substring: substring, ignoreCase: ignoreCase })
```
