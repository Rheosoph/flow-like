# FlowBook source map

This file is the drafting evidence index for *FlowBook: A Developer's Guide to Flow-Like*. It records
where a chapter author should look before stating how the current product behaves. It is not
a replacement for reading the relevant source, tests, generated declarations, and release
notes at the version targeted by the book.

## Evidence policy

Use sources in this order when writing about current behavior:

1. executable tests and implementation code;
2. generated schemas and declarations from the same revision;
3. product documentation from the same revision;
4. founder and maintainer interviews for rationale and intent;
5. measurements or named case studies for performance, scale, cost, and operational claims.

Interview material is authoritative for history, motive, rejected alternatives, and product
doctrine. It is not sufficient evidence that a security boundary, deployment target, or
runtime guarantee is implemented in every execution mode.

Every drafted chapter should carry a version ledger containing the repository revision, the
Flow-Like release used by examples, the status of each changing capability, and the date on
which claims were checked.

### Capability labels

- **Current** — implemented on the named release and covered by suitable source or tests.
- **Preview** — usable, but incomplete, changing, or not yet supported across every surface.
- **Vision** — product direction or intended contract, not a claim about the current release.
- **Measured** — supported by a reproducible benchmark or a named, approved case study.

## Language and two-view authoring

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-FLOWSCRIPT` | `apps/docs/src/content/docs/studio/flowscript.md`; `apps/website/src/content/blog/2026-07-07-flowscript.mdx` | Public syntax overview, authoring workflow, and product framing | The blog is historical marketing copy. Recheck counts, feature status, and terminology against code. |
| `SRC-LANGUAGE-AST` | `packages/ast/src/lib.rs`; `packages/ast/src/model.rs`; `packages/ast/src/schema.rs`; `packages/ast/src/text.rs`; `packages/ast/src/naming.rs` | The dependency-light semantic pivot, statements, expressions, types, naming, and serializable AST | AST support does not by itself prove successful lowering to every Board shape. This is a semantic AST, not a concrete syntax tree that preserves arbitrary author formatting. |
| `SRC-PARSER-RENDERER` | `packages/ast/src/parse/`; `packages/ast/src/render.rs`; `packages/ast/tests/parse.rs`; `packages/ast/tests/render.rs`; `packages/core/src/flow/ast/mod.rs` | Hand-written lexing/parsing, canonical formatting, syntax diagnostics, and text round trips | Keep parser acceptance separate from catalog resolution, semantic validity, and executable Board validity. Parsing currently stops at the first error and reports line/column rather than full retained source spans. |
| `SRC-LANGUAGE-TESTS` | `packages/ast/tests/`; parser, renderer, lowering, and reconciliation tests colocated with their Rust modules; `tests/ast/` | Concrete accepted syntax, round-trip fixtures, and known edge cases | Some fixtures test preservation rather than runtime behavior. Ignored tests must not be presented as guarantees. |
| `SRC-TYPES` | `packages/ast/src/schema.rs`; `packages/core/src/flow/ast/types.rs`; `packages/core/src/flow/pin.rs`; `packages/core/src/flow/ast/reconcile.rs` | FlowScript type forms, container shapes, member access, and their relationship to pin schemas | Describe this as schema and pin safety unless broader compile-time semantics are verified. Enforced schemas currently require canonical equality rather than structural subtyping; complex `oneOf` and `allOf` shapes need explicit testing. |
| `SRC-STRUCT-SCHEMAS` | `packages/catalog/std/src/structs/break_struct.rs`; `packages/catalog/std/src/structs/make_from_schema.rs`; `packages/catalog/std/src/structs/fields/get_field.rs`; `packages/catalog/std/src/structs/fields/set_field.rs`; `packages/catalog/tests/break_struct_schema_propagation.rs`; `packages/ast/flow.d/structs.flow.d` | Typed Make/Break operations, dynamic schema-derived pins, and open Get/Set field access | A missing dynamic field returns `null` with `found = false`; the Get Field operation does not fail by itself. Teach authors to inspect `found` at dynamic boundaries. |
| `SRC-DECLARATIONS` | `packages/ast/flow.d/`; `packages/ast/flow.d/names.json`; `packages/core/src/flow/copilot/declarations.rs` | Version-matched imports, node call signatures, names, and AI-visible declarations | Generated catalog size changes frequently. Never bake a permanent node count into narrative prose. |
| `SRC-CATALOG-DISCOVERY` | `packages/ui/components/flow/flow-context-menu.tsx`; `packages/ui/components/flow/flow-context-menu-nodes.tsx`; `packages/ui/lib/flow-board-utils.tsx`; `packages/ui/components/flow/flowscript/flowscript-language.ts`; `packages/ui/components/flow/flowscript/flowscript-language-features.ts`; `packages/ui/components/flow/flowscript/flowscript-language.test.ts` | Context-sensitive node filtering and search on the Board; namespace, receiver-type, signature, and auto-import completion in FlowScript | Board context sensitivity can be disabled. Unknown receiver types deliberately expose broader method choices. Search ranking is textual relevance; current pickers do not rank candidates by node quality, permissions, cost, or trust. |
| `SRC-NODE-MIGRATION` | `packages/core/src/flow/board/cleanup/sync_node_schema.rs`; `packages/core/src/flow/node.rs`; `packages/catalog/tests/break_struct_schema_propagation.rs`; `packages/ui/lib/flow-board-utils.tsx`; `packages/ui/components/flow/flow-node.tsx` | Catalog-version synchronization, preserved node identity, compatible pin reuse, dynamic-pin safeguards, unavailable-package warnings, and visible node errors | Current static migration may remove deleted pins and their wires or clear connections after a type change, then clears the prior node error. The founder's preserve-and-annotate doctrine is therefore not uniformly implemented. |
| `SRC-GRAPH-MODEL` | `packages/core/src/flow/board.rs`; `packages/core/src/flow/node.rs`; `packages/core/src/flow/pin.rs`; `packages/core/src/flow/board/commands/`; `packages/ui/lib/flow-board-utils.tsx`; `packages/core/src/flow/board/commands/nodes/copy_paste.rs` | Board, node, pin, connection, layer, command, and clipboard-transport semantics | The Board contains identity, presentation, compatibility, package, and runtime metadata that is not all explicitly encoded in text. Clipboard JSON is evidence of a transport model, not a supported hand-authoring contract. |
| `SRC-RECONCILER` | `packages/core/src/flow/ast/reconcile.rs`; `packages/core/src/flow/ast/lower.rs`; `packages/core/src/flow/ast/diagnostics.rs` | Board-to-AST semantic recovery and how edited AST is resolved into minimal Board changes | Reconciliation is the heart of the two-view contract. Document preservation behavior and failure cases, not only successful generation. Existing anchored Function signatures cannot currently be migrated through text, and an ignored whole-fixture round-trip test records remaining duplicated-handler-name issues. |
| `SRC-EXPRESSIONS-SUGAR` | `packages/ast/src/parse/parser.rs`; `packages/ast/src/parse/lexer.rs`; `packages/core/src/flow/ast/lower.rs`; `packages/core/src/flow/ast/reconcile.rs`; `packages/core/src/flow/ast/template.rs`; `packages/catalog/std/src/utils/types/select.rs`; colocated operator, template, and Struct-accumulator tests | Operator families and precedence, unary/compound normalization, Select and template lowering, lossless rendering guards, and temporal Struct rebinding | Parser vocabulary is broader than catalog-backed operators. Float equality/inequality, Integer division, and Struct Get output selection have current round-trip gaps documented below. |
| `SRC-EXPLICIT-CONVERSIONS` | `packages/catalog/std/src/utils/types/try_transform.rs`; `packages/catalog/std/src/utils/string/parse.rs`; `packages/ast/flow.d/utils.flow.d`; `packages/core/src/flow/ast/apply.rs`; `packages/core/src/flow/ast/diagnostics.rs` | Typed parsing, target-shaped Try Transform behavior, conversion failure outputs, and atomic Apply on diagnostics | No current quick fix chooses or inserts an operator conversion. Try Transform returns `null` plus `success = false` on conversion failure; ignoring `success` can defer failure downstream. |
| `SRC-CONTROL-FLOW` | `packages/ast/src/model.rs`; `packages/ast/src/parse/parser.rs`; `packages/core/src/flow/ast/lower.rs`; `packages/core/src/flow/ast/reconcile.rs`; `packages/catalog/std/src/control/for_each.rs`; `packages/catalog/std/src/control/par_for_each.rs`; `packages/catalog/std/src/control/while_loop.rs`; `packages/catalog/std/src/control/for_each_with_break.rs`; `packages/catalog/data/src/events/generic_event/push_generic_result.rs`; `packages/api/src/execution/sse_proxy.rs` | Boolean and named branches, collection and While sugar, hidden defaults, loop runtime ownership, Function/Event return lowering, and result collection | Child errors are currently swallowed by loop nodes; While exhaustion is silent; `break`/`continue` syntax and function-wide early return do not exist; competing result selection varies by execution surface. |
| `SRC-APPLY` | `packages/core/src/flow/ast/apply.rs`; `packages/api/src/routes/app/board/apply_flowscript.rs`; `packages/ui/components/flow/flowscript/flowscript-apply-preview.tsx`; `packages/core/src/flow/board/commands.rs` | Previewing and atomically applying FlowScript edits through staged, rollback-backed Board commands | UI preview, API validation, command execution, rollback, and persistence are distinct stages. Atomicity is the FlowScript boundary's intent; rollback itself can report a failure and must remain observable. |
| `SRC-EDITOR` | `packages/ui/components/flow/flowscript/flowscript-panel.tsx`; `packages/ui/components/flow/flowscript/flowscript-language.ts`; `packages/ui/components/flow/flowscript/flowscript-language-features.ts`; `packages/ui/lib/flowscript-persistence.ts`; `packages/ui/state/backend-state/board-state.ts`; `packages/api/src/routes/app/board/get_flowscript.rs`; `apps/desktop/src-tauri/src/functions/flow/board.rs` | Current draft/preview/apply behavior, clean-source refresh and dirty-buffer merge, language assistance, anchored source retrieval, navigation, and persistence | Authoritative pre-Apply semantic preview is currently Desktop-only; Web reconciles authoritatively during server Apply. Server rendering filters sensitive pins; direct Desktop lowering does not currently share that filter. This surface is actively changing, so pin instructions to one release. |
| `SRC-EDITOR-TESTS` | `packages/ui/components/flow/flowscript/*.test.ts`; `packages/ui/lib/flowscript-persistence.test.ts`; `packages/ui/lib/flowscript-apply-failure.test.ts` | Editor language, anchor, preview, persistence, and failure-path behavior | A UI unit test is not proof of cross-client parity. Confirm web, desktop, and mobile support separately. |
| `SRC-VSCODE` | `apps/extension/src/`; `apps/extension/syntaxes/`; `apps/extension/src/test/`; `apps/extension/README.md` | Existing VS Code extension, syntax grammars, declarations, providers, diagnostics, and tests | Pin installation instructions and the supported feature set to a released extension version; a checked-in VSIX alone does not establish distribution or compatibility policy. |

### Precise two-view doctrine

The defensible formulation for the first edition is:

> The canvas editor and FlowScript editor inside Studio are equal authoring views over one Flow model.

The current implementation persists a Board. FlowScript is parsed and reconciled into Board
commands; the Rust executor runs the resulting graph. Do not describe the text and graph as
two independently persisted programs, and do not describe FlowScript as a separate runtime.

### Code-verified Board ⇄ AST ⇄ text notes

- `BoardAst` is the typed, “lossless-as-possible” semantic pivot between a graph optimized for
  identity/execution and text optimized for reasoning. The language crate deliberately does not
  depend on the Board implementation; graph lowering and reconciliation live in core.
- Board lowering first makes catalog calls fully qualified, then derives imports only for repeated
  static calls whose opened names cannot collide. Ambiguous names remain inline as qualified calls.
- Rendering is canonical rather than style-preserving. Pure format is
  `render(parse(authored))`; the stronger graph-derived normalization happens when a successful
  Apply reloads source from the resulting Board.
- The current parser is a context-free lexer plus recursive descent for document structure and a
  Pratt expression parser. A shared nesting budget of 128 prevents pathological editor/API input
  from overflowing the process stack.
- Reconciliation compares the edited semantic model with the live Board and emits the existing
  command vocabulary. A tested anchored literal edit yields exactly one `UpdateNodePin`; untouched
  coordinates and side metadata survive because the Board is patched rather than rebuilt.
- Any authoritative diagnostic returns zero executed commands. Destructive edits require separate
  approval; valid commands are staged and rollback-backed before persistence.
- Secret-variable defaults never enter the AST and ordinary edits preserve the hidden value. The
  server also filters sensitive pins before rendering. Direct Desktop lowering does not currently
  run that same pin filter, so secret-variable omission is a stronger cross-path guarantee than
  universal sensitive-pin redaction.
- Board reroutes lower transparently and decorative Board comments are not source comments.
  Existing anchored Function signatures are currently immutable through reconciliation. A known
  ignored whole-fixture round-trip test also covers duplicated/cross-handler name collapse.
- Anchored variables, Events, and modules have verified in-place rename paths. An anchored
  Function can move between modules under the same layer ID, but a true Function name change is
  currently rejected even though in-place rename is the intended identity rule. The Board's
  Function editor can rename that layer through its own command path.
- Selection-scoped editing is user-facing in Web and Desktop. The render expands selected nodes to
  their owning sections and transitively referenced Functions, retains variables and interfaces,
  and returns section anchors that limit the deletion diff on Apply.
- FlowScript reconciliation does not currently emit a complete Function or module layer-removal
  plan when its declaration is omitted. Do not teach declaration omission as boundary deletion.
  Use a text-visible node removal for deletion-guard examples.

Pre-publication verification should exercise array destructuring, complex union/intersection
schemas, explicit multi-execution outputs, large layers, reroute presentation, layout
preservation, path schemas, and numeric comparison lowering. These are test targets, not a
founder-supplied list of known parity gaps. Any unsupported round-trip found while producing
the examples should be reported and documented against the affected release.

### Code-verified document-anatomy notes

- The AST separates `use` declarations, interfaces, top-level variables, functions, and Event
  entries. The renderer always emits sections in that order and normalizes spacing, quotes, and
  optional separators.
- `use` changes name resolution against the active catalog; it does not install or approve a
  package. Parsing proves syntax, while catalog-aware reconciliation proves that calls, pins,
  types, identities, and graph changes resolve for the current App.
- Editable source normally includes `//@n`, `//@v`, and `//@l` identity anchors even though the
  low-level renderer defaults to clean text. Preserve anchors for entities that should retain
  their identity.
- Free-standing `//` comments inside function and Event bodies survive text parse/render, but
  top-level prose comments currently have no AST slot and body comments are not persisted as
  canvas comments by Board reconciliation.

### Code-verified type and schema notes

- The scalar surface is `string`, `int`, `float`, `bool`, `Date`, `Path`, `bytes`, `Struct`,
  `exec`, and `any`. Value shapes are normal values, `T[]`, `Map<string, T>`, and `Set<T>`;
  container shape participates in connection compatibility even when the element type is
  generic.
- Concrete data types and value shapes must agree at known pin boundaries. `any` maps to the
  platform's Generic type and remains deliberately permissive, while enforced Struct schemas
  are currently compared by normalized equality rather than structural assignability.
- This authoring check is strongest for typed wires and anchored boundary contracts. Direct
  literal arguments are not uniformly validated against their destination pin during Apply;
  editor diagnostics are best-effort and a typed runtime consumer may be the first hard gate.
- A type name that does not resolve to a declared interface can currently fall back to a
  schema-less Struct. Parser acceptance alone therefore does not prove nominal type-name safety.
- Typed Struct boundaries can project schema fields into dynamic pins through **Break Struct**
  and **Make Struct (Schema)**. Schema-less Structs stay open and use path-based **Get Field**
  and **Set Field** operations instead.
- Current schema-to-pin projection is narrower than the interface grammar: enum-only fields can
  become Generic and map-shaped interface properties can become Struct/Normal field pins. Keep
  those as schema-level facts unless release tests prove a more precise projection.
- A changed schema does not silently delete a still-wired projected field. Make/Break retain the
  stale pin and put an error on the node so the broken connection remains visible; an unwired
  stale field can be removed.
- Literal string brackets parse as Struct member access, so arbitrary schema keys render as
  `value["external-id"]`; numeric and dynamic brackets remain collection indexes.
- Dynamic **Get Field** returns `null` and `found = false` for an absent path. If an author ignores
  `found`, a downstream typed consumer may become the first operation able to prove the mismatch.
  Runtime conversion failures are logged with node attribution, and the current Runs view can
  focus that node from its evidence.

### Code-verified catalog discovery and migration notes

- Dropping a wire from a pin opens the Board menu with **Context Sensitive** enabled. Candidate
  nodes are filtered to those with an oppositely directed compatible pin; authors can disable the
  filter and can search names, friendly names, categories, descriptions, and pin labels.
- Known FlowScript receiver types narrow member completion: a string offers string methods and an
  integer offers integer methods. Unknown receivers intentionally show methods from every class,
  grouped by their expected type. Namespace completion, signature help, and auto-import edits are
  derived from the declaration index.
- Generated `.flow.d` files describe calls for people and tooling, but the syntax parser does not
  bind calls from that snapshot. Catalog-aware reconciliation against the live registry remains
  the semantic authority; a successful parse alone does not prove a callable node is available.
- The Board's current search order uses textual relevance and its unfiltered menu is alphabetical.
  Although node quality scores exist and are visible in node information, the picker and
  FlowScript completion do not currently rank alternatives by safety, permissions, performance,
  cost, or trust. That ranking remains Vision.
- Catalog synchronization preserves a placed node and reuses same-named compatible pin IDs and
  wires. It adds new static pins and preserves connections when widening a pin to Generic.
- The current unsafe-change path is less conservative than the intended doctrine: an ordinary
  removed pin is deleted, and a changed concrete type clears its connections and resets its
  default. Dynamic schema-minted pins have stronger protection and can retain wired stale pins
  with a node error. Do not promise uniform error annotation until static migration is aligned.

### Code-verified expression and sugar notes

- Binary operators select a catalog family from operand types. A known String plus Integer and a
  declared Generic (`any`) plus Integer are hard mismatches; no conversion node is inserted.
  Apply is atomic when reconciliation reports a diagnostic.
- Only an Integer literal can adopt a known Float operand's family. Separately typed Integer and
  Float values remain incompatible until converted explicitly.
- Current text-to-Board families cover String equality/inequality/concatenation, Boolean equality
  and composition, Integer comparison/arithmetic, and Float comparison/arithmetic. Important
  exceptions are Float equality/inequality, Boolean inequality, `|`, integer bitwise operations,
  and Integer division: some parse, some have explicit catalog nodes, but their operator mappings
  are absent or inconsistent. Keep them explicit until the registry is reconciled.
- Board-to-text currently includes Float equality and inequality in its sugar table even though
  text-to-Board and existing-source reuse deliberately exclude them because tolerance is
  meaningful. An untouched Float comparison graph can therefore render source that fails Apply
  even against the Board from which it was rendered.
- Operator rendering keeps the explicit node call when a trailing input is wired or has edited
  nonzero configuration. This preserves `ignoreCase: true` and additional concat inputs.
- `!x` parses into a Boolean Not call; `-x` canonicalizes to `0 - x`. Only `+=`, `-=`, `*=`, and
  `/=` are accepted, and canonical rendering expands them into assignment plus a binary operator.
- A ternary materializes Types Select. Its runtime reads only the chosen data input, but it is not
  an Execution branch. Its current sugar recognizer assumes today's three-input Select shape and
  should be hardened if that node gains meaningful configuration.
- Template literals materialize String Format with dynamic placeholder pins. Board rendering uses
  the template only when the literal format, resolved placeholder set, and regenerated pin names
  match exactly; otherwise it preserves the explicit call.
- Struct field assignment feeds the prior Struct into Set Field and rebinds later references to
  `struct_out`. Earlier consumers retain their earlier producer; the runtime mutates an owned
  value and does not retroactively change an upstream pin.
- Generic Get Field has `value` and `found` outputs, but current Board lowering does not check which
  one is selected before rendering member-access sugar. Use an explicit call when `found` matters
  until this is corrected.
- The proposed mixed-operator UX is Vision: after a rejected Apply, offer separate explicit
  conversion repairs, then Apply and canonically render the chosen visible node. Do not silently
  choose numeric addition versus text concatenation.

### Code-verified control-flow notes

- `else if` is accepted when authoring and canonicalizes to an `if` nested in the preceding False
  arm. Named execution-arm blocks preserve the exact execution output names of a catalog node.
- Sequential For Each evaluates its array once, processes values in input order, and awaits each
  body chain before the next item. A child error ends that item path, is logged, and does not stop
  later items.
- Parallel For Each defaults to 30 active item/body-root tasks. A positive value bounds active
  tasks; non-positive means unlimited. It schedules remaining items after child failures, waits
  for all children before Done, and exposes no collected result whose ordering could be promised.
- Plain `@parallel for` and `while (condition)` preserve only hidden defaults. Custom concurrency
  or maximum iteration values render as explicit `control::parallelForEach` or
  `control::whileLoop` calls.
- While reevaluates condition dependencies before each body, runs at most 15 iterations by
  default, and silently activates Done if the condition remains true at that ceiling. It has no
  duration limit or Exhausted arm. Child errors are logged and later iterations continue.
- Sequential, parallel, While, and break-capable loop nodes currently absorb child errors and
  return Ok. As a result, Error evidence can coexist with a terminal Success run. This conflicts
  with the intended aggregate-failure invariant and needs explicit runtime coverage.
- FlowScript has no `break` or `continue` AST statements. For Each (Break) is a separate visual
  node controlled by a Boolean input and is not registered for structured loop lowering.
- Function `return` wires data sources positionally to Function boundary outputs and does not
  terminate execution. The safe supported shape is one final unconditional return. Event/handler
  return creates a terminal Return Result node for one branch and accepts one value.
- There is no platform-wide first/last rule for competing Event results. Synchronous collectors
  commonly retain the first emitted result, while context merging and UI state can retain the
  most recently merged or observed result. Publish one logical result path instead of racing them.
- Two statically identified round-trip edges need release tests before publication: a consumed
  While `iter` output can lose its handle name because While syntax has no binding, and duplicate
  same-named execution outputs use an indexed selector internally that the current renderer appears
  to normalize into a different label. Do not use either shape in worked examples yet.

## Platform and application model

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-PLATFORM` | `apps/docs/src/content/docs/start/what-is-flow-like.mdx`; `apps/docs/src/content/docs/dev/architecture.md`; `packages/core/src/app.rs` | Product vocabulary and the relationship among platform, Apps, Flows, Boards, interfaces, data, and runtime | Mark client and service capabilities by maturity; do not imply every surface has identical feature coverage. |
| `SRC-APP-MODEL` | `apps/docs/src/content/docs/apps/overview.md`; `apps/docs/src/content/docs/apps/create.md`; `apps/docs/src/content/docs/apps/offline-online.md`; `apps/docs/src/content/docs/apps/share.md`; `packages/core/src/app.rs`; `packages/core/src/app/sharing.rs` | App boundaries, offline/online modes, sharing, and visibility concepts | Publication thresholds and allowed transitions can be deployment-configured. Avoid hard-coded prototype limits. |
| `SRC-IDENTITY-PERMISSIONS` | `packages/api/src/routes/auth.rs`; `packages/api/src/permission/role_permission.rs`; `packages/api/src/routes/app/team/`; `packages/api/src/routes/app/events/invoke_event.rs`; `packages/api/src/routes/app/events/remote_proxy.rs`; `packages/core/src/flow/execution/user_context.rs`; `packages/catalog/std/src/utils/user/`; `packages/ui/lib/permission/`; `apps/desktop/lib/university/courses/advanced/app-governance/content/03-permission-architecture.md` | Authentication entry points, App membership, roles, invitations, Event execution/edit permissions, connected-App caller context, and in-Flow identity checks | There is no per-App-Event ACL or built-in two-party release gate. Separate platform identity, App membership, REST registration auth, connected-App identity, domain authorization, node capabilities, and execution credentials. Arbitrary custom user attributes exist in core but are not populated by every API path. |
| `SRC-APP-SURFACES` | `apps/docs/src/content/docs/apps/pages.md`; `apps/docs/src/content/docs/apps/routes.md`; `apps/docs/src/content/docs/apps/a2ui.md`; `apps/docs/src/content/docs/dev/a2ui/visual-builder.md`; `packages/core/src/a2ui/copilot/component_docs.rs`; `packages/ui/components/a2ui/ActionHandler.tsx`; `packages/ui/components/interfaces/` | Pages, routes, A2UI rendering, workflow-event actions, current element-state payloads, chat/widget integration, and visual application surfaces | The action context carries routing identity rather than arbitrary form values; the client assembles current element state separately. UI builders and component coverage change independently of FlowScript. Anonymous Page hosting remains future work. |
| `SRC-API-SURFACES` | `packages/catalog/web/src/web/rest.rs`; `packages/catalog/web/src/web/remote.rs`; `packages/ast/flow.d/web.flow.d`; `packages/api/src/routes/app/events/setup_event.rs`; `packages/api/src/routes/app/events/upsert_event.rs`; `packages/api/src/routes/app/events/registrations.rs`; `packages/api/src/routes/inbound.rs`; `packages/api/src/openapi.rs` | REST setup composition, function registration, authentication, remote setup persistence, inbound dispatch, structured results, generated OpenAPI, and last-successful setup fallback | Remote OpenAPI bodies are currently open objects; inbound fields are not schema-validated before execution; remote setup persists only the first handler reference per route; malformed/auth/unmatched rejections happen before Run insertion; and stored canary configuration is not selected by audited dispatch paths. Distinguish the platform API document from an App Event's generated document. |
| `SRC-EVENTS` | `apps/docs/src/content/docs/apps/events.md`; `packages/core/src/flow/event.rs`; `packages/core/src/flow/board.rs`; `packages/catalog/data/src/events/simple_event.rs`; `packages/catalog/data/src/events/generic_event.rs`; `packages/ui/lib/event-config.ts`; `packages/ui/components/ui/event-form.tsx`; `packages/api/src/routes/app/events/`; `apps/{web,desktop}/lib/*event-state*` | Flow event entries, compatible App Event surfaces, variable overrides, Latest/pinned versions, Local/Remote Event modes, and Hybrid Board/client dispatch behavior | Keep an event node distinct from the App Event that exposes it. An App Event is never Hybrid. Web dispatch is remote; Desktop can run locally. Stored canary fields do not establish weighted routing. Pre-dispatch rejection is not represented as a Rejected Run today. |
| `SRC-LAYERS` | `apps/docs/src/content/docs/studio/layers.md`; `apps/desktop/lib/university/courses/foundations/thinking-in-flows/content/05-functions-and-collapse.md`; `packages/ui/lib/layer-to-function.ts`; `packages/core/src/flow/board/commands/layer.rs`; `packages/core/src/flow/board/cleanup/bridge_layers.rs` | Presentational layers, placeholders, Function conversion, typed boundaries, and graph maintenance | Ordinary layers and callable Function layers are different abstractions. Do not present every visual layer as a FlowScript function or a cross-Flow API. |
| `SRC-FUNCTION-EXECUTION` | `apps/docs/src/content/docs/dev/writing-nodes.md`; `apps/docs/src/content/docs/studio/nodes.md`; `packages/core/src/flow/node.rs`; `packages/core/src/flow/execution/internal_node.rs`; `packages/catalog/std/src/control/call_function.rs`; `packages/core/src/flow/ast/reconcile.rs`; `packages/core/src/flow/ast/lower.rs`; `packages/ui/components/flow/flowscript/flowscript-language.ts` | Node-designer purity, backward dependency evaluation, Function impurity inference, pure/impure calls, first-parameter method dispatch, and completion | Runtime purity is structural absence of Execution pins, not proof of determinism or side-effect freedom. Branches and loops currently make functions execution-driven. User-method spelling may normalize to a flat call after Board projection. |
| `SRC-FUNCTION-CACHE` | `packages/ast/src/model.rs`; `packages/ast/src/parse/parser.rs`; `packages/ast/src/render.rs`; `packages/catalog/std/src/control/call_function.rs`; `packages/catalog/std/tests/function_cache_continuation.rs`; `packages/catalog/data/src/data/cache/`; `packages/api/src/cache/types.rs`; `packages/api/src/routes/app/cache.rs`; `packages/ui/components/flow/layer-editing-menu.tsx`; `packages/ui/components/flow/functions/function-overlay.tsx` | `@cache` syntax/defaults, key construction, scope/namespace/TTL, hit/miss behavior, invalidation, and visual configuration | A hit skips the whole body. Keys omit code, packages, globals, configuration, models, external state, and output contract unless represented through inputs/versioning. Identical misses are not single-flight. Visual and FlowScript defaults currently differ. |
| `SRC-AGENT-HANDLERS` | `apps/docs/src/content/docs/studio/flowscript.md`; `packages/core/src/flow/ast/lower.rs`; `packages/core/src/flow/ast/reconcile.rs`; `packages/core/src/flow/ast/apply.rs`; `packages/catalog/llm/src/agent/register_tools.rs`; `packages/catalog/llm/src/agent/helpers.rs`; `packages/catalog/data/src/events/generic_event/push_generic_result.rs` | Nested Event handlers, explicit agent function references, inferred tool inputs, tool invocation, and result forwarding | A plain Function layer is not a triggerable tool. Only eligible referenceable Event entries work today; no automatic Function shim, required-argument list, per-tool confirmation, caller policy, or output schema should be promised. |
| `SRC-UNREAL-BLUEPRINTS` | `https://dev.epicgames.com/documentation/en-us/unreal-engine/functions-in-unreal-engine`; `https://dev.epicgames.com/documentation/unreal-engine/ufunctions-in-unreal-engine`; `https://dev.epicgames.com/documentation/en-us/unreal-engine/basic-scripting?application_version=4.27` | Official Blueprint pure/impure definitions, execution wiring, demand evaluation, and the warning that pure results are not cached | Use Unreal as a conceptual comparison, not proof of identical Flow-Like scheduling, per-consumer reevaluation, object ownership, or implementation lineage. |
| `SRC-VARIABLES` | `apps/docs/src/content/docs/studio/variables.md`; `apps/docs/src/content/docs/apps/runtime-variables.md`; `packages/core/src/flow/variable.rs`; `packages/catalog/std/src/variables/` | Board variables, runtime variables, reads, writes, and mutation | Top-level `const` means non-exposed and top-level `let` means exposed; this is not JavaScript immutability. `@readonly` maps separately to editability metadata. Function-local bindings have different semantics. |
| `SRC-RUNTIME-CONFIG` | `apps/{web,desktop}/lib/runtime-vars-db.ts`; `packages/ui/state/execution-service-context.tsx`; `packages/api/src/routes/app/prerun_shared.rs`; `packages/api/src/routes/app/events/db.rs`; `packages/core/src/flow/execution.rs`; `apps/desktop/app/library/config/configuration/page.tsx` | Local runtime-value persistence, override precedence, interactive preflight, remote secret filtering, Event overrides, and exposed configuration editing | The local key is App plus variable, not authenticated user; presence is not full validation; core/direct execution can fall back to defaults or `null`; and official remote clients omit secret runtime values. Do not promise strict per-user isolation or universal fail-closed preflight yet. |
| `SRC-REDACTION` | `packages/ast/src/redact.rs`; `packages/core/src/flow/ast/lower.rs`; `packages/core/src/flow/ast/reconcile.rs`; `packages/api/src/routes/app/board/secrets.rs`; `packages/api/src/routes/app/events/db.rs`; `packages/catalog/std/src/variables/get.rs`; `packages/catalog/std/src/logging/info.rs`; `apps/desktop/lib/university/courses/advanced/app-governance/content/05-secrets-and-execution.md` | Secret omission from source/read APIs, guarded secret writes and schema changes, and protected deletion paths | Secret metadata is not taint tracking. A downstream node can still log or return a secret. Masking also does not prove independent encryption at rest; verify every serialization, preview, error, and deployment storage path before making broader claims. |
| `SRC-PROVIDER-CREDENTIALS` | `packages/ui/components/settings/model-catalog/add-custom-model-dialog.tsx`; `packages/api/src/routes/user/bits.rs`; `packages/api/src/utils/crypto.rs`; `packages/api/src/execution/dispatch.rs`; `apps/desktop/components/tauri-provider/bit-state.ts`; `apps/desktop/src-tauri/src/settings.rs` | Private hosted model/provider credentials, server-side encryption, response filtering, execution hydration, and the offline Desktop copy | Hosted server storage and offline Desktop storage have different boundaries. Do not imply that the downloaded Desktop settings copy receives the server store's application-level encryption. |
| `SRC-VERSIONING` | `apps/docs/src/content/docs/studio/versioning.md`; `packages/core/src/flow/board.rs`; `packages/core/src/flow/event.rs`; `packages/api/src/routes/app/board/version_board.rs`; `packages/api/src/routes/app/board/get_board_versions.rs`; `packages/api/src/routes/app/events/setup_event.rs`; `packages/api/src/routes/inbound.rs` | Immutable Board versions, Latest resolution, App Event version binding, and last-successful REST setup selection | A Flow version and an App Event setup version freeze different things. Package resolution, Event variables/configuration, data, credentials, and external systems may have independent lifetimes. Canary storage is not evidence of canary dispatch. |

## Execution, observability, and compiled artifacts

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-EXECUTION` | `packages/core/src/flow/execution.rs`; `packages/core/src/flow/execution/`; `packages/executor/src/execute.rs`; `packages/api/src/execution/` | Rust execution model, context, dispatch, payloads, and executor integration | Do not teach an inactive or experimental engine as the default. Verify feature flags and each backend at the target revision. |
| `SRC-OBSERVABILITY` | `apps/docs/src/content/docs/studio/logging.md`; `packages/core/src/flow/execution/log.rs`; `packages/core/src/flow/execution/trace.rs`; `packages/api/src/routes/app/board/query_logs.rs`; `packages/api/src/middleware/trace_context.rs`; `packages/ui/components/flow/traces.tsx`; `packages/ui/components/flow/flow-runs.tsx`; `packages/ui/components/flow/flow-node.tsx`; `apps/desktop/src-tauri/src/functions/storage_management.rs` | Node/run logs, traces, query surfaces, visual attribution, local retention, and trace context | Separate execution evidence, product telemetry, infrastructure monitoring, and audit records. They have different scopes and retention. A trace does not automatically persist every pin value, and its internal trace ID is not part of stored log rows. |
| `SRC-RERUN` | `packages/ui/components/flow/flow-runs.tsx`; `packages/ui/components/flow/flow-board.tsx`; `packages/api/src/routes/app/board/invoke_board.rs`; `packages/api/src/routes/app/board/get_runs.rs`; `packages/api/src/execution/payload_storage.rs`; `packages/api/src/execution/wasm_resolve.rs`; `packages/api/src/execution/dispatch.rs` | What the current Re-Run action carries forward and what each new invocation resolves afresh | Do not use “rerun,” “replay,” and “reproduce” interchangeably. Local and remote payload recovery currently differ, and an omitted Board version targets Latest. |
| `SRC-COMPILED-FLOW` | `packages/core/src/flow/compiled/`; `packages/api/src/execution/compiled_artifacts.rs` | Compact versioned compiled-Board artifacts and executor preparation | A `.flcb` artifact is not a universal native binary. It is format-, version-, and catalog-bound and is separate from a WASM node. |
| `SRC-BENCHMARKS` | `apps/docs/src/content/docs/reference/benchmarks.mdx`; repository benchmark targets and release-matched benchmark output | Measured preparation, size, throughput, and latency claims | Publish hardware, dataset, revision, configuration, sample size, and methodology. Existing benchmark prose must be rerun before citation. |
| `SRC-OPERATIONS` | `apps/docs/src/content/docs/self-hosting/docker-compose/monitoring.md`; `apps/docs/src/content/docs/self-hosting/kubernetes/monitoring.md`; `apps/backend/kubernetes/helm/templates/monitoring/` | Infrastructure metrics, dashboards, tracing components, and operating guidance | “Observable by default” must name the deployment shape and enabled components. Retention and alerting are operator policy. |

### Code-verified runtime notes

- Normal execution follows topology: one ready target runs singly; several ready targets run
  concurrently, bounded by executor capacity. An unhandled failure stops its own successor
  path without cancelling sibling targets and marks the ordinary run failed after the
  siblings finish.
- Node-defined Success/Error outputs are normal outcomes. Studio's optional **Handle Errors**
  facility is the generic unexpected-error path; a completed recovery chain preserves
  node-attributed Error evidence but prevents that handled error from failing the run.
- Dedicated Parallel and Sequence control nodes currently catch or discard some errors from
  their child chains. Do not promise uniform aggregate run status across those nodes without a
  release-specific test.
- Persisted log rows contain messages, severity, timestamps, optional node/operation fields,
  and optional usage statistics. Normal execution attributes them to nodes; Debug log level
  provides timed node-execution records. Pin inputs and outputs are not automatically
  persisted, and arbitrary author messages are not universally redacted.
- Numbered Board snapshots are immutable, but the current Runs-panel Re-Run action invokes
  Latest with the old local payload. It does not restore historic package resolution, runtime
  values, profiles, credentials, stored data, external responses, or default-selected models.
  Remote run history currently does not return its separately stored input payload.
- Even an explicitly pinned Board fixes authored graph and configuration, not necessarily the
  catalog implementation across platform upgrades: compiled artifacts are registry-fingerprint
  bound and may be rebuilt through the current node registry.

## Data, application surfaces, and AI

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-DATA` | `apps/docs/src/content/docs/apps/storage.md`; `packages/storage/`; `packages/catalog/data/src/data/` | App storage, files, paths, tables, SQL, vectors, and data nodes | Storage guarantees depend on provider and deployment. Avoid treating every backend as behaviorally or operationally identical. |
| `SRC-STATE-LIFETIMES` | `packages/core/src/flow/execution.rs`; `packages/core/src/flow/execution/context.rs`; `packages/catalog/data/src/data/cache/`; `packages/api/src/cache/types.rs`; `packages/catalog/data/src/data/path.rs`; `packages/catalog/data/src/data/db/vector.rs`; `packages/ui/components/interfaces/chat-default/chat-db.ts`; `packages/ui/components/interfaces/chat-default.tsx` | Run- and invocation-local values, durable key-value cache, file storage paths, App/user database scope, and current chat-session persistence | Key-value cache and the file cache directory are different mechanisms. Cache is replaceable state, database versions are not automatically a permanent audit history, and current chat “global” state is still local to a client/App/Event rather than organization-wide. |
| `SRC-DATA-VERSIONS` | `apps/desktop/src-tauri/src/functions/app/tables.rs`; `packages/storage/Cargo.toml`; `packages/storage/src/databases/vector.rs`; `packages/storage/src/databases/vector/lancedb.rs`; `packages/ui/components/ui/lance-viewer.tsx`; `packages/catalog/data/src/data/db/vector/optimize.rs`; `packages/catalog/data/src/data/datafusion/data_lakes.rs`; `apps/docs/src/content/docs/topics/datascience/datafusion.md` | Inspected desktop Data Studio table storage, LanceDB version pruning, and explicit Delta/Iceberg time-travel operations | The current VectorStore API exposes optimization but not version checkout. “Keep Versions” follows the storage engine's bounded retention behavior; it does not mean permanent history. Delta/Iceberg support is feature-gated. Do not imply that every backend is versioned identically or that Re-Run automatically selects a historic data snapshot. |
| `SRC-DATA-STUDIO` | `apps/docs/src/content/docs/apps/data-studio.md`; `apps/website/src/content/blog/2026-07-12-data-studio-ontologies.mdx`; `packages/catalog/data/src/data/db/graph/` | Data Studio concepts, ontology-backed data, graph operations, and actions | Distinguish implemented editors and operations from the full ontology/governance vision. |
| `SRC-DATA-PIPELINES` | `apps/docs/src/content/docs/topics/data-pipelines/overview.md`; `packages/catalog/data/src/data/datafusion.rs`; relevant generated declarations under `packages/ast/flow.d/` | Ingestion, transformation, query, and storage building blocks | “Big data” and scale language requires measurements for the actual storage and execution backend. |
| `SRC-RAG` | `apps/docs/src/content/docs/topics/genai/rag.md`; `apps/desktop/lib/university/courses/specialist/data-in-flow-like/content/build-a-rag-agent.md`; `packages/catalog/llm/src/embedding/` | Retrieval, chunking, embedding, indexing, and document-agent learning sequence | Model quality, privacy, cost, and local execution vary with provider and configuration. |
| `SRC-CHAT` | `apps/docs/src/content/docs/apps/chat-ui.md`; `apps/docs/src/content/docs/topics/genai/chat.md`; chat interface code under `packages/ui/components/interfaces/` | Chat as an App surface and its connection to Flows | Keep UI affordances separate from agent/runtime guarantees. |
| `SRC-AI-AUTHORING` | `apps/docs/src/content/docs/studio/flowpilot.md`; `apps/docs/src/content/docs/studio/flowpilot-external-agents.md`; `packages/core/src/flow/copilot/`; `packages/ui/lib/flowpilot/` | AI context, declarations, validation, edit delivery, and guarded Board changes | AI generation still needs review. Explain that constraints narrow and validate the solution space; do not promise correctness. |
| `SRC-FLOWPILOT` | `apps/desktop/lib/university/courses/foundations/building-with-flowpilot/`; `apps/website/src/content/blog/2026-07-05-flowpilot-whole-app.mdx`; `packages/ui/lib/flowpilot/flowscript-generation-receipt.ts` | The practical human/AI authoring loop and change receipts | Label provider support and whole-App generation by release maturity. Marketing examples are not reliability studies. |
| `SRC-AI-DETERMINISTIC-FIRST` | `packages/core/src/copilot/prompts.rs`; `packages/core/src/flow/ast/reconcile.rs` | Model-facing guidance that reserves LLM calls for semantic work and the current 100-node per-layer reconciliation limit | A node budget constrains edit shape; it does not establish correctness or security. The number is release-specific. |
| `SRC-AI-USAGE-CONTROLS` | `packages/api/src/entity/llm_usage_tracking.rs`; `packages/api/src/entity/embedding_usage_tracking.rs`; `packages/api/src/routes/app/analytics/overview.rs`; `packages/api/src/routes/admin/usage.rs`; `packages/api/src/routes/usage/history.rs`; `packages/api/src/usage_accounting.rs`; `packages/api/src/usage_limits.rs` | Usage attribution and aggregation by App, human user, technical user, and model; cost/token windows, warnings, and hard limits | The shipped editor covers App and technical-user limits, not a separate limit for every human user. Hosted LLM prices may be provider-reported after a call, so limits are not universal pre-call guarantees. |
| `SRC-AI-MODEL-SELECTION` | `packages/core/src/bit.rs`; `packages/core/src/profile.rs`; `packages/catalog/llm/src/llm/find_llm.rs`; `packages/catalog/llm/src/llm/preferences/`; `packages/ui/components/settings/model-catalog/add-custom-model-dialog.tsx` | User-owned model profiles and weighted selection by cost, speed, reasoning, safety, coding, and other traits | Weights score authored classification metadata. They do not prove live task quality or guarantee the smallest adequate model. |
| `SRC-AI-MODEL-INVENTORY` | `packages/api/src/routes/app/ai_act/board_scan.rs`; `packages/api/src/routes/admin/ai_act/reconcile.rs`; `packages/api/src/routes/admin/models/sync_models.rs`; `packages/ui/components/ui/model-benchmarks.tsx` | Feature-gated App model inventory, observed-use reconciliation, and imported benchmark metadata | This is not automatic task-level model evaluation. Current usage rows do not identify a Board node or semantic task. |
| `SRC-VIBEINCREMENT` | `https://pypi.org/project/vibeincrement/`; `https://github.com/tahayparker/vibeincrement/blob/main/src/vibeincrement/ai.py` | The August 2025 experimental package and its actual GPT-backed increment implementation | Say that it reads as satire without asserting authorial intent. It is not representative production practice, and the model call does not perform local arithmetic. |

### Code-verified state and configuration notes

- Every run creates fresh values from caller, Event, or Board configuration. Function locals are
  fresh per invocation; Flow variables can be shared across branches of one run. Parallel writes
  are serialized by the runtime lock but their winning order is not deterministic.
- Exposed `let` values are currently edited through Board configuration and Board-write
  permission. Do not promise a separate configuration-only role until the routes use the existing
  config permission flags.
- Web and Desktop runtime-value records use `${appId}:${variableId}` in local IndexedDB. No user ID,
  sync, logout cleanup, or application-level encryption is present in that store. Standard clients
  send non-secret runtime values to remote execution and deliberately omit secret ones.
- Interactive execution prompts when a saved runtime record is absent. The check is presence-only,
  and the core runtime still resolves missing inputs through Event value, Board default, then
  `null`. Universal typed fail-closed preflight remains an intended invariant.
- `@readonly` prevents definition/configuration edits today, but Set Variable does not inspect the
  editable flag. Runtime immutability remains an intended invariant.
- Secret defaults are omitted from rendered FlowScript and ordinary Board/Event reads; non-empty
  secret initializers are rejected. Once read at runtime, however, a secret travels as an ordinary
  value and can be exposed by an author-written log or result.
- For `lastSuccessfulSync`, use cache only when loss safely causes repeated work. Use a durable
  database record when loss can skip work, repeat an irreversible effect, or violate recovery or
  audit requirements. Choose reference-data storage by access pattern: file for bulk/static,
  cache for small derived/rebuildable data, and database/Data Studio for evolving/queryable data.
- A private provider profile is the implemented hosted OpenAI BYOK boundary. Server credentials
  are encrypted and hydrated only for execution; offline Desktop downloads a local copy under a
  different at-rest boundary.

### Code-verified function, handler, and cache notes

- Runtime purity is the absence of Execution pins. Before an execution node runs, the engine walks
  its input dependencies backward and schedules reached pure nodes in dependency order. A pure
  output with no consumer is inert; an impure call on the execution chain runs even if its data
  output is unused.
- FlowScript infers a Function layer's execution boundary from its body. Impure calls, Board
  variable writes, branches, loops, and calls to other impure Functions propagate impurity. This
  makes some deterministic functions structurally execution-driven.
- Every user function with a first parameter participates in typed method dispatch on that
  parameter. Catalog methods remain receiver-metadata-driven. Current graph lowering renders Call
  Function nodes in flat form, so authored method spelling may not survive round-trip projection.
- Agent tools are explicit references to eligible Event/handler entries. A plain Function layer
  has no trigger node and is rejected as a direct tool reference; the automatic thin Event shim is
  product direction rather than current implementation. Handler types inform the tool schema, but
  current generated properties are not marked required and there is no per-reference confirmation
  or authorization object.
- `@cache` is explicit and Function-only. A hit restores named outputs, activates continuation,
  and skips the body, including side effects. Caching an impure Function is allowed today, so the
  author must ensure the body is safe to omit.
- Function cache keys hash the stable layer ID plus successfully evaluated input names and values
  inside App/user scope and namespace. They do not hash implementation or output contracts.
  Changing code, dependencies, packages, models, configuration, or external data requires an input
  revision, TTL, namespace version, and/or explicit invalidation.
- Namespace invalidation must use the same scope and a non-empty namespace. It is not a barrier:
  an older in-flight miss may write afterward. A data revision in the key prevents that late value
  from serving callers on the new revision.
- FlowScript's bare cache defaults are `global`, 300 seconds, App scope; current visual enablement
  starts with an empty namespace and no expiry. Published examples should spell namespace and TTL
  until the views converge.

## WASM nodes and packages

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-WASM-DOCS` | `apps/docs/src/content/docs/dev/wasm-nodes/overview.md`; `apps/docs/src/content/docs/dev/wasm-nodes/manifest.md`; `apps/docs/src/content/docs/dev/wasm-nodes/sandboxing.md`; `apps/docs/src/content/docs/dev/wasm-nodes/runtime-models.md` | Public extension model, manifests, language SDK routes, and intended sandbox contract | Documentation must be checked against both core-module and component execution paths. Avoid “super secure” and other absolute wording. |
| `SRC-WASM-SDK` | `packages/wasm/wit/flow-like-node.wit`; `packages/wasm/src/abi.rs`; `packages/wasm/src/manifest.rs`; language examples linked by the WASM docs | Node ABI, manifest fields, pins, host functions, and SDK contract | A declared manifest capability is only a security guarantee when runtime enforcement is verified. |
| `SRC-WASM-RUNTIME` | `packages/wasm/src/engine.rs`; `packages/wasm/src/instance.rs`; `packages/wasm/src/component/`; `packages/wasm/src/limits.rs`; `packages/wasm/src/memory.rs`; `packages/wasm/src/host_functions/`; `packages/wasm/tests/no_ambient_host_environment.rs` | Wasmtime setup, host exposure, resource configuration, component/core execution, and isolation tests | Confirm limiter wiring, inherited environment/stdio, component CLI fallback, network allowlists, and every non-interactive run path before stating boundaries. |
| `SRC-WASM-CONSENT` | `packages/ui/state/execution-service-context.tsx`; `packages/ui/components/flow/wasm-sandbox-warning-dialog.tsx`; `packages/api/src/routes/app/prerun_shared.rs` | Interactive package warnings, displayed permissions, remembered decisions, and pre-run package resolution | This proves an interactive client flow, not a universal human-consent gate for schedules, API calls, internal executions, or every client. |
| `SRC-WASM-EXAMPLES` | `examples/sales-insights/node/`; `examples/sales-insights/README.md`; `packages/wasm/tests/rust_sdk_e2e_test.rs`; `packages/wasm/tests/external_package_test.rs` | A real custom node, packaging workflow, widget integration, and test patterns | The sales example should be pinned to a known-good toolchain and package format before publication. |
| `SRC-PACKAGES` | `apps/docs/src/content/docs/start/packages-library.md`; `apps/docs/src/content/docs/start/packages-store.md`; `packages/api/src/routes/registry/`; `packages/api/src/entity/app_package.rs`; `packages/api/src/entity/wasm_package_review.rs` | Installing packages into Apps, registry publication, review records, and access | Resolve whether trust and approval attach to a package ID, version, digest, or some combination before writing supply-chain guarantees. |
| `SRC-GOVERNANCE` | `packages/core/src/copilot/governance.rs`; `packages/api/src/routes/admin/governance/`; `packages/api/src/routes/app/internal/change_visibility.rs`; `packages/api/src/publication/gate.rs`; `apps/desktop/lib/university/courses/advanced/app-governance/` | App visibility transitions, publication gates, derived scores, and governance workflow | Score direction currently appears inconsistent between comments and aggregation. Audit semantics and configurable policy must be verified in INT-06. |

## Deployment and architecture

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-ARCHITECTURE` | `apps/docs/src/content/docs/dev/architecture.md`; workspace manifests; `packages/core/`; `packages/executor/`; `packages/api/` | High-level component boundaries and shared Rust engine | An architectural directory proves implementation activity, not production readiness or feature parity. |
| `SRC-DEPLOYMENT` | `apps/docs/src/content/docs/self-hosting/`; `apps/backend/docker-compose/`; `apps/backend/kubernetes/`; `apps/backend/aws/`; `apps/backend/azure/`; `apps/backend/gcp/` | Existing local, Compose, Kubernetes, AWS, Azure, and GCP deployment implementations | Confirm supported installation paths and operational maturity. The repository does not currently substantiate a full StackIT or Cloudflare runtime deployment. |

Provider cost comparisons, claims about thousands of Apps sharing one to three cloud
environments, canary behavior, compliance posture, and maintenance savings belong in the
book only after INT-07 supplies a reproducible model or an approved case study. Describe the
platform contract first; label provider-specific optimization separately.

## Learning material and worked applications

| Evidence ID | Primary repository sources | What it supports | Drafting cautions |
| --- | --- | --- | --- |
| `SRC-UNIVERSITY` | `apps/desktop/lib/university/courses/foundations/`; `apps/desktop/lib/university/courses/specialist/`; `apps/desktop/lib/university/courses/advanced/` | Existing pedagogy, terminology, exercises, incident/debugging sequence, and audience assumptions | Reuse concepts, not large passages. The book needs one continuous narrative and runnable fixtures rather than a course catalog. |
| `SRC-CAPSTONE-DOCS` | `tests/ast/ttwctnp08u18sg2z6nmcqqak.flow`; `tests/ast/ttwctnp08u18sg2z6nmcqqak.anchored.flow`; `tests/ast/ttwctnp08u18sg2z6nmcqqak.board` | An existing substantial document/agent Flow and its text/graph companions | Distill it into teachable stages. Do not print a roughly 500-line fixture as a single unexplained listing. |

The deterministic **Incident Triage** tutorial should become a new small book fixture. Its
minimum shape is event input → normalize → classify/branch → deliberate log or failure →
response. It must require no external account, model, or secret and should be tested through
parse, reconcile, text rendering, and execution where practical.

The **Incident Room** capstone should grow from `SRC-CAPSTONE-DOCS`, `SRC-RAG`, `SRC-CHAT`,
and `SRC-DATA-STUDIO`. `examples/sales-insights` supplies a later custom-node/widget case
study rather than replacing the main capstone.

## Open fact-check gates

The following statements are not approved as unqualified book claims yet:

1. **“Every Board round-trips losslessly.”** Say that the views preserve supported program meaning
   and untouched Board identity. Do not promise byte-for-byte source style, pixel-for-pixel canvas
   reconstruction, comment parity, or support for every Board shape.
2. **“WASM nodes are completely isolated.”** Verify resource limits, capabilities, ambient
   host access, consent, version trust, and every execution path in INT-05.
3. **“Governance scores show how good an App is.”** Resolve score direction, provenance,
   aggregation, and policy override in INT-06.
4. **“Every target has the same production-ready deployment.”** Classify each target as
   supported, preview, partial, or planned in INT-07.
5. **“AWS is cheapest and Azure is most expensive.”** Require a defined workload, region,
   architecture, date, pricing inputs, and reproducible calculation.
6. **“Ten thousand Apps need only one to three cloud rooms.”** Require an architecture model
   and operational case study, including isolation, quotas, failure domains, and ownership.
7. **“Audit and governance come for free.”** State what evidence is automatic, which failures
   are surfaced, what is configurable, and what still requires people and operating policy.
8. **“The node catalog contains N nodes.”** If useful at all, generate the number for the
   pinned release and label it as a snapshot.
9. **“Web, desktop, mobile, and embedded clients have parity.”** Establish actual supported
   authoring and execution capabilities per client.
10. **“FlowScript is 99% complete.”** Replace the percentage with a named parity matrix and a
    list of unsupported or lossy Board constructs.

## Drafting checklist

Before a chapter moves from outline to manuscript:

- answer its interview dependency in `INTERVIEWS.md`;
- read every cited source rather than relying on this summary;
- pin and run its examples against one repository revision;
- include both the FlowScript and graph representation for meaningful logic;
- test one relevant failure path;
- mark changing features Current, Preview, or Vision;
- move volatile signatures and exhaustive catalog material into generated reference links;
- attach measurements to performance, scale, cost, or security-strength claims; and
- add unresolved discrepancies to the fact-check queue instead of smoothing them over.
