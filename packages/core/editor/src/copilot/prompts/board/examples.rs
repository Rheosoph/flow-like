//! Verified FlowScript examples embedded in workflow prompts.

/// Compact FlowScript examples distilled from the non-anchored `tests/ast/*.flow` fixtures.
///
/// The examples intentionally show syntax and composition patterns rather than exact node choices:
/// the agent still has to call `get_declarations` and use the signatures returned for the current
/// catalog.
pub const FLOWSCRIPT_FEW_SHOT_EXAMPLES: &str = r##"
## FLOWSCRIPT NAMES AND SUGAR
- Every catalog node is `namespace::alias({ pin: value })` — `::` separates namespace path
  segments (`string::trim`, `ai::response::make`), `.` is field/method access on a VALUE. Argument
  names are the node's exact pin names; never rename an argument.
- `use ns::*` at the TOP of the file (before interfaces and variables, nowhere else) opens a
  namespace so its members are called bare: `use db::*` then `open({ name: "x" })`. Open a
  namespace when you call several of its members; rendered boards open every namespace with two or
  more call sites. `use a::b` (brings `b::…` into scope), `use a::b as c`, `use a::{ x, y }` and
  `use a::{ x as y }` are accepted too.
- A declaration with a `this:` parameter is also a METHOD on that value: `s.trim()`,
  `s.contains("?")` (one remaining input may be positional), `s.contains({ substring: "?",
  ignoreCase: true })`, `xs.push(item)`, `date.format("%Y-%m-%d")`. The receiver binds the `this`
  pin, so do not repeat that pin inside `{ … }`. Numeric literals need parens: `(5).abs()`.
- The legacy camelCase name (`stringTrim`, `openLocalDb`, shown as `@alias` in declarations)
  still resolves everywhere; prefer the qualified or method spelling in new code.
- Template literals lower to `string::format`: `` `Topic ${label}\nGoal: ${source.goal}` `` mints one
  pin per placeholder (named after the reference, the last `.segment`, or `argN`). Plain string
  `+` concatenates (`string::concat`). Literal text containing `{name}` is rejected — it would be
  read as a placeholder at runtime.
- Loops: `for (const item of items) { … }` (element = the loop node's `value`),
  `for (const [index, item] of items) { … }`, `@parallel for (const item of items) { … }`, and
  `while (cond) { … }` (`control::whileLoop`). The explicit `for (const it of control::forEach({ array: items }))`
  handle form, with `it.value`, stays accepted.
- Destructure MULTI-output nodes by pin name: `const { text, usage } = ai::invoke({ … })`,
  `const { value, exists } = ui::getElementValue({ … })`. A single-output call is the value
  itself — write `const digest = content.md5()`, never `const { hash: digest } = content.md5()`.
  `const x = call()` binds the default output.
- Top-level `const name = "literal"` infers `string | int | float | bool` (JSON object → `Struct`,
  array → `any[]`; a `{…}`/`[…]` initializer may hold only literals); `const name: Type = …`
  is still required for anything else. Operators:
  `+ - * / % **`, comparisons, `&& || !`, unary `-x`, `+= -= *= /=`, ternary `c ? a : b`.
  `'single quotes'` and trailing `;` are accepted and render as double quotes without `;`.

## FLOWSCRIPT FEW-SHOT PATTERNS
Use these as shape examples when the current board is empty or sparse. They are syntax patterns,
not a replacement for `get_declarations`: always use the exact qualified names and parameter
names returned by declarations. App Event interfaces/sinks (cron, chat UI, forms, API exposure)
are not catalog nodes; choose a compatible entry-node pattern below and let the outer assistant
configure the Event record after the board edit.

Actionable empty-board edits:
- New catalog nodes you author are created by **calls inside a function/event block**, for example
  `function run() { const db = db::open({ name: "email_vectors" }) }`. A rendered board may also
  show a `detached { ... }` block: an existing execution chain that no trigger reaches, so its
  nodes never run. Its statements are ordinary anchored calls whose inputs you may read and edit,
  but NEVER author a new `detached` block — to make that work run, move those statements (keeping
  their `//@n:` anchors) into an event or function body.
- Do not put node calls in top-level declarations. Top-level `const name = literal` /
  `const name: Type = literal` is only board state/defaults and must use literal defaults, not
  `db::open(...)` or another call.
- For `variable::get({ varRef: "NAME" })` and other `varRef` inputs, `NAME` must already exist as a
  board variable or be declared as a top-level FlowScript variable, for example
  `const NAME = ""`.
- Inside a function/event block, `const name = expression` and `let name = expression` can bind
  calls, literals, references, field access, arithmetic, objects, arrays, and template literals.
  Use `let` for a value reassigned across branches or loops. Local aliases may render with a
  different binding spelling; a literal alias alone creates no node or board variable.
- Object and call-argument fields always use colon syntax: `{ host: "imap.gmail.com", port: 993 }`.
  Do not write `{ host = "imap.gmail.com" }`; `expected Colon, found Assign` means a field used
  `=` where FlowScript expected `:`.
- Build database rows and payloads with object and array expressions, including dynamic values:
  `const row = { title: title, revision: revision + 1 }`, then `const rows = [row]`. FlowScript
  lowers computed fields and array elements to the catalog's struct and array nodes. To change
  one field on an existing struct, use `row.status = "done"` on a mutable binding or
  `row = row.set({ field: "status", value: "done" })`; both preserve its other fields.
- Functions ARE first-class in FlowScript: a `function name(params): (returns) { ... }` declaration
  creates a Function layer — its params become input pins, its returns become output pins, and its
  body nodes are placed inside the layer. Use functions to keep boards clean: a reusable helper, a
  per-page onLoad handler, and a widget-action handler should each be their own function rather than
  one long event block. You do NOT need `emit_commands` to create function layers; write the
  `function` in FlowScript. Reserve `emit_commands` for position-only node moves and canvas
  comments; placeholders and all layer mutations are not accepted.
- Every helper that executes `return ...` must declare a named return pin per returned value, for
  example `function classify(...): (isSupport: bool) { ...; return result.value }`. A bare
  `function classify(...) { return value }` has no output boundary pin and is invalid. Return
  values may be node outputs, parameters, literals (`return "done"`), or mutable `let` bindings;
  each declared return pin needs a matching return value. An event-level `return` accepts exactly
  one value.
- Mutable branch state: a `let` reassigned across `if`/`for` blocks promotes to a board variable
  with its initializer preserved (`let x = someCall(...)` then `x = other(...)` inside an arm is
  valid). Never reassign a `const` binding inside a branch arm — declare it with `let` instead.
  For a value chosen between branches, assign the same `let` in BOTH arms.
- Do not submit comments-only drafts, TODOs, "replace this later" placeholders, or prose
  implementation plans. After retaining the accepted active segment (the complete full-shape
  document under a `single` plan), if a compiler diagnostic identifies a missing declaration, call
  `get_declarations` once with concrete terms rather than inventing a stub.
- Before checking or committing, trace every explicit user requirement to reachable FlowScript.
  Preserve exact requested variable names/defaults, persisted field and status names, decision
  predicates, and success ordering (for example, acknowledge/mark complete only after downstream
  work succeeds). Catalog/type validity proves graph shape, not that this behavioral contract was
  preserved.
- Always call `write_flowscript` with the complete source in the `source` argument. Never call it
  with an empty string, a summary, or a markdown fenced block instead of the full document.
- Control flow IS supported: plain `if (booleanValue) { ... } else { ... }` creates a Branch node
  with both arms wired from its true/false pins, and the statement after the `if` continues
  correctly (fan-in from the arm ends and any untaken pin). Loops use `for (const item of items)`.
- A trailing comment on an `if` brace is an execution-pin LABEL only when the condition is itself
  a catalog/control-node call. On a boolean condition it is ordinary text and is kept as the first
  comment inside the branch body — it does NOT name an exec pin, so do not use it to steer
  execution. To wire specific arms, use an exact control-node call from `get_declarations` and
  label its arms.
- `!` negates a boolean: `if (!ready) { ... }`, and `while (!done) { ... }` is a real loop. Unary
  minus is an operator too: `-x` lowers to `0 - x`.

### Compiler-verified microexamples
These small examples are kept parseable and reconcilable in CI against the generated catalog
signature registry. Retrieve the same declarations before adapting them; copy the construct, not
the placeholder values.

- Treat each returned declaration as authoritative even when its function or argument shape is
  unintuitive; do not substitute a familiar library name or guessed pin.
- When a declaration repeats the same argument name, repeat that exact key in declaration order.
  Argument names are never renamed: do not invent names such as `a` / `b` for repeated pins, and
  do not put command-only `[#N]` selectors in FlowScript.
- A closed-schema `Struct` return permits only fields listed in its live schema note; use
  the catalog's typed accessor calls when supplied as companions. An open or schema-less Struct
  still does not justify guessed business fields: validate the intended accessor/declaration first.

#### Repeated same-name input pins
FlowScript accepts repeated object keys when the catalog declaration has repeated pins.
```flowscript-verified
function either(first: bool, second: bool): (result: bool) {
    const result = bool::or({ boolean: first, boolean: second })
    return result
}
```

#### Local aliases and computed payloads
Use ordinary expressions for payload construction. The compiler wires the calculated revision
into the row and the row into the array.
```flowscript-verified
function makePayload(title: string, revision: int): (payload: Struct) {
    const status = "ready"
    const nextRevision = revision + 1
    const row = { title: title, revision: nextRevision, status: status }
    const rows = [row]
    const payload = { rows: rows }
    return payload
}

eventsSimple() {
    const payload = makePayload({ title: "Review", revision: 2 })
    log::info({ message: payload })
}
```

#### Secret state, Generic conversion, a typed return, and a plain branch
`struct::get(...).value` is `any`. Convert it before a typed comparison; never compare the raw
Generic value directly with a string.
```flowscript-verified
use log::*

@secret
const expectedSender = ""

function senderMatches(payload: Struct, expected: string): (matches: bool) {
    const { value: rawSender } = payload.get({ field: "sender" })
    const sender = json::stringify({ value: rawSender })
    let matches = sender == expected
    return matches
}

eventsGeneric(payload: Struct) {
    const approved = senderMatches({ payload: payload, expected: expectedSender })
    if (approved) {
        info({ message: "approved sender" })
    } else {
        info({ message: "unapproved sender" })
    }
}
```

#### Loop bodies, impure continuation, and layer decomposition
Aim for 20–30 nodes per helper and split before the 100-node layer guideline. The statement after
the loop runs from its `done` output; the statement after `processBatch` continues from the helper's
Function `exec_out` boundary.
```flowscript-verified
use log::*

function validateBatch(items: any[]) {
    info({ message: items })
}

function processBatch(items: any[]) {
    for (const item of items) {
        info({ message: item })
    }
    info({ message: "batch complete" })
}

eventsSimple() {
    validateBatch({ items: ["first", "second"] })
    processBatch({ items: ["first", "second"] })
    info({ message: "all helpers continued" })
}
```

#### Function references
`tools: [echoTool]` is explicit FlowScript function-reference syntax emitted by the decompiler. It
is metadata for `agent::registerFunctionTools`, not a catalog input pin.

**Each array item must name a handler block — `name(params) { … }` — never a `function`.** A
`function` compiles to a Function layer whose signature becomes boundary pins, and a layer cannot be
referenced as a tool: it has no entry node for the runtime to trigger, so the reference is rejected
and the whole edit is refused. A handler block compiles to an event entry, which is what the agent
actually invokes: its **data outputs become the tool's arguments** and its **`return` becomes the
tool result**. Declare the handler inside the same scope that registers it.
```flowscript-verified
use agent::*

eventsSimple() {
    const agent = registerFunctionTools({
        agentIn: fromModel({ model: struct::make() }),
        tools: [echoTool]
    })
    log::info({ message: agent })
    echoTool(payload: Struct) {
        return json::stringify({ value: payload }).string
    }
}
```

#### Explicit policy for a node with several execution outputs
Never place a sequential statement directly after a multi-exec node. Bind the call, name every
execution arm shown by its declaration, and continue after the enclosing helper call.
```flowscript-verified
use http::*
use log::*

function fetchWithPolicy(url: string) {
    const request = makeRequest({ method: "GET", url: url })
    const result = fetch({ request: request })
    result {
        execSuccess: {
            info({ message: "request succeeded" })
        }
        execError: {
            error({ message: "request failed" })
        }
    }
}

eventsSimple() {
    fetchWithPolicy({ url: "https://example.com" })
    info({ message: "fetch helper continued" })
}
```

Common parse fixes:
Function names and field names below demonstrate grammar only; use `get_declarations` for exact
signatures before submitting.
```ts
// Bad: object fields use `=`
imap::connect({ host = "imap.gmail.com", port = 993 })

// Good
imap::connect({ host: "imap.gmail.com", port: 993 })

// Bad: function `const` binding is not a node call
function run() {
    const row = { id: "<CUID>", body: "<BODY>" }
}

// Good: local literal alias sugar, then a method call on the array
function run() {
    let rows = []
    rows = rows.push({ id: "<CUID>", body: "<BODY>" })
}

// Good: pass objects/literals directly to a real node call
function run() {
    db::batchUpsert({
        database: db::open({ name: "email_vectors" }),
        value: [{ id: "<CUID>", body: "<BODY>", sentiment: "neutral" }]
    })
}

// Also good: `const` binds a node-call output, then dynamic row fields are built explicitly
use ai::embedding::*

function run(embeddingBit: Struct) {
    const database = db::open({ name: "email_vectors" })
    const model = loadModel({ bit: embeddingBit })
    const vector = embedDocument({ model: model, queryString: "<BODY>" })
    const id = random::cuid()
    let rows = []
    let row = struct::make()
    row = row.set({ field: "id", value: id })
    row = row.set({ field: "body", value: "<BODY>" })
    row = row.set({ field: "vector", value: vector })
    rows = rows.push(row)
    database.batchUpsert({ value: rows, idRow: "id" })
}

// Bad: labelled branch with a non-call condition
function run() {
    if (rowCount > 0) { // exec_out_has_rows
        notify::user({ title: "Rows found" })
    }
}

// Good: plain boolean branch has no labels
function run() {
    if (rowCount > 0) {
        notify::user({ title: "Rows found" })
    }
}
```

### 1. Create typed state first, then build behavior around it
```ts
@category("Report")
const reportCreated = false
@category("Report")
const reportID = ""
@category("Report")
const reportRows: Struct[] = []

function generateReport() {
    const cuid = random::cuid()
    reportID = cuid
    const database = db::open({ name: "reports", userScoped: true, batchSize: 1000 })
    database.batchInsert({ value: reportRows })
}
```

### 2. Build dynamic database rows with struct::set chains
```ts
function ingestRows() {
    const database = db::open({ name: "reports", userScoped: true, batchSize: 1000 })
    const cuid = random::cuid()
    const date = datetime::now()
    let rows = []
    let row = struct::make()
    row = row.set({ field: "id", value: cuid })
    row = row.set({ field: "created_at", value: date })
    row = row.set({ field: "title", value: "Placeholder title" })
    rows = rows.push(row)
    database.batchUpsert({ value: rows, idRow: "id" })
}
```

### 3. Prefer readable intermediate constants for nested calls
```ts
use http::*
use types::*

function search(query: string, language: string, page: int, payload: Struct): (result: Struct) {
    const { result: pageNumber } = fallback({ value: page, default: 1 })
    const { result: lang } = fallback({ value: language, default: "en-US" })
    const q = ui::urlEncode({ input: query })
    const request = makeRequest({
        method: "GET",
        url: `https://search.flow-like.com/search?q=${q}&format=json&pageno=${pageNumber}&language=${lang}`
    })
    const response = fetch({ request: request })
    const json = responseToJson({ response: response })
    return json
}
```

### 4. Existing branches and loop bodies render as normal FlowScript blocks
```ts
use files::*
use path::*

function loadConfig() {
    if (pathExists({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })) { // exec_out_exists
        const content = readToString({ path: child({ parentPath: pathFromUserDir({ nodeScope: false }), childName: "config.json" }) })
        userConfiguration = json::parse({ string: content })
    } else { // exec_out_missing
        userConfiguration = { general: { news: false }, sources: [] }
        saveConfig({ config: userConfiguration })
    }
}

function processAllSources() {
    for (const source of userConfiguration.sources) {
        processSource({ source: source })
    }
}
```

### 5. DataFusion over Open Database follows open -> session -> register -> SQL
`df::createSession` needs only a session name — every other pin is an optional tuning default.
Create the session ONCE in the entry function and pass `session` to helpers as a `Struct`
parameter instead of recreating it per helper.
```ts
use df::*

function loadOverview(session: Struct): (rows: Struct[]) {
    const database = db::open({ name: "report_overview", userScoped: true, batchSize: 1000 })
    registerLance({ session: session, database: database, tableName: "reports" })
    const { rows } = sqlQuery({ session: session, query: "SELECT report_id, title, created_at FROM reports ORDER BY created_at DESC LIMIT 25;" })
    return rows
}

eventsSimple() {
    const session = createSession({ sessionName: "default" })
    const overview = loadOverview({ session: session })
    log::info({ message: overview })
}
```

### 6. Factor reusable logic into helper functions (each becomes a Function layer)
Declaring `function name(...) { ... }` creates a Function layer with boundary pins from its
signature. Prefer several small helpers over one giant event block. Note the split below: ordinary
reusable logic is a `function`, but anything an agent invokes is a **handler block** declared in the
scope that registers it, because only a handler compiles to an entry node the runtime can trigger.
```ts
use agent::*
use http::*

function runResearch(task: string): (answer: string) {
    const model = ai::findModel({})
    const history = history::fromString({ modelName: "", message: task })
    const agent = registerFunctionTools({
        agentIn: fromModel({ model: model, maxIter: 15, infiniteContext: false, contextMode: "summarize", maxContextTokens: 32000 }),
        tools: [fetchPage]
    })
    const { response } = invoke({ agent: agent, history: history })
    fetchPage(url: string) {
        const page = fetch({ request: makeRequest({ method: "GET", url: url }) })
        const text = responseToText({ response: page })
        return md::fromHtml({ html: text, skippedTags: ["script","style","iframe"] }).markdown
    }
    return ai::response::lastContent({ response: response }).content
}
```

### 7. Dashboard onLoad: query data, then populate page elements and widgets
Element refs (`"<page_id>/<element_id>"`) and the widget selector (`"Article"`) come from
`ui_inspect`, NOT from guessing. Keep the page-load logic in its own function and factor the
container fill into a helper. Iterate rows with `for (const row of rows)`.
```ts
use df::*
use ui::*

function briefingPageLoad() {
    const database = db::open({ name: "reports", userScoped: true, batchSize: 1000 })
    const session = createSession({ sessionName: "default" })
    registerLance({ session: session, database: database, tableName: "reports" })
    const { rows, rowCount } = sqlQuery({ session: session, query: "SELECT report_id, title, summary, created_at FROM reports ORDER BY created_at DESC LIMIT 25;" })
    setElementText({ elementRef: "e6x8wvsr1r6ouilc1qbop8uz/subline-right", text: `${rowCount} Briefing(s)` })
    fillArticles({ rows: rows })
    showScreen()
}

function fillArticles(rows: Struct[]) {
    clearChildren({ containerRef: getElement({ elementRef: "e6x8wvsr1r6ouilc1qbop8uz/archive-grid" }).element })
    for (const row of rows) {
        const elementRef = instantiateWidget({ widgetSelector: "Article", instanceId: row.report_id, dynPathTitle: row.title, dynPathSummary: row.summary, dynPathDate: row.created_at.format("%B %-d, %Y"), fnRefs: [openBriefing] })
        pushToContainer({ containerRef: getElement({ elementRef: "e6x8wvsr1r6ouilc1qbop8uz/archive-grid" }).element, elementRef: elementRef, position: -1 })
    }
}

eventsWidgetAction openBriefing(widgetInstanceId: string, eventName: string, actionContext: Struct, inputValues: Struct) {
    navigateTo({ route: `/briefing?report_id=${widgetInstanceId}` })
}
```
A widget action target is neither a `function` nor a generic handler: `ui::instantiateWidget`
validates that every `fnRefs` entry is a **Widget Action Event** and errors otherwise, so declare it
as `eventsWidgetAction name(...)`. Its parameters are the action payload the runtime delivers.

### 8. Drive a dashboard chart/table directly from a DataFusion query
`df::sqlQuery(...).table` is a `CSVTable` you can hand straight to `ui::pushCsvToChart` (format `CSV`).
Look up the chart element ref with `ui_inspect` first.
```ts
use df::*
use ui::*

function renderTrend() {
    const database = db::open({ name: "metrics", userScoped: true, batchSize: 1000 })
    const session = createSession({ sessionName: "default" })
    registerLance({ session: session, database: database, tableName: "metrics" })
    const { table } = sqlQuery({ session: session, query: "SELECT day, SUM(amount) AS total FROM metrics GROUP BY day ORDER BY day;" })
    pushCsvToChart({ elementRef: getElement({ elementRef: "yg7y9ag1wz4ib8wg95k93erh/trend-chart" }).element, library: "Nivo", format: "CSV", table: table, chartType: "Line" })
    showScreen()
}
```

When generating from an empty board, start with this kind of coherent skeleton: placeholder
literals/state when useful, small helper/tool functions, one entry function, and concrete
database/index/search node calls where needed. For dashboard work, call `ui_inspect` first so every
`ui::*` element reference and widget selector is real.
"##;

/// Domain-specific worked examples covering the widely-used catalog areas (mail, LLM invoke,
/// ingestion/search, struct arithmetic, DataFusion reads). Every fenced block below is compiled
/// against the real catalog by `prompt_example_validation.rs` — a broken example fails CI.
pub const FLOWSCRIPT_DOMAIN_EXAMPLES: &str = r##"
## DOMAIN EXAMPLES (verified against the live catalog)

### Email round-trip: fetch unseen mail, send a tagged draft for approval, persist, mark seen
Connection nodes take real credentials — leave them as empty strings for the user to fill.
```ts
use imap::*

eventsSimple triageInbox() {
    const conn = connect({ host: "", port: 993, username: "", password: "" })
    const inbox = conn.inbox({ inbox: "INBOX" })
    const listed = inbox.listMails()
    const smtp = smtp::connect({ host: "", port: 587, username: "", password: "" })
    const db = db::open({ name: "Mail Drafts", userScoped: false, batchSize: 1000 })
    for (const mail of listed) {
        const reference = mail.toReference()
        const full = reference.fetchMail()
        const { subject, plain } = full.getContent()
        const { from } = full.getHeaders()
        const sender = json::stringify({ value: from, pretty: false })
        const draftId = random::cuid()
        const tagged = `[DRAFT ${draftId}] ${subject}`
        let row = struct::set({ structIn: {}, field: "id", value: draftId })
        row = row.set({ field: "sender", value: sender })
        row = row.set({ field: "subject", value: subject })
        row = row.set({ field: "status", value: "awaiting_approval" })
        // Database writes have (execOut, error) outputs: bind and branch instead of sequencing.
        const saved = db.upsert({ value: row, idRow: "id" })
        saved {
            execOut: {
                smtp::send({ connection: smtp, from: "", to: "", subject: tagged, bodyText: plain })
                // Mark-as-seen takes the EmailRef (connection/inbox/uid), not the fetched mail.
                markSeen({ email: reference, markAsSeen: true })
            }
            error: {
                log::info({ message: "draft persist failed; leaving mail unseen for a retry" })
            }
        }
    }
}
```

### LLM invoke plus struct-field arithmetic (read the field, coerce, then write it back)
`row.revision + 1` directly is INVALID: a struct field read is Generic, so coerce first.
```ts
function reviseDraft(row: Struct, feedback: string): (updated: Struct) {
    const llm = ai::findModel({})
    const { result: revised } = ai::invokeSimple({ model: llm, systemPrompt: "Revise the reply draft using the reviewer feedback. Return only the new draft body.", prompt: feedback })
    let updated = row.set({ field: "body", value: revised })
    const { value: revision } = updated.get({ field: "revision" })
    const { typeOut: parsed } = types::tryTransform({ typeIn: revision })
    const nextRevision = int::add({ integer1: parsed, integer2: 1 })
    updated = updated.set({ field: "revision", value: nextRevision })
    return updated
}
```

### Knowledge ingest: extract, chunk, embed, persist searchable rows
The embedding model loads from a Bit; leave the bit id empty for the user to select.
```ts
use ai::embedding::*

eventsSimple ingestDocument() {
    const bit = ai::loadBit({ bitId: "" })
    const embedder = loadModel({ bit: bit })
    const db = db::open({ name: "Library Chunks", userScoped: false, batchSize: 1000 })
    const chunks = ai::processing::chunkText({ model: embedder, text: "document text", overlap: 80 })
    for (const chunk of chunks) {
        const vector = embedDocument({ model: embedder, queryString: chunk })
        const id = random::cuid()
        let row = struct::set({ structIn: {}, field: "id", value: id })
        row = row.set({ field: "text", value: chunk })
        row = row.set({ field: "vector", value: vector })
        db.upsert({ value: row, idRow: "id" })
    }
}
```

### Semantic search with an explicit empty-result path
Search reads have a single `execOut`; detect emptiness from the values array, not from an arm.
```ts
use ai::embedding::*

function answerFromLibrary(question: string): (answer: string) {
    let answer = "No matching knowledge found."
    const bit = ai::loadBit({ bitId: "" })
    const embedder = loadModel({ bit: bit })
    const db = db::open({ name: "Library Chunks", userScoped: false, batchSize: 1000 })
    const queryVector = embedDocument({ model: embedder, queryString: question })
    const found = db.vectorSearch({ vector: queryVector, limit: 5 })
    if (found.length() > 0) {
        answer = json::stringify({ value: found, pretty: true })
    }
    return answer
}
```

### Impure function bodies END on a plain single-output statement so callers can continue
Every impure `function` must feed its exec_out: close all control flow, then finish the body with
one plain trailing statement that has a single execution output (a log, a variable set, or a
simple write). Never end a function body inside a branch/arm block, and never end it on a
multi-output call — put that call earlier and let a plain statement finish the body.
```ts
function persistDecision(row: Struct, approved: bool): (status: string) {
    let status = "rejected"
    if (approved) {
        status = "sent"
    }
    const updated = row.set({ field: "status", value: status })
    log::info({ message: json::stringify({ value: updated, pretty: false }) })
    return status
}
```
"##;
