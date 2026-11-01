---
title: FlowScript
description: The text projection of a board — syntax, calls, namespaces, and the declaration files behind it
sidebar:
  order: 18
---

**FlowScript** is a TypeScript-flavoured rendering of a board. Every board can be shown as
FlowScript, edited as text, and applied back onto the graph; it is also the language FlowPilot
reads and writes when it changes a workflow. This language reference combines examples from
rendered boards with authoring patterns that map back to the graph.

For a guided introduction, read [FlowBook's anatomy of a FlowScript
document](https://book.flow-like.com/part-2/06-anatomy-of-a-flowscript-document/)
and its [first Flow chapter](https://book.flow-like.com/part-1/04-first-flow-incident-triage/).
FlowBook teaches the language through connected examples; this page is the
current syntax and authoring reference. Check each chapter's Draft status.

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
    const database = open({ name: "report_overview", userScoped: true, batchSize: 1000 })
    const session = df::createSession({ sessionName: "default" })
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
its arguments: an object whose keys are the node's exact input pin names, optionally preceded by
leading inputs passed positionally in pin order:

```ts
const markdown = md::fromHtml({ html: text, skippedTags: ["script", "style"] })
const same = md::fromHtml(text, ["script", "style"])
log::info(source, { toast: false })
```

Rendered boards pass at most two values positionally (a method receiver counts as one) and stop
at the first input that is not set, at an object literal (a trailing `{ … }` is the named
object), and at a bare boolean, number or `null` unless it is the call's only argument. Everything
after that is named. Naming an argument is always valid; giving the same pin positionally and by
name is an error.

`::` separates namespace segments; `.` is field or method access on a *value*. The same node can
be spelled three ways, and all three resolve to the same node:

| Spelling | Example | When |
| --- | --- | --- |
| qualified | `string::trim(s)` | always works; the canonical static form |
| method | `s.trim()` | nodes whose declaration has a `this:` parameter |
| legacy flat | `stringTrim({ string: s })` | the camelCase node type; accepted forever, never rendered |

A node is a **method** when it has a receiver pin — by default the first input when its type is
the namespace's own value type (`string`, `int`, `float`, `bool`, `array`, `map`, `set`, `struct`,
`bytes`, `path`, `datetime`). The receiver binds that pin, so it is not repeated inside `{ … }`;
the next input may follow positionally:

```ts
const arrayOut = elements.push(item)
const label = "Topic {label}\nGoal: {Goal}".format({ goal: source.goal, label: source.label })
setElementText({ elementRef: "page/generated-date", text: date.format("%A, %B %-d, %Y") })
if (values.get(0).success) { … }
```

Numeric literal receivers need parentheses (`(5).abs()`), exactly as in JavaScript.

### Outputs

A call expression evaluates to the node's default output. A node with a single data output is
the value itself; pick outputs of a multi-output node by pin name, either through destructuring
or field access:

```ts
const hash2 = md5({ input: item3.content })
const { rows, rowCount } = sqlQuery({ session: session3, query: "SELECT …" })
const { value, exists } = getQueryParams({ paramName: "report_id" })
const aPICall = fetch({ request: makeRequest({ method: "GET", url: url }) })
const text = responseToText({ response: aPICall.response })
```

Destructuring renames with `pin: local` (`{ value: reportId }`); a binding used as a whole value
(`aPICall`) is the node's default output and still exposes every pin as a field.

### Directory scope

`files::cacheDir`, `files::storageDir`, and `files::userDir` accept `nodeScope`, which defaults
to `false`. When it is `true`, the directory path includes the directory node's ID. Each call
site creates a separate node, so two calls with `nodeScope: true` return different paths, even
inside the same function or event.

If several steps need the same file, store the path in one variable and reuse it:

```ts
const file = files::cacheDir({ nodeScope: true }).child("document.pdf")
file.writeBytes({ content: documentBytes })
const pages = ai::processing::extractDocument({ file: file })
```

Alternatively, set `nodeScope: false` to use the base directory without the node ID. Other
scope options, such as `userScope` on `cacheDir`, still apply.

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
        const text = responseToText({ response: aPICall.response })
        return text
    }
    execError: {
        return `FAILED TO FETCH WEBSITE - SKIP`
    }
}

if (pathExists({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })) { // exec_out_exists
    const content = readToString({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })
    userConfiguration = fromString({ string: content })
} else { // exec_out_missing
    userConfiguration = { general: { arXiv: false, hn: false, news: false }, sources: [] }
    saveConfig({ config: userConfiguration })
}
```

### Variables in loops

Inside a `for` or `while` body, a mutable `let` runs its initializer each time execution reaches
the declaration. For example, `let count: int = 0` starts at zero on each visit, including
when the declaration is inside a branch in the loop. A variable declared before the loop keeps
its value between iterations, so use that placement for a running total.

To reset an outer variable for each iteration, assign its starting value at the beginning of the
body. This also works with older compiler versions that only stored a loop-local literal as the
variable's default:

```ts
function logOccurrences(needles: int[], values: int[]) {
    let count: int = 0
    for (const needle of needles) {
        count = 0
        for (const value of values) {
            if (value == needle) {
                count += 1
            }
        }
        log::info({ message: `${needle}: ${count}`, toast: false })
    }
}
```

Each logged count belongs to one needle. You can also move the loop body into a helper function
and declare the counter there; function-local variables start from their defaults on each call.

## Groups (`@collapse`)

`@collapse("Name") { … }` wraps statements in a **group**: a Collapsed layer on the canvas that
holds every node they produce, including inlined pure calls and the nodes behind `if`, loops,
assignments and `return`. Groups are presentation only. Execution order, bindings and `let` scope
behave exactly as if the statements were written in the enclosing block, so `session` below is
still in scope after its group closes.

```ts
eventsSimple publishLatestReport() {
    @collapse("Load") {   //@l:tq3hw8zk1lr0bmx6c2vye9pa
        const database = open({ name: "report_overview", userScoped: true, batchSize: 1000 })
        const session = df::createSession({ sessionName: "default" })
        session.registerLance({ database: database, tableName: "reports" })
    }
    const { rows } = session.sqlQuery({ query: "SELECT report_id FROM reports ORDER BY created DESC LIMIT 1;" })
    @collapse("Publish/Notify") {                                 // no anchor: two new groups
        if (reportCreated) {
            @collapse("/") {
                log::info({ message: "Report already published", toast: false })
            }
        } else {
            navigateTo({ route: `/briefing?report_id=${rows[0].report_id}` })
        }
    }
}
```

Nested blocks build the hierarchy. The path is read from the raw string literal:

```ts
@collapse("Publish/Notify") { … }   // Notify inside Publish, same as two nested blocks
@collapse("Input\/Output") { … }    // one group named Input/Output
@collapse("/Cleanup") { … }         // Cleanup directly under the section root
@collapse("/") { … }                // no group and no anchor: back at the section root
```

The section root is the board root, module or Function layer the statements belong to.
`@collapse("/")` steps out of every group; above, it takes a branch arm out of the group its `if`
sits in. `""` is one group with an empty name; any other empty segment (`"A//B"`, `"A/"`) is an
error. Groups are statements: write them in an event, function, handler, `detached`, branch arm
or loop body, never at the top level or directly in a `module` body. The argument is a string
literal followed by `{`, and `@collapse` does not combine with other decorators.

The `{` line carries the group's `//@l:` anchor. An anchored block is that group: edit the name to
rename it, or nest the block under another group to re-parent it. A block without an anchor
creates a new group, shared by every unanchored block with the same name under the same parent
in the same event or function. An anchor copied from another board names no group there, so its
block also creates a new group, shared by every block carrying that same anchor.
Rendered boards nest one block per level, each with its own anchor; a group whose statements are
not consecutive renders as several blocks with the same anchor.

Applying the document moves the nodes of every statement written inside a block into that group;
inlined pure calls follow when all of their consumers move to the same group. A document that
contains any `@collapse` block is authoritative for grouping: statements written outside every
block move out of their group to the section root, and a group the edit leaves empty is removed.
A document without a single `@collapse` block moves nothing out and removes no group. Event
headers never move.

## Functions and handlers

```ts
function filterSources(hash: string) {
    let elements: Elements[] = []
    for (const item3 of userConfiguration.sources) {
        const hash2 = md5({ input: item3.content })
        if (hash != hash2) {
            const arrayOut3 = elements.push(item3)
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

An event parameter written `name?: Type` is optional: callers may omit it and the runtime fills
the type's empty value (`""`, `0`, `false`, `[]`, `{}`), while `name?: Type = literal` stores that
literal as the default instead. `payload` is never optional, and function parameters carry neither
`?` nor `=`.

```ts
eventsGeneric fetchPage(url: string, retries?: int = 3, payload: Struct) {
    …
}
```

A `function` becomes a Function layer. Declare named output pins for returned values:
`function double(n: int): (out: int) { return n * 2 }`. User functions also join the method tables.
The first parameter is the receiver, so `fullName.parseName()` calls
`function parseName(name: string)`.

A helper function may omit `return` when it has no outputs. If it returns values, use one
`return` as the last statement in the function body, outside every branch, loop and execution
arm. Early, nested and multiple function returns are rejected. Initialize a mutable `let` with
a literal, assign it in the branches, then return it after the control block:

```ts
function chooseLabel(enabled: bool, left: string, right: string): (label: string) {
    let label = ""
    if (enabled) {
        label = left.trim()
    } else {
        label = right.trim()
    }
    return label
}
```

Event handlers keep separate return semantics. A Generic Event can return from a branch and
emits one value; the return examples under [Control flow](#control-flow) are handler examples.
A handler such as `eventsGeneric fetchPage(…)` declared inside a function is an entry node that
agents can invoke with `tools: [fetchPage]`. Its returns belong to that handler, not its enclosing
function.

### Event and tool descriptions

Place `@description("…")` directly above an event or nested handler to describe what it does:

```ts
@description("Fetch a web page and return its text for the agent to read.")
eventsGeneric fetchPage(url: string, payload: Struct) {
    const response = http::fetch({ request: http::request({ method: "GET", url: url }) })
    return http::toText({ response: response })
}
```

**Apply to board** writes the description to the event's entry node. When an agent uses that
handler through `tools: [fetchPage]`, its tool definition includes the same description. The
decorator also works on events inside modules and on handlers nested in functions or events.

Descriptions appear again when the board is rendered as FlowScript. Use `\n` inside the string
for line breaks and `\"` for quotes. Omitting the decorator keeps the current board description;
use `@description("")` to clear it. Ordinary comments do not set descriptions.

## Anchors and editing

In the editable view every statement carries its node id as a trailing `//@n:<id>` comment
(variables `//@v:`, layers `//@l:`). Keep the anchor and you are editing that node; drop it and you
are deleting it — and deletions are blocked unless explicitly allowed. See the
[FlowScript blog post](https://flow-like.com/blog/flowscript/) for the reasoning behind the
round-trip invariants.

`//@l:` also identifies [groups](#groups-collapse): it sits on the `{` line of a `@collapse` block.

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
