---
title: Develop and evaluate FlowPilot
description: Run app-generation benchmarks and inspect the staged app-build preview
---

This guide covers the desktop evaluation harness and the opt-in staged-build
preview. For everyday authoring, see [FlowPilot](/studio/flowpilot/).

## Run app-generation evaluations

Use the repository-root CLI to evaluate app generation through the real
development desktop runtime. Live runs spend model budget and keep generated
apps for inspection. Install the [desktop prerequisites](/dev/build/) and authenticate
the Codex CLI. A normal run uses the desktop's local profile and requires other desktop
instances to be closed. Add `--isolated` to start with private storage and an empty local
profile instead:

```sh
bun run flowpilot:e2e -- --case simple-agent
bun run flowpilot:e2e -- --suite smoke --min-chars 1200 --json
bun run flowpilot:e2e -- --case forum --case ops-dashboard --repeat 3 --fail-fast
bun run flowpilot:e2e -- --case ai-adventure --model sol
bun run flowpilot:e2e -- --case simple-agent --tier behavioral
```

`--model` pins the generation model for the parent turn and every nested specialist. It accepts a
benchmark alias or the model id itself: `terra` / `gpt-5.6-terra` (default) and `sol` /
`gpt-5.6-sol`. The harness assigns each result a cohort key derived from its declared harness
contract version, exact provider/model/reasoning tuple, validation tier, and case-suite fingerprint.
The comparison guard rejects scores from different cohorts. Changing a model or fixture therefore
requires a new baseline; the cohort key does not claim that thresholds were recalibrated.

The default `structural` tier checks compiler receipts, persisted readback, and app structure.
`--tier behavioral` additionally requires host-produced scenario results from an attested isolated
runtime, complete assertions, and successful outcomes for every started run. The current desktop
host does not advertise runtime isolation, so this opt-in tier fails explicitly until a real
isolated adapter supplies that evidence. It never treats a transport acknowledgement as workflow
success.

Useful inspection modes do not start Tauri or spend model budget:

```sh
bun run flowpilot:e2e -- --list
bun run flowpilot:e2e -- --case simple-agent --dry-run
```

By default Tauri starts the normal desktop Next dev server at `http://localhost:3000`. For a faster
edit/run loop, keep that Next server running and pass `--frontend-url http://localhost:3000` so the
CLI reuses it. Normal runs reject a running desktop process to keep one writer for shared local
app data. `--isolated` creates a fresh, private temporary root, a separate application identifier,
and an incognito webview. It inherits Codex CLI authentication without copying the desktop's
settings, app inventory or credentials. The CLI prints the data root and retains it for inspection.
Both modes keep the native single-instance guard and a CLI lock. `--keep-desktop` is available
for debugging; close that retained process before the next run.

Child-process logs go to stderr, so `--json` reserves stdout for exactly one machine-readable result,
including on infrastructure errors. Use `--output /tmp/flowpilot-e2e.json` for a stable artifact
path. Exit code `0` means every requested run passed, `1` means at least one completed benchmark
failed, and `2` means CLI/startup/transport failure. The controller independently verifies the
selection and artifact order and recomputes the final summary instead of trusting the webview's
pass bit. It also recomputes behavioral run totals across every case and repeat and checks each
report against the requested evaluation cohort.

The benchmark uses the shared desktop global-chat and `GlobalToolBridge` path,
with the selected Codex model at `high` reasoning. A nonce-protected loopback
callback returns the webview result to the controller.

The live benchmark is available in a **development** desktop build at
`/developer/flowpilot-e2e`. Production builds fail the runner preflight before
spending model budget because detailed compiler evidence is disabled there.

Quick entry points:

- One case: `/developer/flowpilot-e2e?case=simple-agent&run=1`
- Default three-case smoke suite: `/developer/flowpilot-e2e?suite=smoke&run=1`
- Every case: `/developer/flowpilot-e2e?suite=full&run=1`
- Benchmark the other model: append `&model=sol` (the header toggle does the same for manual runs)
- Override the per-case non-whitespace character floor: append `&minChars=1200`

The heaviest case is `ai-adventure`: an offline AI text-adventure with a three-screen custom game
UI, two repeated widgets, six exactly named tables including a vector-embedded per-adventure memory
store, a savepoint/restore system, and a story agent that plans the campaign up front and then
generates scenes toward the persisted global goal. It exercises far more of the surface than the
single-purpose cases, so expect it to be the slowest and the strictest.

The same runner is callable from the desktop webview console:

```js
await window.flowPilotE2E.run({
  caseId: "forum",
  modelKey: "sol",
  minFlowScriptNonWhitespaceChars: 900,
});
```

Each returned artifact includes the resolved prompt; byte-for-byte authored
FlowScript candidates; exact compiler and commit receipts per app/board; canonical board
readbacks; parser and authoritative reconciliation results; app/UI/data/event inventory; persisted
node-capability and generated-ID checks; lower/upper compactness bounds; partial collector failures;
a stable failure fingerprint for grouping regressions; and the assistant debug trace.
Compiler evidence accepts a separate `check_flowscript` receipt or the native commit path's
inline validation. The inline path must match the retained write or patch by exact source,
revision and compiler fingerprints, then prove the applied command count and persisted readback.
A generic queued response alone does not establish successful compilation or application.
The focused Vitest suite tests case construction and artifact evaluation without
invoking a model.

## Isolated FlowScript draft tests

`test_flowscript` checks a retained source revision, applies its exact compiler commands to a
disposable board, and executes it through the normal workflow runtime. Supply `draft_id`,
`expected_revision`, a named Generic Event `entry`, a fixture `payload`, and `expected_output`.
The host requires one successful Generic Event result and compares it with the expectation using
JSON equality. After a mismatch, patch the retained source and test the new revision with the
same expectation. Testing never queues or applies a live edit.

The runner covers small deterministic JSON transformations with a restricted catalog of
trusted built-in implementations and fresh memory stores. It rejects unsupported nodes anywhere
on the board, variables, macros, caches, and WASM. Ordinary local helper functions can call other
local helpers and share the same invocation, value, and error limits. Calls must name a static
function on this board; recursion and reference-based dispatch are rejected. Computed scalar,
object, and array returns are covered. Literal-only and identity passthrough helpers currently
lower to unsupported variable or reroute nodes. It has no live credentials,
external dispatch, UI, storage, or network nodes. Preparation and execution run in process with a
cooperative three-second deadline and fixed node, value, output, and invocation limits.
A blocked result means that this runner cannot verify the draft; preserve the requested
workflow and report that limitation.

The catalog includes the compiler's supported binary arithmetic, comparison, boolean and string
operators, plus ternary selection. Integer overflow, invalid exponents, division by zero and
nonfinite float results fail before native execution can panic or turn a numeric result into
JSON `null`. Concatenation checks the combined input size before allocating its result. Format
string expansion remains outside the restricted catalog.

Receipts identify the source revision and board/catalog fingerprints. A pass covers only the
supplied input/output case. It does not certify UI wiring, app Events, persistence, or other inputs,
and it does not unlock staged app-build promotion. Live integration tests still run after Apply.

The shared workflow session retains test evidence separately from compiler diagnostics. It fixes
the first expectation for each entry and input, then carries bounded input, expected output,
actual output, and runtime errors into desktop repair continuations. Changing an expectation
cannot clear a recorded failure. Editing the source makes earlier results historical until the
new revision is tested; late receipts from an older revision cannot replace current evidence.
Blocked checks remain unverified. This feedback supports repairs without adding a commit gate,
and model-authored expectations still need to reflect the user's request.

## Workflow behavior benchmark

In the desktop app, open **Packages**, click the **Developer Settings** button next to
**New package**, and choose **Workflow behavior benchmarks**. Select a configured agent
backend, and enter an explicit model ID. Choose cases and repetitions, then start the run.
Each case uses the ordinary board authoring tools on a disposable board with a five-minute
generation deadline. The suite has 16 tasks and 64 fixed input/output checks covering JSON
transformations, helper functions, and repairs that preserve existing entries.

For unattended runs, the development desktop binary also has a headless entry point. It uses
the ordinary backend and authoring tools without starting the main application, loading its
settings or projects, opening a window, or requiring a frontend server. An existing desktop
instance can stay open. Create a JSON configuration with an explicit installed backend, model,
reasoning setting (a string or `null`), case selection, repetition count, and a new absolute
output path:

```json
{
  "backend": "codex",
  "model_id": "YOUR_CONFIGURED_MODEL_ID",
  "reasoning_effort": null,
  "prompt_profiles": ["legacy", "focused"],
  "case_ids": ["profile-card", "normalize-pair-helper", "repair-casing"],
  "repeats": 1,
  "output": "/tmp/workflow-baseline.json"
}
```

Save it as `/tmp/workflow-benchmark.json`, then run from the repository root:

```sh
RUSTC_WRAPPER= cargo run -p flow-like-desktop --bin flow-like-desktop --no-default-features -- --flowpilot-workflow-benchmark /tmp/workflow-benchmark.json
```

The CLI rejects an existing output file before starting a model. It checkpoints completed runs
and writes final host scorecards. Exit status `0` means all requested cases passed, `1` means a
case did not pass, including generation deadlines, and `2` means CLI setup, execution, or report
collection failed.
Each report retains the last authored source and dispatched tool timings so a failure can be
investigated without reconstructing it from a source hash. Tool traces are capped at 256 calls;
the report marks truncation explicitly.

`prompt_profiles` accepts `legacy`, `focused`, or both. Omitting it selects `legacy`. With both
profiles, the CLI runs each case as a pair and reverses profile order across adjacent cases and
repetitions. Each profile has its own cohort and scorecards. The focused profile keeps the
FlowScript lifecycle and engine rules, uses a shorter core reference, and selects domain guidance
from the public request and existing source. Unclear requests retain every domain. Tools, runtime
limits, and host grading checks are the same for both profiles.

Ordinary desktop SDK and external-agent board sessions select the focused profile only for
explicit Generic Event transformations of JSON or primitive input values. The current board
must pass the isolated runner's static checks. Integration requests, ambiguous wording,
attachments, conversation history, retained recovery and additional host context keep the full
profile. A benchmark's explicit profile overrides this routing so control runs stay comparable.
Authoring prompts describe the tools exposed by the authoring filter. Read-only and combined
sessions retain their own tool guidance.

Transport traces distinguish MCP initialization, tool listing, requests, preflight results, and
handler dispatch. Preflight can accept a scope plan without dispatching a handler, so use the
recorded status to distinguish these results from refusals. SDK traces start at the workflow
guard. Counts continue after the 512-event trace limit. External process snapshots record the
exact prompt bytes supplied by FlowPilot,
stdin delivery, the latest protocol event kind, process exit, and cancellation. These byte counts
exclude instructions added by the CLI or provider and are not token counts. A dropped process
future retains its latest snapshot without guessing why it stopped. Snapshots contain fixed
metadata rather than model text, payloads, credentials, or stderr contents; at most 16 phases are
retained, with truncation marked explicitly.

The model receives the task and, for repairs, the existing workflow. Reference implementations
and grading inputs stay in the host. Once generation ends, the host resolves the returned commit
claim against the retained source revision and executes its command batch in the restricted
runner. It also checks required helper calls and existing entry identities. Grading results never
enter a repair continuation. A model-authored `test_flowscript` expectation is separate from
these fixed checks.

Export the JSON reports to retain model and reasoning settings, fixture and catalog hashes,
source revisions, command fingerprints, elapsed time, and check results. Source attempts count
accepted retained revisions, including invalid source; repeated reads, checks, and tests do not
count as repairs. The scorecard attributes a pass to the final committed revision. It cannot
establish whether an earlier uncommitted revision would have passed the hidden checks.

GitHub Copilot token totals come from raw provider usage events. Codex totals come from structured
CLI completion events before assistant text is processed. Missing counts, incomplete generations,
and ambiguous CLI phases remain unknown; Claude Code usage is still unknown. Keep comparisons within
an identical cohort. The build identity
covers workspace Rust sources and manifests, the dependency lockfile, target, features, and build
flags. Prompt and tool fingerprints identify the stable authoring recipes; each report records
the observed tool schema hash separately when setup reaches that stage. Setup failures remain
in their planned cohort's denominator. External CLI versions, provider-side model updates, and remote configuration still need to
be recorded alongside a baseline. This benchmark measures isolated workflow behavior. It does
not certify whole-app integration or enable staged app-build promotion.

## Search existing implementations

FlowPilot's Scout, workflow and Data Studio specialists can search persisted implementation
content with `search_workspace`, then open an exact result with `read_symbol`. Search indexes
canonical FlowScript function, Event and interface bodies, Event input contracts, table fields,
page bindings and selected bundled documentation. Ranking boosts exact symbol and title matches,
normalizes common word endings, and removes general request words. It tries prefix matches for
every term, then fuzzy matching when needed. Multiword requests also admit partial candidates
with at least two meaningful terms and half the weighted query coverage; rarer terms carry more
weight. Uppercase acronyms in the query must match. `match_mode`, `match_quality` and
`query_coverage` expose how a hit was found. Coverage measures lexical overlap. Open the source
to establish whether it implements the requested behavior before reusing it.

Start with a concrete identifier or behavior and narrow by `app_id`, `board_id` or `kinds` when
the target is known. Search defaults to the current app. Outside an app, it inspects at most four
apps from the current profile. Each app contributes at most 16 resources per kind; workflow
boards can contribute multiple symbols. `coverage.complete: false` and `coverage.issues` expose
read failures, omitted resources and projection limits. A zero-hit result with incomplete
coverage does not establish that an implementation is absent.

For example, a workflow specialist can search for a payload field used inside a helper:

```json
{"query":"account_reference","app_id":"APP_ID","kinds":["workflow","event"]}
```

Copy the selected hit's `resource_id` and `revision` unchanged into `read_symbol`. The host reads
the source again and rejects a changed revision. Workflow revisions cover the canonical board
source; contract revisions cover their searchable projection. A read returns at most 16,000
JSON-escaped content bytes. Continue with `next_offset` when present; offsets count UTF-16 code
units. Search replies are capped at 24,000 bytes and provide an expiring `next_cursor` for further
hits. Cursor requests retain the query and scope and recheck the resources they return.

The first search for a scope reads its inventories and source, then builds an index. The component
can retain two scope indexes for up to 120 seconds from collection start, within an 8 MiB budget
for serialized documents. Follow-up queries reuse that index and re-read the returned resources;
workflow hits on the same board share one revision check. Concurrent requests for the same scope
share collection work. Backend or profile changes and stale or unreadable exact reads invalidate
retained indexes. Collections with read failures are not retained.

`freshness` reports the observation time, corpus age, cache outcome and number of revalidation
reads. `coverage_basis: "observed_snapshot"` means inventory coverage describes that observation.
The cache does not poll for newly created resources or changes to resources outside the returned
hits. These can remain undiscovered until the scope expires or is invalidated. A cached zero-hit
answer therefore cannot establish current absence, even when its observed coverage was complete.
Narrowing the app or board reduces cold collection cost; retaining an index reduces follow-up
work without eliminating source checks.

These tools share the existing pre-draft inspection budget. Read the most relevant result and
make one focused refinement if needed. A stale result permits a fresh search without resetting
that budget. Scout handoffs carry resource IDs and revisions so the implementing specialist can
read current evidence. The public-web researcher and app orchestrator do not expose these tools.

Search covers contracts and implementation source, excluding table rows, Event configuration
values, global variable declarations, page state values and embedded widget definitions. Event
schemas retain bounded structure and reachable local references; omitted validation constraints
are marked as incomplete. Indexed text remains untrusted source content, and symbol extraction
does not validate workflow execution. Use the compiler, draft checks and behavior tests before
reusing an implementation.

The documentation corpus contains sections from nine selected FlowScript, Event, data and A2UI
pages. Its response metadata lists included source paths and exclusions. After editing those
pages, regenerate the bundle and check that the generated content is current:

```sh
bun apps/docs/scripts/generate-flowpilot-corpus.ts
bun apps/docs/scripts/generate-flowpilot-corpus.ts --check
```

To compare retrieval over committed workflow fixtures and bundled documentation, run the
development queries and the frozen holdout separately:

```sh
bun apps/desktop/scripts/benchmark-workspace-search.ts --output /tmp/workspace-search-development.json
bun apps/desktop/scripts/benchmark-workspace-search.ts \
  --queries apps/desktop/scripts/workspace-search-holdout.json \
  --output /tmp/workspace-search-holdout.json
```

The default queries live in `apps/desktop/scripts/workspace-search-queries.json`. The independently
authored holdout has 30 positive and 10 absent-capability queries, with different target symbols
and documentation sections. It was frozen before evaluating the revised ranker. Keep its queries
and expected targets unchanged. Once holdout failures inform tuning, freeze another independent
set for the next evaluation.

The report compares the frozen earlier ranker, the current full-text ranker, and the current
ranker with body content removed. The descriptor-only comparison measures the contribution of
body text. None of these replays the old agent. Expected-target hit rates use sparse acceptable
answer labels, so report absent-query abstention alongside them. Known-target reads separately
compare the actual `read_flowscript_source` and `read_symbol` handlers. Add `--quality-only` to
skip timing and response-size experiments; `--repetitions` controls local timing repeats.

The JSON report records source and query hashes, per-query results, serialized response sizes,
local timing distributions and backend read counts. `index_build_and_rank_ms` rebuilds the index;
`retained_index_query_ms` measures querying an existing index. Service experiments separate cold
and follow-up searches at profile, app and board scope. They use an in-memory fixture backend
with real source files and inject 0, 25 or 100 ms per inventory/source read. The injected delays
show sensitivity to backend cost; they are simulated timing, not observed hosted latency.
This benchmark does not invoke a model or measure app-build success.

### Compare live app builds

Run two pairs through the development desktop and real Codex provider:

```sh
bun run flowpilot:e2e -- --retrieval-ab --isolated --repeat 2 --model terra \
  --output /tmp/flowpilot-retrieval-ab.json
```

This selects `retrieval-intake`, the only supported comparison case. Each pair starts from the
same seeded source app and creates two separate destination apps with the same requirements and
model settings. One arm uses the frozen earlier ranker with index retention disabled; the other
uses current ranking and retention. The runner reverses arm order on the second repeat, producing
four app-build attempts. Runs are serial and use the structural tier; `--suite`, `--fail-fast`
and behavioral acceptance are unavailable in this mode.

The task retrieves an existing intake policy and builds a page, Event, reusable helper and ticket
table around it. Host checks inspect compiler receipts, persisted references, table field types,
policy labels and deadlines, and a static call path to the helper from the registered Event on the
page's owning board. These are structural checks. Generated workflows are not executed, so a
pass does not establish routing precedence, runtime persistence or UI behavior. `--isolated`
separates app storage and webview state; it does not enable the behavioral tier.

The controller verifies the paired model, normalized prompt, fixture and runtime-source
fingerprints, distinct destination apps, and unchanged source before comparing results.
`retrievalComparisonResults` includes completed structural requirements, failed wiring checks,
generation attempts, elapsed time, retrieval-call durations and response bytes, and available
provider token counts. Missing usage and repair-decision counts remain `null`. Check
`retrievalExercised` before attributing a difference to search. These timings come from actual
model-driven builds, while the earlier fixture benchmark uses injected backend delays. Two pairs
help expose failures and order effects; use more repeats and additional cases before generalizing
to complex-app success rates.

### Check intake completion through the runtime

The `intake-reliability` case uses the host's AppSpec provisioning operations to create a local
app, reuse its empty default board, verify the exact ticket schema and reserve page and Event IDs.
The model builds the page and workflow against those identifiers. After generation, the host
resolves the persisted `submitTicket` entry and registers the reserved Events. This measures a
fixed, host-provisioned intake build; its completion rate is a different cohort from free-form
app creation or the paired retrieval experiment.

Build the workflow first so the page can bind its button to the persisted entry node ID. Page
actions carry form elements in the runtime payload. The workflow reads the input through
`ui::getElement` and `ui::getElementValue`, then updates the result through `ui::setElementText`.
An action's context does not populate top-level Generic Event pins, and returning a value alone
does not update the page.

Use `flowpilot_board` with `mode="inspect"` and exact app and board IDs to read authoritative
canonical FlowScript, entry IDs and bounded node/pin connections without a specialist or writes.
Check coverage before treating the result as complete. Compiler diagnostics are unavailable from
these read APIs; inspection does not compile or execute the workflow. `mode="explain"` delegates
a question to a specialist, whose paraphrase is separate from the canonical source.

Canonical source renders a graph. Anchors carry node identity, and repeated expression text can
refer to one producer connected to several consumers. Wildcard imports and empty rendered
branches alone do not establish a defect. Use exact graph facts, diagnostics or a behavioral
failure to justify a repair; avoid changes made only to restyle canonical serialization.

For atomic board edits, the host matches the receipt's graph fingerprint against a fresh storage
read. The fingerprint includes node and pin identities, so it can verify a replacement whose
canonical text stays the same. Missing or stale receipts leave the saved state unverified.

UI generation receipts distinguish staged output from a saved page. The host verifies the
saved identity and content through authoritative readback. Use `flowpilot_widget` with
`mode="inspect"` and exact app, board and page IDs to read persisted components, actions and
lifecycle bindings without another generation. Inspection does not verify rendered behavior.

```sh
bun run flowpilot:e2e -- --case intake-reliability --isolated --tier behavioral \
  --model terra --repeat 10 --output /tmp/intake-reliability.json
```

The native development runtime must attest the offline app, exact board snapshot and compiled
Page actions before execution. The fixture permits a bounded set of local routing, database and
UI nodes, restricts database access to `intake_tickets`, and rejects credentials, network nodes,
WASM and lifecycle execution. These constraints apply to this fixture, not arbitrary app builds.

The runner mounts the actual Page, enters five distinct summaries and triggers its submit
action. Each scenario requires a successful native run, exactly one new row with the expected
summary, queue and deadline, and the assigned queue visible on the Page. Cases cover interruption,
refund, general requests, overlapping conditions and case-insensitive matching. Unknown or
missing runtime evidence fails acceptance. The Page Event is restored after execution.

Artifacts retain provisioning readbacks and durations for provisioning, generation, registration,
collection and behavioral checks. Native and delegated tool traces remain separate because their
time windows overlap. The controller writes `<output>.progress.json` after each completed build;
the final envelope includes all requested runs, including failures. Keep pilot runs separate from
a repeated cohort and freeze code, prompts and acceptance checks during that cohort.

## Staged app-build preview

FlowPilot can keep an app contract and its build progress outside a model conversation. This is an opt-in preview for developers testing app construction. The staged app-build adapter cannot yet isolate workflows with external effects, so it cannot certify runtime behavior or promote a build. Ordinary app requests continue through the existing build playbook.

### System model

An `AppSpec` describes requirements, resources, dependencies, and acceptance scenarios. The compiler validates references and assigns stable physical identifiers within an app/build pair. Specialists receive the part they own and a shared symbol table. They generate artifacts; the host reads those artifacts back before recording an applied receipt.

The desired contract fingerprint and the observed artifact fingerprint have different jobs. The former identifies requested intent. The latter detects changes in persisted content. Resource existence, a model's completion message, and a successful tool response cannot substitute for readback evidence.

The [app-build modules](https://github.com/Rheosoph/flow-like/tree/dev/packages/ui/lib/app-build)
contain the contract compiler, resumable engine, resource readback, staging
lifecycle, and scenario runner.

The Rust store in `packages/core/runtime/src/app_build.rs` persists app-scoped JSON checkpoints. Web/API and desktop/Tauri adapters use the same revision contract: create revision zero, then compare-and-swap from revision `n` to `n + 1`. Cloud stores use conditional writes and fail explicitly when those are unsupported. The explicit local backend uses a cross-process file lock around the revision check and atomic file replacement, because the local object store does not implement conditional updates. Records have a 1 MiB limit. Missing state is distinct from read failure or corrupt state.

Build verification uses separate authoritative reads for app metadata, boards, FlowScript, Events, pages, widgets, table schemas, and routes. These methods never repair remote data, accept a cached fallback after a server error, or substitute a live artifact when a requested version is missing. Staging status writes use the same explicit local/hosted routing. Normal UI reads retain their availability-oriented behavior. Pure Event definitions and component-type metadata are separated from React renderers so host validation does not load the application UI.

Scenario checkpoints retain outcome counts, a result fingerprint, and bounded diagnostics. They do not copy raw app payloads into the build journal. Full result retention belongs to the caller or a trace exporter; a fingerprint is a change detector, not a signature.

### Build operations

Use one stable `app_id` and `build_id` throughout a build. Read `app_build(operation="schema")` first for the current contract and host capabilities. Capability recipes are pinned contract scaffolds with generation briefs, not precompiled or runtime-certified workflow modules.

1. `begin` validates the complete spec and persists intent before staging an eligible empty private app. A revision-zero retry repeats staging, covering interruption between those writes. The original request is bound separately from the model-authored spec.
2. `advance` processes dependency-ready resources with bounded parallelism. A durable operation lease prevents a second caller from replaying active work. Expired operations are inspected before retry; unexplained existing artifacts remain unknown.
3. `repair` invalidates selected logical resources and their dependents. It retains the original requirements and invalidates stale verification. The next specialist brief includes prior host diagnostics. Repair does not delete retired resources or authorize a smaller replacement app.
4. `validate` checks persisted artifacts, linkage, schemas, Event routes, and current staging state. Evidence is tied to the exact applied resource snapshot.
5. `test` runs bounded acceptance scenarios through a host adapter. Read-only observations receive contract-only certification. Runtime certification requires an attested isolated execution context, terminal successful run outcomes, and expected domain state read back by the host.
6. `promote` revalidates current resources and requires behavioral coverage for every requirement. The current interactive adapter cannot supply this evidence, so promotion stays blocked.

`status` reports resource states, retry counts, scenario outcomes, and readiness blockers after interruption. Do not create replacement IDs after a timeout. Unknown or partial promotion requires review of its subreceipts before recovery.

### Activation boundaries

An inactive app is a staging convention, not a runtime sandbox. Event sinks can begin work when an Event becomes active. Promotion therefore versions resources, pins Events while they remain inactive, verifies the pins, activates the app, and activates Events serially. It records attempted writes before awaiting their responses.

This sequence is not a cross-resource transaction. On failure, the host attempts to deactivate the app and disable planned Events, including writes whose responses were lost. `external_effects_may_have_occurred` remains true after any activation attempt. Compensation cannot undo an email, external API write, or workflow that has already started.

Other clients can still mutate or activate the same app between host checks. Conditional checkpoint writes protect build state; they do not provide a server-enforced lock over every board, page, table, and Event write. Post-write readback detects some races but cannot prevent their effects.

### Current integration limits

Runtime isolation requires capability enforcement at native-node invocation
boundaries and remote dispatch. Shadow storage alone does not isolate network
calls or native side effects. An adapter for data-write scenarios needs an
ephemeral namespace or equivalent verified isolation, cleanup receipts, and a
pinned-version endpoint that can test inactive resources without exposing live
Events. It must derive terminal outcomes from the backend lifecycle and read
domain state independently of workflow-provided success text.

App-shell provisioning is not a server-idempotent operation across object
storage, SQL, initial boards, and profile association. The creation journal
records returned IDs and retries incomplete profile registration, but a lost
initial creation response still has an unknown outcome.

Operation leases protect one build checkpoint. Resource mutation and promotion
still need server-side app/build ownership fencing. Long-running delegated work
uses leases aligned with its dispatch deadline, so abrupt process loss can delay
recovery until expiry. The host has no heartbeat-based lease renewal or
relinquishment mechanism.

Capability recipes remain contract scaffolds until they have been compiled and
executed against explicit runtime and catalog versions. Unit tests and structural
fixtures do not establish a model's app-building reliability. Rebaseline live
evaluations when the provider, model, reasoning settings, harness, or runtime
changes, and report behavioral results separately from compile/apply success.

Evaluation cohorts identify the declared harness contract and fixture, provider,
model, reasoning settings, and tier. They do not attest which source or runtime
binary executed; retain that context with any baseline.

### Local verification

Run the deterministic suites without invoking a model or creating live apps:

```sh
bunx --no-install vitest run packages/ui/lib/app-build packages/ui/components/global-chat/tools --maxWorkers=4
RUSTC_WRAPPER= cargo test -p flow-like-runtime --no-default-features --features app app_build::tests --lib
RUSTC_WRAPPER= cargo test -p flow-like-editor tool_spec::tests --lib
RUSTC_WRAPPER= cargo test -p flow-like-editor draft_test --lib
RUSTC_WRAPPER= cargo test -p flow-like-catalog --no-default-features --features draft-testing --lib draft_test::tests
RUSTC_WRAPPER= cargo test -p flow-like-catalog --no-default-features --features draft-testing --lib benchmark_case_tests
RUSTC_WRAPPER= cargo test -p flow-like-editor --lib behavioral_
RUSTC_WRAPPER= cargo test -p flow-like-desktop --bin flow-like-desktop --no-default-features workflow_benchmark
```

Run application typechecks and API/desktop checks separately. A filtered
typecheck can help diagnose a feature failure, but does not verify the whole
repository.
