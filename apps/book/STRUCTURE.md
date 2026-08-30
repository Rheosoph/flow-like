# FlowBook structure

This is the editorial architecture for the first edition of *FlowBook: A Developer's Guide to
Flow-Like*. It is a teaching sequence, not a mirror of the documentation sidebar.

## Front matter

### Title page

**FlowBook: A Developer's Guide to Flow-Like**
*Build reliable software as typed text and a visible workflow*
*Software that explains itself.*

### Foreword — The domain expert belongs in the room

A short outside perspective from an enterprise operator, domain expert, or technical
leader. The foreword should establish the human cost of software whose logic and failure
modes are legible only to its original authors.

### How to read this book

- **The builder path** follows Chapters 1–13, then 17–20 and 23; Chapters 14–16 are optional
  implementation mechanics.
- **The developer path** follows every language and round-trip chapter.
- **The extender path** adds the WASM and package chapters.
- **The operator path** focuses on evidence, versions, governance, runtime, and deployment.
- **Current / Preview / Vision** markers distinguish shipped behavior from changing surfaces
  and architectural intent.

### Conventions

Explain FlowScript formatting, code-versus-graph panels, node and pin callouts, terminal
transcripts, intentionally failing examples, and version badges. Establish that generated
node signatures are versioned reference material and that examples target a named Flow-Like
release.

### Introduction: One Program, Two Ways to See It

Define FlowBook, Flow-Like, Studio, App, Flow, Board, canvas, FlowScript, and runtime before the
terms carry explanatory weight. Use one conceptual diagram to show the whole model and its
connection to existing systems.

Give technical decision-makers the business case on one page: an AI-first company needs shared
application and execution contracts as software output rises. Make incremental adoption explicit;
the existing estate remains connected through typed nodes, packages, Events, and APIs.

Establish the canvas and FlowScript editor as two authoring views inside Studio. Explain why text
scales and why generic code boxes weaken the graph. Introduce AI only after this model is clear.
Separate authoring agents from model calls during a run, state the deterministic-first rule, and
defer model selection and usage-control details to the later AI chapters.

**Evidence:** SRC-AI-AUTHORING, SRC-AI-DETERMINISTIC-FIRST, SRC-AI-USAGE-CONTROLS,
SRC-AI-MODEL-SELECTION, and SRC-AI-MODEL-INVENTORY.
**Interview source:** INT-02 AI-era opening follow-up is complete.

---

# Part I — Software That Explains Itself

Part I earns the premise before teaching syntax. The reader experiences the problem, the
design conviction, the platform vocabulary, and the complete dual-view loop in a small,
deterministic application.

## Chapter 1: The 3 A.M. Call

**Summary:** Open inside the major-incident call that inspired Flow-Like: a costly outage,
an unfamiliar system, and a room waiting for the one person who understood it. Turn that
moment into the central question of the book: why does critical software not explain itself?

### 1.1 Waiting for context

Reconstruct the incident as a scene: what was known, what was invisible, who was missing,
and why every passing hour mattered.

### 1.2 The explanation had drifted away

Show why diagrams, tickets, runbooks, and tribal knowledge drift away from the executing
system even in well-intentioned organizations.

### 1.3 Start at the failed operation

Reimagine the same incident with a visible graph, typed boundaries, per-node logs, and a
runtime that can lead an unfamiliar responder to the responsible operation.

### 1.4 Domain knowledge belongs in the program

Connect the domain expert's knowledge, a developer's need for text, and an operator's need for
run evidence. Show how those requirements led to one Flow with two authoring views and a platform
around it.

**Interview source:** INT-01 is complete. The non-confidential scene detail is that no technical
failure was visible; domain experts could report only that production was on hold.

## Chapter 2: The Manifesto: Constrained Freedom

**Summary:** State the principles that govern FlowScript and Flow-Like. The manifesto makes
the trade explicit: give up some unconstrained implementation freedom to gain reliability,
legibility, portability, and safe reuse.

### 2.1 Reliability begins during authoring

Define reliability as a design property that starts before deployment and includes useful run
evidence and reviewable changes.

### 2.2 Readability is operational

Argue that structure is part of correctness when software is maintained by teams, domain
experts, operators, and AI systems.

### 2.3 Hard work should move quickly

Explain how a large, typed node library and built-in platform services remove repetitive
infrastructure work without removing the ability to solve real problems.

### 2.4 Why text belongs beside the graph

Follow the design chain explicitly: useful platforms need low-level building blocks; honest
low-level graphs grow large; arbitrary inline code makes them opaque; FlowScript keeps the
same typed blocks manageable in text without creating a hidden escape hatch.

### 2.5 Constraints must state their boundary

Introduce typed connections, capability-scoped extensions, deletion guards, secrets kept
out of source, versioning, and reviewed publication as examples of constraints that carry
their weight.

### 2.6 Broad outcomes, opinionated implementation

Develop the central trade: broad application scope, but fewer arbitrary ways to smuggle in
unreviewed code, credentials, deployment patterns, or invisible side effects.

### 2.7 The product has to live by the manifesto

Close with the standard for future design decisions and with the tension it creates: the
platform must make the robust path practical enough that users do not need the shortcut.

**Interview source:** INT-01 and INT-02 are complete. INT-09 will test the manifesto against
decisions where the team rejected a faster implementation.

## Chapter 3: One Platform, One Flow Model

**Summary:** Give readers the minimum complete map of Flow-Like before they write code.
FlowScript defines logic; Flow-Like supplies the authoring surfaces, runtime, data layer,
interfaces, collaboration, governance, and deployment environments around it.

### 3.1 App, Flow, and Board

Fix the vocabulary: an App is the project and governance boundary, a Flow is executable
logic, and a Board is that Flow's persisted graph representation.

### 3.2 Studio and the two authoring views

Present Studio as the complete desktop application. Its canvas editor and FlowScript editor are
equal authoring views over the same model, while the Board is what the current system persists and
runs.

### 3.3 Nodes, pins, wires, and layers

Introduce typed operations, data connections, execution connections, pure and impure work,
and layers as the vocabulary shared by the code and graph views.

### 3.4 Existing systems stay in the picture

Show how catalog nodes and packages connect APIs, services, files, structured data, and device
capabilities. Make incremental adoption and continued systems of record explicit.

### 3.5 Events connect a Flow to callers

Separate an event node inside a Flow from an App Event that exposes it to a schedule, API,
chat, page, quick action, or another supported trigger.

### 3.6 Data, people, and execution share the App boundary

Place files, structured data, members, roles, Event authentication, runtime credentials, and run
evidence around the same App. Preview local, remote, and hybrid execution without turning this
chapter into a deployment guide.

**Implementation evidence:** SRC-PLATFORM, SRC-APP-MODEL, SRC-IDENTITY-PERMISSIONS, and
SRC-EXECUTION.
**Interview source:** INT-02 is complete; INT-07 must classify production-supported,
preview, and roadmap surfaces.

## Chapter 4 — First Flow: Incident Triage in Two Views

**Summary:** Build and run a useful Flow before teaching the language systematically. The
reader changes the same program from the canvas and FlowScript editor, then diagnoses a failing input
from its run evidence.

### 4.1 The contract: accept, classify, respond

Define a small typed incident record and a deterministic severity rule with no external
services or credentials.

### 4.2 Build it visually

Create an event, normalize the report, compare severity, branch, and log or return a result
using four to six nodes.

### 4.3 Read the same Flow as source

Open FlowScript and connect each declaration, call, value, and block to the graph the reader
just built.

### 4.4 Change text, watch the graph

Edit the escalation threshold, preview the command plan, apply it, and locate the changed
node on the canvas.

### 4.5 Change the graph, watch the text

Add a visual normalization or warning step and observe the canonical source that results.

### 4.6 Break it on purpose

Run normal, urgent, and malformed inputs; use the run log as a flight recorder and find the
specific failing node.

### 4.7 Save the first known-good version

Snapshot the Flow and explain why production entry points can later target a tested version.

**Worked material:** `examples/incident-triage/triage.flow`, kept in canonical parser/render
form by `packages/ast/tests/book_examples.rs`. Its six constituent nodes and mappings have
direct reconciliation/runtime coverage; add a combined UI fixture and screenshots before the
chapter is marked final.
**Evidence:** SRC-UNIVERSITY, SRC-FLOWSCRIPT, SRC-APPLY, SRC-EDITOR, SRC-OBSERVABILITY, and
SRC-VERSIONING.
**Interview source:** INT-04 defines node-attributed evidence and the distinction between modeled
failure outcomes and unexpected node errors. The opening incident is reused only as an explicit
counterfactual; the manuscript does not invent a customer outcome.

---

# Part II — Thinking and Writing in Flows

Part II teaches the language by continually answering two questions: “What does this source
mean?” and “What graph and runtime behavior does it represent?”

## Chapter 5 — Nodes, Pins, Wires, and Execution

**Summary:** Establish the execution model before syntax becomes complex. Readers learn why
data dependencies and control dependencies are distinct and why visibility depends on both.

### 5.1 A node is a typed operation

Relate catalog nodes to function calls while retaining the metadata, permissions, quality
signals, and execution behavior that make a node more than an arbitrary function.

### 5.2 Data pins carry values

Explain direction, type, value shape, schemas, defaults, and why incompatible connections
fail during authoring.

### 5.3 Execution pins carry order

Show how side-effecting work enters an execution chain and why data availability alone does
not imply that an operation should run.

### 5.4 Pure work is demand-driven

Contrast pure nodes with impure nodes and connect the concept to expression-shaped source.

### 5.5 Explicit sequence and explicit parallelism

Explain the topology rule: a chain is sequential and multiple ready successors can advance
concurrently. Canvas position never supplies order. Use sequence, parallel, loop, and gather
operations when the intended contract should be explicit, and show that a failing branch does
not cancel its siblings by default. After siblings settle, any unhandled child failure must make
the aggregate terminal status Failed; a completed error handler keeps its evidence without
failing the run.

### 5.6 Layers organize; functions are callable

Give readers the distinction they will need before functions appear in FlowScript.

**Evidence:** SRC-GRAPH-MODEL, SRC-UNIVERSITY, and SRC-EXECUTION.
**Interview source:** INT-01 supplies the Unreal Blueprints influence; INT-03 defines the
language and two-view contract; INT-04 supplies concurrency and failure-path doctrine, with
current aggregate-status caveats recorded in the interview ledger.

## Chapter 6 — Anatomy of a FlowScript Document

**Summary:** Walk through the canonical file from top to bottom and explain why its order is
stable: imports, interfaces, top-level variables, functions, and event entry blocks.

### 6.1 Canonical source, not stylistic trivia

Explain formatting, normalization, optional semicolons, accepted quote styles, and why one
canonical rendering makes diffs and round trips easier to reason about. Describe the surface
as TypeScript-familiar, with Rust-style `use` and `::` paths and Flow-specific declarative
annotations; do not reduce the language to a percentage blend.

### 6.2 `use` declarations

Preview namespaces and Rust-like imports without yet covering full call resolution.

### 6.3 Interfaces

Show the readable surface of schemas and how declarations support variables and pin values.

### 6.4 Top-level variables

Introduce Flow state and decorators while warning readers not to import JavaScript meanings
for `const` and `let` without qualification.

### 6.5 Functions

Show how a callable section maps to a Function layer and typed boundary pins.

### 6.6 Events

End at the entry blocks that cause work to run and explain the difference between an event
type and its optional given name.

**Worked material:** `examples/document-anatomy/anatomy.flow`, kept in canonical parser/render
form by `packages/ast/tests/book_examples.rs`. Add catalog-aware reconciliation, a real run, and
editor diagnostics captured against the publication release.
**Evidence:** SRC-FLOWSCRIPT, SRC-LANGUAGE-AST, SRC-PARSER-RENDERER, SRC-RECONCILER,
SRC-DECLARATIONS, and SRC-VARIABLES.
**Interview source:** INT-03 settles the author-facing definition, the conservative lineage
description, platform-only execution, and the `const`/`let` distinction.

## Chapter 7 — Values, Types, Collections, and Interfaces

**Summary:** Teach FlowScript's pin- and schema-oriented type system on its own terms rather
than presenting it as a smaller TypeScript type system.

### 7.1 Scalars and `any`

Cover strings, integers, floats, booleans, dates, paths, bytes, structures, and the places
where generic values are necessary.

### 7.2 Value shapes

Explain normal values, arrays, maps, and sets and how shape is part of connection
compatibility.

### 7.3 Inference and its boundaries

Show scalar literal inference, when explicit annotations are required, and why `null` or
complex defaults cannot always determine a safe type.

### 7.4 Interfaces as visible schemas

Build a typed incident or document record with optional fields, defaults, arrays, maps,
unions, and literal alternatives.

### 7.5 Struct fields and schema propagation

Follow typed fields through node inputs and outputs, including bracket syntax for names that
cannot be written as identifiers.

### 7.6 Edges and preserved metadata

Test complex schema shapes against the release used for the book. When a schema exceeds the
readable surface, show what metadata is preserved and how to report an unsupported round trip
rather than presenting an unverified permanent limitation.

**Worked material:** `examples/types-and-interfaces/types.flow`, kept in canonical parser/render
form by `packages/ast/tests/book_examples.rs`. Add catalog-aware reconciliation, exact
schema-derived pin inspection, and a captured runtime-drift failure against the publication
release before marking the chapter final.
**Evidence:** SRC-LANGUAGE-AST, SRC-TYPES, SRC-STRUCT-SCHEMAS, SRC-LANGUAGE-TESTS, and
SRC-OBSERVABILITY.
**Interview source:** INT-03 defines the type-safety boundary: reject known incompatibilities as
early as possible, make typed structures the easiest path, and localize unknowable external drift
at the responsible runtime node.

## Chapter 8 — Calling the Node Library

**Summary:** Treat the catalog as FlowScript's expansive, typed standard library. Readers
learn how names, arguments, outputs, methods, and generated declarations connect source to
real nodes.

### 8.1 Qualified calls

Use `namespace::operation({ pinName: value })` as the canonical mental model: named arguments
are exact input-pin names.

### 8.2 Imports and aliases

Cover namespace imports, globs, selected members, aliases, collision handling, and when the
renderer chooses an import automatically.

### 8.3 Method form

Explain receiver pins and calls such as `text.trim()` without suggesting arbitrary prototype
extension or object methods.

### 8.4 Single and multiple outputs

Show default outputs, field selection, and object destructuring by output-pin name. Explain
why array destructuring is intentionally unsupported.

### 8.5 Generated `.flow.d` declarations

Read a declaration as the exact bridge between documentation, completion, the VS Code
extension, FlowPilot lookup, and the catalog node contract.

### 8.6 A large library without a memory test

Teach discovery by intent and type. Mention the current generated entry count only as a
versioned observation, never as a permanent product slogan.

**Worked material:** `examples/calling-the-catalog/catalog.flow`, kept in canonical parser/render
form by `packages/ast/tests/book_examples.rs`. Add catalog-aware reconciliation, a real run, and
captured Board and completion views against the publication release before marking the chapter
final.
**Evidence:** SRC-DECLARATIONS, SRC-FLOWSCRIPT, SRC-CATALOG-DISCOVERY, and SRC-NODE-MIGRATION.
**Interview source:** INT-03 establishes compatibility- and type-led discovery, desired candidate
ranking, and safe node migration with visible error annotation when automatic repair is unsafe.
The built-in-catalog versus package boundary remains a later Chapter 21 interview dependency.

## Chapter 9 — Expressions, Operators, and Readable Sugar

**Summary:** Show that familiar syntax is a legible projection of explicit graph operations.
Every feature is taught together with the node or wiring it lowers to.

### 9.1 Arithmetic, comparison, and Boolean expressions

Cover precedence, supported operators, type-directed node selection, and cases where the
language refuses to invent missing semantics.

### 9.2 Unary and compound assignment

Explain normalized negative expressions and how `+=`, `-=`, `*=`, and `/=` expand into reads,
operations, and writes.

### 9.3 Conditional expressions

Distinguish a value-selecting ternary from execution branching.

### 9.4 Template literals

Show placeholder naming, formatting-node lowering, escaping, and why ambiguous literal
placeholders are rejected.

### 9.5 Field reads and writes

Follow member access and mutable struct field assignment through getter/setter nodes and
rebinding.

### 9.6 Sugar must round-trip honestly

Explain the rule: render sugar only when it can reproduce the exact graph contract without
discarding meaningful inputs.

**Evidence:** SRC-PARSER-RENDERER, SRC-RECONCILER, SRC-LANGUAGE-TESTS,
SRC-EXPRESSIONS-SUGAR, and SRC-EXPLICIT-CONVERSIONS.
**Interview dependency:** Resolved in INT-03: reject ambiguous coercion, preserve meaningful
configuration in explicit call form, and describe field assignment as temporal rebinding over
visible Struct values.
**Worked material:** `examples/readable-sugar/sugar.flow`, kept in canonical parser/render
tests and used to relate operators, Select, String Format, and Set Field to one Incident example.

## Chapter 10 — Branches, Loops, Parallelism, and Return

**Summary:** Teach control flow as visible topology: exact execution arms, loop ownership,
bounded concurrency, explicit stopping conditions, and result boundaries that do not pretend
to offer function-wide stack unwinding before the runtime can support it.

### 10.1 `if`, `else if`, and `else`

Build simple Boolean branches and then reveal how execution-pin labels disambiguate more
specialized branch nodes.

### 10.2 Named execution arms

Use explicit blocks for nodes with several success, error, or outcome pins so no path is
hidden behind exception-like magic.

### 10.3 `for … of`

Cover value and zero-based index bindings, one-time array evaluation, input-order sequential
semantics, Done as the join point, and the current rule that the loop logs a failed body path
and continues with later items.

### 10.4 `@parallel for`

Make concurrency an opt-in design decision. Explain the current default of 30, explicit-call
form for custom limits, Done as a barrier rather than an ordering guarantee, continued sibling
work after failure, and the operational questions authors must answer before parallelizing
external side effects.

### 10.5 `while`

Teach condition reevaluation, the default maximum of 15, explicit custom maximums, progress,
cooperative cancellation, and why hitting the current silent ceiling must not be confused with
domain success.

### 10.6 Structural stopping and skipping

Record that FlowScript has no `break` or `continue` statements today. Model continue-like skips
with visible branches. Explain the separate Boolean-controlled For Each (Break) catalog node and
why it is not currently ordinary loop sugar.

### 10.7 `return`

Connect function return expressions positionally to output pins, and Event returns to one
terminal result node. Teach one final unconditional Function return and one logical Event result
path: function-wide early termination is not implemented, Event returns terminate only their
current branch, and first/last result aggregation differs across execution surfaces.

### 10.8 Read the complete control-flow example

Walk from source to graph through a Boolean branch, exact HTTP execution arms, sequential and
parallel iteration, bounded While, a Function output, and an Event result.

**Evidence:** SRC-FLOWSCRIPT, SRC-CONTROL-FLOW, SRC-RECONCILER, and SRC-EXECUTION.
**Interview dependency:** Resolved in INT-04 for current loop ownership, bounded parallelism,
maximum iteration handling, branch-scoped Event results, and sequential-by-default production
guidance. Aggregate child-failure status and whole-function early return remain implementation
gaps rather than interview questions.
**Worked material:** `examples/control-flow/control.flow`, kept in canonical parser/render tests;
add catalog-aware reconciliation, execution assertions, and a paired Board capture before final.

## Chapter 11 — State, Configuration, Runtime Values, and Secrets

**Summary:** Separate ephemeral computation, run-local Flow variables, shared App defaults,
client-local runtime configuration, secret values, files, caches, and durable records so readers
do not use one mechanism for every kind of state.

### 11.1 Local bindings and Flow variables

Explain what remains a source-local value, what becomes visible board state, and how reads
and writes appear in the graph.

### 11.2 `const` and `let` in FlowScript

Teach the deliberate scope split. At the top level, `const` keeps a Board variable out of
normal App configuration and `let` exposes it; both remain mutable within one run. Inside a
function, `const` binds a node output while `let` can act as a mutable local accumulator.
Neither a previous run's mutation nor JavaScript's top-level immutability model carries over.

### 11.3 Variable decorators

Cover `@category`, `@description`, `@readonly`, `@runtime`, and `@secret` as author-visible
metadata with platform consequences. Clarify that `@readonly` is user-editable metadata, not
a runtime write guard today, and that interfaces are preferred over the legacy `@schema` form.
Record true runtime immutability as the intended contract rather than silently teaching the gap.

### 11.4 Runtime configuration

Show how values vary without rewriting the Flow. Distinguish the intended per-user-and-device
contract from today's IndexedDB key, which is App plus variable inside a local client profile.
Teach the interactive saved-value preflight, its presence-only check, and the missing universal
fail-closed check in core/direct execution paths.

### 11.5 Secrets never become an authoring channel

Explain redacted rendering, empty placeholders, trusted configuration, and the guarded
declassification/type-change rules. Use local OpenAI BYOK for `@secret @runtime`, then distinguish
it from hosted provider profiles and trusted server-side Event secrets. Never infer encryption or
universal downstream redaction merely from masking and source omission.

### 11.6 Choose the right lifetime

Compare local bindings, Flow variables, runtime values, App Storage, Data Studio records,
cache, and chat/session state. Choose by the consequence of losing the value: cache only
rebuildable/miss-tolerant state, use files for bulk static material, and use a database for
queryable or correctness-bearing facts.

### 11.7 Read the complete configuration example

Walk through exposed URL, retry, and language defaults; a non-editable provider name; and a
value-free local runtime OpenAI secret. Explain why last-sync and reference data are deliberately
absent from the variable declarations.

**Evidence:** SRC-VARIABLES, SRC-RUNTIME-CONFIG, SRC-REDACTION, SRC-DATA, and
SRC-STATE-LIFETIMES.
**Interview dependency:** Resolved in the INT-03 Chapter 11 follow-up for ownership, intended
runtime scope, preflight, runtime immutability, lifetime selection, and the OpenAI BYOK example.
INT-05 still supplies the broader WASM and hostile-package threat model for Chapters 21–22.
**Worked material:** `examples/state-and-secrets/state.flow`, kept in canonical parser/render
tests; add catalog-aware reconciliation and separate local/remote credential-path tests before
final publication.

## Chapter 12 — Functions, Layers, Handlers, and Caching

**Summary:** Turn repeated node networks into reusable typed contracts without losing the visible
execution, Event, and cache boundaries that let humans, the runtime, and agent tooling understand
what a call actually does.

### 12.1 Functions become layers

Map parameters and returns to Function-layer boundary pins and follow a Call Function node through
the graph. Use the founder's extraction rule: when a layer is copied because its logic must remain
the same, make it a function. Caching is the second reason to establish a Function boundary.

### 12.2 Pure and impure functions

Explain the two directions of evaluation: impure execution moves forward while required pure data
is pulled backward from input pins. Purity is a node-designer promise expressed by Execution-pin
shape, not a theorem proved by the engine. Recommend cheap deterministic work as pure and expensive
or potentially side-effecting work as impure. Compare this with Unreal Blueprint's official
state-mutation definition and demand evaluation without claiming identical reevaluation behavior.

### 12.3 Functions as methods

Show that every user function with a first typed parameter can be called as a method on that value.
The receiver fills the first input without changing the Function layer or implying mutation. Note
that catalog-node receivers remain metadata-driven and Board-to-source projection may normalize a
method spelling back to a flat call.

### 12.4 Nested handlers and agent tools

Distinguish a reusable Function layer from a triggerable Event entry. Agent tool registration must
explicitly reference an eligible Event handler; a plain function currently requires an
author-written thin handler shim. Explain inferred argument/result shapes, nested handler scope,
and the lack of automatic per-tool confirmation or authorization metadata.

### 12.5 `@cache`

Teach `@cache` as an explicit authored promise that a prior output may replace the whole call,
including side effects. Cover namespace, TTL, App/user scope, the current cache key, cache/backend
failure behavior, concurrent misses, permanent entries, and the present mismatch between visual
and FlowScript defaults. Cache only when declared inputs express every meaningful dependency.

### 12.6 Invalidate after the source of truth changes

Invalidate the exact namespace and scope after a durable update succeeds, not after every read.
Use TTL, namespace versions, and an explicit data revision input. Explain the in-flight-miss race
and why a revision key prevents a late old result from serving new callers.

### 12.7 Layers for organization

Return to presentational layers and show when visual compression is useful without turning
every named section into a callable abstraction. Contrast “one noisy section” with “copies intended
to remain identical,” and show the Studio layer-to-Function conversion boundary.

### 12.8 Read the complete function and cache example

Walk through a pull-evaluated normalizer, a deterministic but structurally execution-driven cached
resolver, direct and method calls, a registered agent Event shim, an explicit data-revision input,
and a separate namespace-invalidation Event placed after authoritative data update.

**Evidence:** SRC-LANGUAGE-AST, SRC-RECONCILER, SRC-LAYERS, SRC-FUNCTION-EXECUTION,
SRC-FUNCTION-CACHE, SRC-AGENT-HANDLERS, and SRC-UNREAL-BLUEPRINTS.
**Interview dependency:** Resolved in the INT-03 Chapter 12 follow-up for extraction, purity,
methods, Event shims, explicit cache ownership, and author-controlled invalidation. INT-08 can add a
production agent-tool case study but no longer blocks the language chapter.
**Worked material:** `examples/functions-and-caching/functions.flow`, kept in canonical
parser/render tests; add catalog-aware reconciliation and cache miss/hit/expiry/invalidation
execution coverage before final publication.

## Chapter 13 — Events, Interfaces, and Complete Apps

**Summary:** Move from a callable Flow to a complete App. Readers keep one typed incident decision
behind separate Page, Quick Action/Cron, and REST adapters; configure identity, credentials,
execution location, and immutable releases; and inspect both current guarantees and boundary gaps.

### 13.1 An event node is an entry; an App Event is an exposure

Separate the triggerable Board node from the App-level binding that selects a compatible surface,
version, variable overrides, route, exposure, and execution location.

### 13.2 Keep one decision and write honest adapters

Reuse one typed Function while adapting Page, Quick Action/Cron, and REST callers through explicit
Event entries. Explain why REST registers a handler entry rather than a Function layer.

### 13.3 A Page should be experienced, not merely described

Embed the interactive React Incident Desk prototype, then map its fields and button to the A2UI
component, element-state, workflow-event, Page-target, and route contracts used in the App.

### 13.4 Quick Action and Cron share a shape, not an identity

Show exposed variables, Event overrides, the Quick Action form, and an unattended schedule over the
same Simple Event without confusing interactive and scheduled credentials.

### 13.5 A REST App Event is a setup program

Compose server configuration, one registered Generic handler, API-key auth, OpenAPI routes, and REST
Server inside a Simple setup Event. Explain save-time remote setup and last-successful fallback.

### 13.6 The HTTP boundary is structured—and still evolving

Call the route, receive the typed incident result, inspect reserved request/client inputs, and name
the current remote OpenAPI, schema-enforcement, and rejected-run gaps.

### 13.7 Identity, permission, and credentials are different boundaries

Distinguish App roles, REST registration auth, connected-App identity, in-Flow authorization, and
the credentials used for downstream work.

### 13.8 Hybrid is a Board choice, not an Event location

Explain Local, Remote, and Hybrid Boards; Local/Remote-only App Events; Studio versus web dispatch;
and why a single run never splits between machines.

### 13.9 Pin the contract before other people depend on it

Use Latest for authoring, pin production to a numbered Flow version, verify successful REST setup,
retain rollback, and label stored-but-unwired canary selection honestly.

### 13.10 The complete App checklist

Close with entry, contract, interface, identity, credentials, location, release, and evidence as the
minimum handoff questions for a complete App.

**Evidence:** SRC-APP-MODEL, SRC-EVENTS, SRC-IDENTITY-PERMISSIONS, SRC-APP-SURFACES,
SRC-API-SURFACES, and SRC-VERSIONING.
**Interview dependency:** Resolved for Chapter 13 in the INT-03 Event/interface follow-up and code
audit. INT-07 still supplies the wider production-deployment claims for Chapters 3 and 25.
**Worked material:** `examples/events-and-interfaces/events.flow`, kept in canonical parser/render
tests; embedded `IncidentDeskDemo.tsx`; add end-to-end remote setup/inbound and rejected-run coverage
before upgrading the documented gaps to guarantees.

---

# Part III — The Two-Way Contract

Part III explains the mechanism behind FlowScript's defining promise. Readers learn why
round trips are difficult, what identity means, how changes are reviewed, and how evidence
connects a running system back to its building blocks.

## Chapter 14 — Board ⇄ AST ⇄ Text

**Summary:** Open the machinery without requiring readers to be compiler engineers. A Board
is lowered into a typed semantic pivot and rendered; edited text is parsed, validated, and
reconciled into a minimal atomic batch of Board commands.

### 14.1 One program, three representations

Introduce Board, `BoardAst`, and canonical FlowScript text and state which information each
representation owns. Establish equal authoring surfaces over one persisted Flow model.

### 14.2 Why Board JSON is not the language

Let readers copy several nodes into a text editor, inspect the transport-oriented JSON, and see
why an editable language needs a semantic pivot rather than direct storage manipulation.

### 14.3 Board to AST

Show how nodes, layers, variables, functions, events, schemas, and execution arms become
language constructs; explain pure-expression recovery and semantically transparent reroutes.

### 14.4 AST to text

Explain canonical rendering, derived imports, qualified calls at collisions, safe sugar, stable
naming, standardized author style, and secret omission.

### 14.5 Text to AST

Tour the context-free lexer, recursive-descent structure parser, Pratt expression parser,
bounded nesting, and the syntax-versus-semantics diagnostic boundary in one expert aside.

### 14.6 AST to commands

Explain catalog-aware resolution, type and shape checks, dynamic pins, correction proposals,
minimal edit planning, preview, rollback, persistence, and canonical reload.

### 14.7 Follow one Incident Desk change through the loop

Show one anchored literal becoming exactly one `UpdateNodePin`, then derive a local
`customerFacing` signal as a structural preview without changing an established Function signature.

### 14.8 What the text deliberately does not own

Discuss coordinates, colors, comments, reroutes, and other graph metadata that minimal
reconciliation can preserve even when it is not fully encoded in source.

### 14.9 The mental model to keep

Close on lower, render, parse, reconcile, apply, and reload as the six verbs behind the contract.

**Evidence:** SRC-LANGUAGE-AST, SRC-PARSER-RENDERER, SRC-GRAPH-MODEL, SRC-RECONCILER,
SRC-APPLY, and SRC-EDITOR.
**Interview dependency:** Resolved for Chapter 14 in INT-03. The founder supplied the later
crystallization, minimal-command, canonical-style, namespace, secret, validation, and expert-depth
decisions; the precise historical rationale is labelled as an inference from code and repository
history.
**Worked material:** `examples/board-ast-text/canonical.flow`, kept in canonical parser/render
tests; the exact literal-edit claim is backed by the core anchored-reconciliation regression test.

## Chapter 15 — Identity, Anchors, and Safe Change

**Summary:** Explain the safety model that lets a text edit target existing graph entities
without guessing. Stable anchors preserve identity; previews and deletion guards make
destructive intent explicit.

### 15.1 Identity is not the same as spelling

Show why renaming a local variable or call display must not accidentally replace the wrong
node, function layer, or event entry. Show an anchored Function rename producing one
`RenameLayer` under the stable layer ID. Keep Function signature migration outside the current
guarantee.

### 15.2 `//@n`, `//@v`, and `//@l`

Read node, variable, and layer anchors and explain why they appear as trailing comments in
editable source.

### 15.3 Reconciliation, not wholesale regeneration

Show how minimal commands preserve layout and metadata better than deleting and rebuilding
the graph.

### 15.4 Corrections and diagnostics

Distinguish safe canonical corrections from ambiguous changes that fail closed. Explain that one
anchor assigned to two distinct entities fails during raw-source preflight, before Board lookup.
The Event header and immediate first arm-routing Branch may repeat an anchor because they represent
one entity. Ordinary node, variable, Function, and module anchor IDs absent from the current Board
are then reported as unavailable and reconciled as unanchored entities. Absent Event anchors retain
their specialized recovery path. It re-anchors one compatible entry or creates a fresh entry when
zero or several candidates remain. Invalid Event metadata, incompatible live anchors, and ambiguous
ordinary recovery still stop the plan.

### 15.5 Deletions require explicit intent

Remove one node anchor from an otherwise unchanged Incident Desk call. Walk through the proposed
replacement and removal, the blocked Apply, the destructive command preview, and the separate
approval to proceed.

### 15.6 One atomic, undoable change

Explain validation before persistence and why a complete edit lands as one reviewable undo
unit.

### 15.7 Scoped editing for large Flows

Show how selected event/function sections and their transitive dependencies can be edited
without treating the unseen remainder as deleted.

**Evidence:** SRC-RECONCILER, SRC-APPLY, and SRC-EDITOR.
**Interview dependency:** Resolved in the Chapter 15 follow-up to INT-04. No historical failure
was supplied, so the chapter uses a clearly labeled safety drill and treats AI as one editor among
people and tools.

## Chapter 16 — The Dual-View Editor and Language Tools

**Summary:** Teach FlowScript as an intentional development environment: Monaco editing,
structural and authoritative diagnostics, generated declarations, bidirectional navigation,
formatting, previews, and optional VS Code support.

### 16.1 A language service in the Studio

Introduce highlighting, completion, hover, signatures, definition lookup, diagnostics, and
canonical formatting.

### 16.2 Fast feedback and authoritative checks

Separate the browser's responsive structural analysis from the Rust parser/reconciler that
previews the actual board command plan.

### 16.3 Navigate from code to graph

Use anchors to reveal and highlight the responsible node or layer from the cursor.

### 16.4 Navigate from graph to code

Select a node, open the relevant source section, and explain large-Flow scoped views.

### 16.5 Preview before apply

Read additions, updates, removals, diagnostics, and stale-board warnings before changing the
persisted Flow.

### 16.6 `.flow.d` and VS Code

Show how generated declarations and the extension support source outside the in-app panel
without promising a standalone FlowScript runtime.

**Evidence:** SRC-EDITOR, SRC-DECLARATIONS, and SRC-VSCODE.
**Interview dependency:** INT-03 should describe the desired end state for external tooling
and language-server support.

## Chapter 17 — Runs, Logs, Traces, and Versions

**Summary:** Return to the founding problem and show the operational loop: execute a named
entry, inspect the run, follow node-attributed logs to the responsible blocks, reuse a
recorded invocation payload where available, repair the Flow, and release a verified version.

### 17.1 A run is evidence

Define run identity, input availability, total timing, terminal execution status, highest log
severity, attributed logs, and available Board-version metadata. State where local and remote
records differ, and do not imply that every input, output, or resolved version is archived.
Treat terminal status and highest severity as separate signals: a recovery path can preserve
Error evidence without making the Core run fail.

### 17.2 The log as a flight recorder

Use per-node messages, severity, timing, optional usage statistics, and deliberately emitted
diagnostics to reconstruct what happened. Treat logs as a governed data boundary rather than
assuming arbitrary author messages are automatically redacted.

### 17.3 Trace the failing building block

Navigate from a run or log record back to the node and corresponding FlowScript statement.

### 17.4 Rerun representative inputs

Define Re-Run as applying a recorded invocation payload to today's Flow: a regression tool, not
a time machine. Contrast it with deliberately invoking an immutable Board version, then name the
surrounding state that both approaches resolve under separate lifecycles—packages, runtime
values, profiles, credentials, stored data, external responses, and model behavior. Explain
why indiscriminate snapshots are operationally infeasible and still cannot reproduce every
external effect.

### 17.5 Design reliable failure paths

Cover validation, explicit result arms, timeouts, bounded retry, idempotency, redaction, and
correlation identifiers. Separate modeled negative outcomes from unexpected node errors and
show how an On Error recovery path preserves the failed node's evidence.

### 17.6 Draft, immutable version, and rollback

Test on latest, create a version, pin the Event, retain the previous target, and roll forward
with a patch after a correction.

### 17.7 Product telemetry is not execution evidence

Briefly distinguish user-visible run logs, infrastructure monitoring, anonymous product
telemetry, and configurable audit records.

**Evidence:** SRC-OBSERVABILITY, SRC-RERUN, SRC-VERSIONING, and SRC-EXECUTION.
**Interview dependency:** INT-04 is the primary interview for this chapter. Revisit the original
3 a.m. incident as a counterfactual unless an approved, attributable repair narrative becomes
available before publication.

---

# Part IV — Building a Real Application

Part IV assembles the language and platform into a private document assistant called
**Incident Room**. It begins as a forkable local application and adds complexity in visible,
testable increments.

## Chapter 18 — Capstone: Build Incident Room

**Summary:** Build an application that accepts runbooks and incident documents, makes them
searchable, and lets a user ask grounded questions through an agentic chat interface.

### 18.1 Start from the user journey

Define upload, indexing, readiness, question, cited answer, failure, and deletion journeys
before selecting nodes.

### 18.2 Draw the application boundary

Choose the App, Flows, event entries, chat/page surfaces, storage, tables, and runtime
variables that belong together.

### 18.3 Ingest a document

Validate the file, extract appropriate content and metadata, split it, create embeddings,
store searchable records, and report progress.

### 18.4 Define narrow agent tools

Expose search, read, list, and other approved operations as typed handlers rather than giving
the model an unrestricted code or storage surface.

### 18.5 Answer with evidence

Retrieve relevant passages, build bounded context, invoke a configured model, return sources,
and handle insufficient evidence honestly.

### 18.6 Keep it local when required

Configure local models and offline storage, then explain what changes when the App moves to
an online or remote execution path.

### 18.7 Inspect the complete Flow in two views

Use the existing large fixture to teach navigation, functions, handlers, scoped editing, and
why the text view becomes especially valuable as a project grows.

**Worked source:** `tests/ast/ttwctnp08u18sg2z6nmcqqak.flow` and its Board/anchored
companions, distilled into staged book fixtures rather than printed as one wall of code.
**Evidence:** SRC-CAPSTONE-DOCS, SRC-RAG, and SRC-CHAT.
**Interview dependency:** INT-08 must recover the intended user story, data contracts, model
choices, and which current fixture details are incidental.

## Chapter 19 — Files, Tables, SQL, and Data Studio

**Summary:** Generalize the capstone's data choices into a durable model for application
state. Readers learn when to use App Storage, databases, SQL, vector search, ontologies, and
governed actions.

### 19.1 Files are not arbitrary host paths

Introduce `FlowPath`, scoped stores, uploaded artifacts, user/app ownership, and portability
between local and hosted execution.

### 19.2 Tables and durable records

Design stable schemas, deterministic keys, batch writes, upserts, checkpoints, and safe
retries.

### 19.3 Query with SQL

Use DataFusion for set-based filtering, joins, aggregation, and large transformations that
would become unreadable per-record loops.

### 19.4 Search text and meaning

Compare full-text, vector, and hybrid retrieval and make model/embedding cost and freshness
observable.

### 19.5 Ontologies and governed actions

Model domain objects and relationships, then expose approved actions without moving the
underlying data into a disconnected system.

### 19.6 Data Studio as a shared surface

Show how domain experts, builders, workflows, and the Data Studio agent inspect and act on
the same app-owned data.

**Evidence:** SRC-DATA, SRC-DATA-STUDIO, and SRC-DATA-PIPELINES.
**Interview dependency:** INT-08 must identify the stable hands-on subset and provide real
scale/benchmark evidence before the prose makes scale claims.

## Chapter 20 — FlowPilot and AI-Authored Software

**Summary:** Show AI as another author operating through the same typed declarations,
reconciliation, review, execution, and logs—not as a privileged bypass around the platform.

### 20.1 Prototypes became cheap; reliable operation did not

Frame the new gap: generating source is easy, but understanding, scaling, governing, and
repairing the result remains hard.

### 20.2 Deterministic before probabilistic

Apply the opening doctrine inside real applications: ordinary code and nodes handle exact
work; model calls begin only where language, ambiguity, or judgment makes uncertainty useful.

### 20.3 Author and runtime operation are different roles

Separate AI proposing an application change from a model making a probabilistic decision
inside a running Flow. Give each role its appropriate validation, evidence, permissions, and
failure boundaries.

### 20.4 Spend the smallest adequate intelligence

Design narrow calls, compare task quality, latency, and cost, and use the smallest model that
meets the measured contract. Show current App/user/model attribution, limits, profiles, and
weighted routing without presenting automatic task evaluation as complete.

### 20.5 Constrain the solution space productively

Explain how known building blocks, types, permissions, and platform services reduce invalid
and catastrophic outcomes without claiming that generated software is automatically correct.

### 20.6 Declarations before invention

Show how an agent searches exact live node signatures instead of hallucinating calls and
arguments.

### 20.7 Draft, check, preview, commit

Follow retained source through diagnostics, repairs, command preview, explicit deletion
intent, and an atomic application.

### 20.8 Execute and inspect in a later loop

Separate structural success from runtime proof; run the persisted entry, inspect logs, and
feed concrete failure evidence into a focused repair.

### 20.9 Parametrize and fork instead of rebuilding

Develop the idea that many applications are variations of an existing shape and that a
clonable, configurable App gives an AI a safer and faster starting point than a blank project.

### 20.10 Humans can still read the result

Return to the promise: a teammate outside the generation session can inspect the code, graph,
packages, permissions, data, version, and evidence.

**Evidence:** SRC-AI-AUTHORING, SRC-FLOWPILOT, SRC-FLOWSCRIPT,
SRC-AI-DETERMINISTIC-FIRST, SRC-AI-USAGE-CONTROLS, SRC-AI-MODEL-SELECTION, and
SRC-AI-MODEL-INVENTORY.
**Interview dependency:** INT-08 is the primary source; it needs one successful and one failed
AI-generated App story.

---

# Part V — Extension, Capability, and Trust

Part V serves experienced developers and reviewers. It explains how Flow-Like remains
extensible without adopting arbitrary inline code as the primary escape hatch.

## Chapter 21 — Build a Custom WASM Node

**Summary:** Add one missing capability as a typed, reusable WebAssembly node. Rust is the
teaching implementation; the same host contract is available through other maintained SDKs
with different maturity and runtime models.

### 21.1 When a package is the right boundary

Choose a custom node for a stable, reusable capability—not merely to avoid learning the
catalog or to hide an entire application in one opaque block.

### 21.2 Start from the Rust template

Tour `flow-like.toml`, the node definition, typed pins, schemas, scores, permissions, `run`,
tests, and the compiled artifact.

### 21.3 Pure or impure

Classify side effects honestly and add execution pins only when the contract requires them.

### 21.4 Define the pin contract

Choose names, types, shapes, defaults, errors, documentation, and versioning so the node is
useful from both the graph and FlowScript.

### 21.5 Implement and test

Read inputs, perform bounded work, write outputs, exercise denial and failure paths, and keep
logs free of secrets.

### 21.6 See the node become FlowScript

Install the package, generate its declaration/name metadata, call it as source, and locate it
on the canvas.

### 21.7 Other language SDKs

Explain Component Model versus core-module templates and route readers to the current
capability matrix rather than claiming equal parity forever.

**Worked source:** `templates/wasm-node-rust` for the first node and
`examples/sales-insights` for a complete package with nodes and widgets.
**Evidence:** SRC-WASM-SDK and SRC-WASM-EXAMPLES.
**Interview dependency:** INT-05 should provide the philosophy behind the node/package
boundary and why inline code blocks are prohibited by design.

## Chapter 22 — The WASM Sandbox: Capabilities, Limits, and Consent

**Summary:** Explain the security model precisely. WASM supplies memory isolation and a
controlled host boundary; declared capabilities, runtime limits, consent, deployment egress,
and package review address different risks.

### 22.1 Isolation is a boundary, not a magic adjective

State what a guest cannot directly address and distinguish that from trust in its behavior,
outputs, cost, or permitted network calls.

### 22.2 Capability-scoped host functions

Cover network, storage, variables, cache, streaming, models, A2UI, OAuth, and function-call
permissions and what a node can still do without them.

### 22.3 Fuel, deadlines, and resource tiers

Explain current CPU/time controls and manifest tiers. Claim memory/table enforcement only
after its runtime wiring is verified.

### 22.4 Storage and network boundaries

Use scoped paths and host functions; add infrastructure egress policy when destination
control matters.

### 22.5 Interactive consent

Show package/permission prompts, run-once or remembered trust, their local storage scope, and
the fact that consent does not grant a capability the node did not declare.

### 22.6 Scheduled and API execution

Describe non-interactive policy only after INT-05 resolves the intended enforcement model;
do not extrapolate the current interactive prompt into a universal approval gate.

### 22.7 Honest caveats

Document component-runtime environment inheritance, version-keyed trust behavior, permitted
expensive operations, and any fallback path outside ordinary store controls.

**Evidence:** SRC-WASM-RUNTIME, SRC-WASM-CONSENT, and SRC-WASM-DOCS.
**Interview dependency:** INT-05 plus targeted implementation verification are mandatory
before drafting security guarantees.

## Chapter 23 — Packages, Governance, Versions, and Release

**Summary:** Treat reuse as a controlled supply chain. A package has identity, immutable
versions, access rules, extracted node contracts, compilation artifacts, review states, and
an installation relationship with an App.

### 23.1 Package identity and immutable versions

Use stable reverse-domain IDs, semantic versioning, hashes, manifests, and versioned
artifacts.

### 23.2 Private packages

Explain team/member access and why private packages can become usable without a public
platform review.

### 23.3 Public and request-access packages

Follow publication review and then the additional owner-controlled access step for a
request-access package.

### 23.4 Install, update, pin, and remove

Manage a package as an explicit project dependency and review permissions and changelog
before updating.

### 23.5 Quality and risk signals

Introduce privacy, security, performance, governance, reliability, and cost scores only
after the repository's conflicting score direction is resolved.

### 23.6 App visibility and publication

Compare offline, private, prototype, public, and public-request-access Apps, member limits,
review transitions, and forking.

### 23.7 Governance that arrives with authorship

Show how known nodes, permissions, versions, logs, data, roles, and publication history make
useful documentation and review evidence available with less manual bookkeeping.

**Evidence:** SRC-PACKAGES and SRC-GOVERNANCE.
**Interview dependency:** INT-06 must resolve score semantics and distinguish current
automated governance from the desired end state.

---

# Part VI — Runtime, Deployment, and Contribution

Part VI explains how a visible Flow becomes efficient running software, how the same contract
fits different deployment shapes, and how contributors can evolve the system without
breaking its central invariant.

## Chapter 24 — From Flow to Compiled Artifact to Rust Runtime

**Summary:** Follow a Flow after authoring. The graph is prepared, compiled into a compact
execution-scoped artifact, bound to a compatible node-registry fingerprint, converted into
an immutable run template, and executed with isolated per-run state.

### 24.1 FlowScript does not execute directly

Restate the pipeline: source reconciles to a Board; the runtime executes the Flow model.

### 24.2 Prepare the graph

Apply node updates, mint dynamic pins, synchronize schemas, remove editor-only reroutes where
safe, and validate topology.

### 24.3 The compiled-board artifact

Explain the versioned `FLCB` envelope, registry fingerprint, compressed `rkyv` payload, stale
artifact rejection, and why this is not a universal native executable.

### 24.4 Immutable template, per-run context

Show which structures can be cached across runs and which variables, credentials, traces,
callbacks, and cancellation state belong to one execution.

### 24.5 Pure dependencies and execution traversal

Connect the conceptual model from Chapter 5 to the active Rust engine without describing
inactive experimental engines as current behavior.

### 24.6 Logs, streaming, cancellation, and callbacks

Follow observable run state through desktop and remote execution surfaces.

### 24.7 Performance claims need workloads

Teach readers how to interpret the repository's narrow benchmarks and require workload,
hardware, version, and measurement method for broader claims.

**Evidence:** SRC-COMPILED-FLOW, SRC-EXECUTION, and SRC-BENCHMARKS.
**Interview dependency:** INT-04 must provide intended invariants, representative performance
measurements, and the reasoning behind the compiled artifact.

## Chapter 25 — Deployment Without Rebuilding the Application

**Summary:** Show how deployment targets host the same executor and Flow artifact contract
while differing in identity, storage, queues, isolation, scaling, cost, and operational
maturity.

### 25.1 Local and offline

Run device-bound or private work locally and explain which collaboration and remote-trigger
features are intentionally absent.

### 25.2 Docker Compose

Present the development/small self-hosting shape, its services, persistence, monitoring, and
limits without calling it the universal production answer.

### 25.3 Kubernetes

Explain the API, executor pool, compiler, sink services, Helm configuration, storage options,
network policy, and the current job-once execution gap.

### 25.4 Cloud-specific targets

Describe only the production-supported subset of AWS, Azure, GCP, Cloudflare, StackIT, or
other targets after INT-07 classifies code present in the repository, private deployments,
and roadmap work.

### 25.5 Execution backends

Compare HTTP pools, queues, Lambda modes, and externally supplied runners; emphasize that a
configured dispatcher is not proof of an end-to-end worker or retry policy.

### 25.6 Storage and secrets

Map metadata, content, logs, and credentials to provider-specific implementations while
keeping the logical App contract stable.

### 25.7 One platform environment, many Apps

Present the “few governed environments rather than one cloud room per App” operating model as
an architectural thesis and case-study target until measured enterprise evidence supports
specific scale and cost numbers.

### 25.8 Release, observe, and roll back

Tie version-pinned Events, canary strategy where implemented, metrics, traces, alerts, and
rollback into one operational checklist.

**Evidence:** SRC-DEPLOYMENT and SRC-OPERATIONS.
**Interview dependency:** INT-07 is mandatory. It must cover supported targets, StackIT,
Cloudflare scope, comparative costs, environment counts, and one real deployment profile.

## Chapter 26 — Internals: Protecting the Round Trip

**Summary:** Give contributors a map of the language, graph, catalog, executor, clients, and
tests, then show how to add behavior without breaking the two-view contract.

### 26.1 Repository map

Locate the AST crate, core graph model, catalog, executor, compiler/WASM runtime, API,
clients, docs, SDKs, and examples.

### 26.2 Evolve the grammar

Change lexer, parser, typed AST, renderer, diagnostics, editor analysis, and tests as one
coherent language surface.

### 26.3 Evolve the graph projection

Update lowering and reconciliation together, preserve identity, and fail closed when a graph
contract cannot be expressed honestly.

### 26.4 Evolve a node

Version pin contracts, generated declarations, schemas, documentation, scores, purity, and
permissions together.

### 26.5 Test the invariant

Use parse/render idempotence, fixture snapshots, property tests, unchanged-board no-op tests,
reconciliation/apply tests, compiled artifact tests, and editor tests.

### 26.6 Known gaps are part of the specification

Maintain a versioned limitations ledger for unsupported destructuring/schema shapes,
round-trip edge cases, dynamic contracts, and deployment/security caveats.

### 26.7 Contribute without weakening the thesis

Evaluate a change against readability in both views, runtime honesty, permissions,
observability, portability, and the “no worse-product shortcut” rule.

**Evidence:** SRC-ARCHITECTURE, SRC-LANGUAGE-TESTS, SRC-EDITOR-TESTS, and
SRC-COMPILED-FLOW.
**Interview dependency:** INT-09 should supply contribution philosophy, stability policy,
and the roadmap for FlowScript as a language surface.

---

# Epilogue — Make the System Explain Itself

Return to the incident room. Contrast software that requires its absent author with software
whose structure, permissions, data, version, run history, and failing operation are available
to the people responsible for it. End with a challenge: the goal is not to make every reader
an infrastructure specialist; it is to let them solve their real problem without silently
creating the next 3 a.m. mystery.

The epilogue also returns to AI: generated applications become valuable when they can be
understood, governed, repaired, and reused after the generation session ends.

**Interview source:** INT-01 and the final INT-09 retrospective.

---

# Appendices

## Appendix A — FlowScript syntax reference

A compact, versioned grammar and examples for declarations, statements, expressions,
operators, precedence, imports, calls, types, and comments.

## Appendix B — Decorator reference

Exact syntax and semantics for variable and function decorators, including cache defaults
and secret/runtime restrictions.

## Appendix C — Type and schema reference

Scalar types, value shapes, interface forms, inference rules, pin compatibility, and current
schema projection limits.

## Appendix D — Generated declarations and node discovery

How to read `.flow.d`, names and schema tables, use in-app/VS Code completion, and ask
FlowPilot for exact declarations.

## Appendix E — Diagnostic and reconciliation glossary

Parse, type, resolution, executability, correction, stale-board, deletion, dynamic-pin, and
apply errors with safe repair patterns.

## Appendix F — Coming from another environment

Focused concept maps for TypeScript, Rust, Python, n8n, Unreal Blueprints, LangChain, and
traditional enterprise integration tools. These are translations, not claims of identical
semantics.

## Appendix G — Compatibility and status ledger

Flow-Like/FlowScript version policy, file/declaration compatibility, current limitations,
preview features, deployment support matrix, and the rules for updating book claims.

## Appendix H — Glossary

Canonical definitions for Flow-Like, App, Flow, Board, Studio, FlowScript, node, pin, wire,
layer, function, handler, Event, package, Bit, Data Studio, runtime, and execution backend.

## Appendix I — Exercise solutions and fixture index

Links each exercise to its FlowScript, Board, anchored source, expected diagnostics, run
inputs, and version-matched automated check.
