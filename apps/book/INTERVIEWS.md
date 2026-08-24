# FlowBook interview ledger

The interviews are primary sources for motive, philosophy, historical decisions, and case
studies. The repository is the primary source for current implementation behavior. Neither
source replaces the other.

Answers recorded here are **lightly normalized editorial notes**, not approved verbatim
quotations. Any sentence presented in quotation marks in the published book must be returned
to the speaker for wording approval.

## Interview plan

| ID | Session | Status | Main chapters |
| --- | --- | --- | --- |
| INT-01 | Origin and manifesto | Complete | 1, 2, Epilogue |
| INT-02 | Audience, platform, security, deployment, and AI | Complete, fact-check follow-up required | 2, 3, 20, 22–25 |
| INT-03 | Language design and the two-view contract | Complete; code-verified notes added | 5–16, 26 |
| INT-04 | Runtime reliability, incidents, execution, and observability | In progress; failure, evidence, and rerun answers code-checked | 4, 5, 10, 15, 17, 24 |
| INT-05 | WASM threat model and the no-inline-code decision | Planned | 11, 21, 22 |
| INT-06 | Governance, packages, scores, and publication | Planned | 2, 23 |
| INT-07 | Deployment support and enterprise operating model | Planned | 3, 13, 25 |
| INT-08 | Data Studio, document-agent capstone, and AI authorship | Planned | 12, 18–20 |
| INT-09 | Tradeoffs, rejected shortcuts, limits, contribution, and future | Planned | 2, 26, Epilogue |

## INT-01 — Origin and manifesto

### Editorial record

FlowScript combines a mostly TypeScript-familiar surface with selected ideas from Rust and
Python. Every script has a visible workflow representation. Text is efficient for experienced
developers and large projects; the graph lowers the entry barrier and improves debugging and
tracing.

The broader platform was shaped by enterprise reliability requirements. Nodes are approved
and inspectable building blocks, extensions use WASM, execution is traceable, and deployment
concerns are supplied by Flow-Like so builders can focus on the domain problem.

The origin was a major-incident call in enterprise IT. The future founder did not know the
systems or interfaces involved, and the group waited hours in the middle of the night for
someone with the missing system knowledge. Each hour of downtime carried potentially
millions in damage. The experience raised a foundational question: why did a critical system
not document and reveal itself well enough for an unfamiliar responder to find the failure?

The missing person was the system expert. The idea did not emerge as a complete product plan
during the call; its significance crystallized later. The present-day feature that most
directly answers the incident is node-attributed run evidence: the logs identify the relevant
building block, and Studio can focus it from that evidence with one click.

Nothing technical was visible to the responders on the call. The only concrete information
came from the domain experts, who reported that production was on hold. The team therefore knew
the business consequence while lacking a visible path to its technical cause.

Unreal Engine Blueprints supplied another influence: a visual graph can be a genuine
programming surface and an approachable entry point. Enterprise teams added the third piece:
business-oriented developers and domain experts often understand the process better than
specialist programmers. A shared textual and visual representation can let those groups work
together at much higher speed.

The team rejected arbitrary code embedded inside workflow blocks. In its view, that pattern
creates opaque “Frankenstein applications” that are harder to maintain than conventional
software. A useful platform still needs low-level building blocks, which can make visual
workflows large. A language was therefore necessary: it had to express the same constrained
building blocks in a form that remains manageable for large-scale authoring.

### Manifesto statements supplied by the founder

- Software should be reliable and efficient.
- A Flow should be organized and readable.
- FlowScript should make hard things fast to realize and easy to reason about.
- FlowScript should make dangerous things impossible to do wrong accidentally.
- The design deliberately trades some freedom of choice for a more opinionated, robust, and
  scalable approach while preserving broad application scope.
- The team will not accept a shortcut when it makes the final product worse.

### Scene detail supplied

The opening can safely state that nothing technical was visible to the call participants. The
domain experts could report only the operational consequence: production was on hold.

## INT-02 — Audience, platform, security, deployment, and AI

### Editorial record

The intended audience is deliberately broad: enterprise developers; startup founders without
deep technical backgrounds; technical enterprise users; domain experts; students; AI and
non-AI builders; and experienced developers who extend the platform with WASM nodes and help
shape scalable solutions.

Flow-Like is the complete platform. FlowScript and Boards are the textual and visual logic
surfaces. The platform also supplies web, desktop, and mobile clients; on-device execution;
project and permission management; authentication; automatic documentation and governance;
Data Studio for ontologies and data; API exposure; storage; the Rust runtime; and multiple
deployment shapes.

The two authoring views are intended to be peers. The visual workflow existed first with the
textual projection already in mind. The founder considers current FlowScript authoring usable
and close to feature parity, while acknowledging remaining convenience and round-trip gaps.

Custom nodes execute through WASM and declare capabilities such as data, network, and storage
access. A package must be added to an App before its nodes are available. Private packages are
restricted to members; public publication requires platform review; public request-access
packages also let the owner control membership. Interactive users are informed about WASM
packages and permissions before execution.

Apps have a related visibility/governance path: offline, private, bounded prototype, public,
and public request-access. The intended model lets experimentation remain easy while widening
access triggers stronger review. Because the platform owns the authoring, runtime, logs,
models, data, and permissions, much documentation and governance evidence can be derived
automatically.

The deployment thesis is that every Flow can use the same performance-oriented execution
contract rather than forcing every application team to rediscover authentication, logging,
auditing, storage, scaling, and release infrastructure. Different environments can map this
contract onto provider-appropriate components.

AI is an important accelerant but not the sole reason for Flow-Like. Modern agents can create
prototypes quickly; far fewer people can make them reliable, understandable, scalable, and
cost-efficient. Flow-Like narrows generation to typed, traceable building blocks and gives
agents access to the resulting declarations, validation, and logs. The founder also sees many
applications as variations that should be parametrized and forked rather than regenerated
from nothing.

### Follow-up prompts

1. Rank the audiences for the first edition. Who must be delighted even if another audience
   occasionally skips a deep dive?
2. What should a non-programmer be able to build alone after the main path?
3. What should require an experienced developer or operator?
4. Which platform features are stable, preview, private, or roadmap today?
5. What is the intended non-interactive WASM policy for API and scheduled executions?
6. Which deployment/cost statements have measured evidence or a named customer case study?
7. What is the best example of a parametrized App replacing repeated bespoke builds?

## INT-03 — Language design and the two-view contract

### Editorial record

The working definition supplied in the interview was close but needed a sharper boundary. The
book will use:

> FlowScript is Flow-Like's typed textual language for authoring Flows. It gives the same
> program a compact code view and an editable visual workflow while keeping execution inside
> the platform's governed node model.

“Language” matters because the text is an intentional authoring surface with its own readable
syntax and semantics, not a passive serialization format. “Authoring” matters just as much:
FlowScript does not introduce a second execution engine. Source is parsed and reconciled with
the Board model, and Flow-Like runs the resulting Flow.

The language should be described as **TypeScript-familiar**, not as a percentage blend that
implies exact ancestry. Its braces, declarations, calls, object-shaped arguments, interfaces,
operators, and control-flow forms make TypeScript the dominant visual reference. The clearest
Rust-derived syntax named by the founder is `use` with `::`-qualified paths.

The founder suggested the `@secret` family as a possible Python influence. Editorially, that
is too weak to present as uniquely Python-derived: TypeScript also uses `@` decorators, while
other languages have attributes or annotations with similar intent. The accurate claim is
that FlowScript uses decorator-shaped metadata such as `@secret`, `@runtime`, `@readonly`,
`@description`, `@category`, and `@cache`. We should only claim a broader Python lineage if a
later design history supplies a concrete feature or principle.

The two views have equal authoring authority over one model; they are not separately persisted
sources that can silently diverge. FlowScript is intentionally platform-constrained and is not
intended to become a standalone runtime. The graph and Flow-Like runtime are part of its
meaning, not deployment options bolted on later.

The founder does not currently identify a specific list of important parity gaps. Readers may
encounter unsupported edges as the language evolves and should report them. The manuscript
must therefore avoid inventing a “remaining 1%” checklist or implying that a known gap is a
design commitment.

### Code-verified global-variable semantics

Top-level FlowScript declarations represent typed Board variables shared by nodes in one Flow;
each run receives its own mutable in-memory variable state. The initializer is the persisted
default for a new run, not durable cross-run state. Assignments affect that run only; durable
state belongs in storage, files, or Data Studio. Their syntax deliberately differs from
JavaScript semantics:

- top-level `const` maps to a non-exposed Board variable;
- top-level `let` maps to an exposed Board variable, which can appear in App configuration and
  accept compatible invocation or Event values;
- neither keyword makes the runtime value immutable—`@readonly` sets the variable's
  user-editable metadata to false but does not prevent a Flow from assigning the value during
  its run;
- `@runtime` marks a value as configured per user at runtime, and `@secret` marks it for
  sensitive handling and redaction; and
- function-local `const` bindings to node outputs and mutable `let` aliases/accumulators are a
  separate scope with different authoring semantics.

This distinction needs an explicit warning in the variables chapter. It is both a useful
example of FlowScript serving the graph model and a likely trap for readers arriving from
TypeScript.

### Type-safety doctrine for Chapter 7

FlowScript should reject a mismatch at the earliest point where the Flow and its active catalog
know enough to prove it. If a producer is now an integer and a consumer requires a string, a
known contract mismatch should block reconciliation or connection rather than waiting for a
run. The platform cannot statically know every value returned by a changing external system.
When an undeclared change reaches runtime, the failure should remain transparent: attribute it
to the responsible operation, show the actual error, and let the author jump directly to the
node where the value or conversion failed.

`any`, Generic values, and schema-less Structs remain necessary escape hatches, but they should
not be the easiest way to work. The intended penalty is primarily structural and ergonomic:
typed connections can expose named fields through **Break Struct** and **Make Struct (Schema)**,
while untyped structures require more path-based inspection and mutation through operations such
as **Get Field** and **Set Field**. The language need not ban dynamic data to make precise types
the naturally faster and safer choice.

Schema evolution follows the same rule rather than a separate ideology. Catch incompatible
known fields, types, containers, and enforced schemas during authoring. If an external producer
changes without publishing a usable contract, surface the resulting runtime problem at its
responsible node instead of pretending the platform could have predicted it. The design promise
is early rejection where knowledge exists and fast, local diagnosis where it does not.

The current implementation does not yet realize that doctrine uniformly. Typed new wires and
anchored boundary contracts receive strong reconciliation checks, while direct literal arguments
are not all validated against their destination types during Apply. Unresolved named types can
also fall back to schema-less Structs. These are release gaps for the manuscript to name, not
exceptions to the intended earliest-knowable rule.

### Catalog discovery and upgrade doctrine for Chapter 8

The catalog should not be a memory test. On the Board, beginning from a pin narrows the explorer
to compatible nodes and search narrows it by intent. In FlowScript, completion should expose the
nodes available for the value or receiver type. The two authoring surfaces therefore answer the
same question in their native form: “What can I safely do with this value?”

When several compatible nodes can perform the job, ranking is desirable. The intended direction
is to make the safer or otherwise better fit easier to select rather than returning an arbitrary
wall of equally weighted results. The implementation audit must establish which trust,
permission, performance, cost, package, or governance signals are actually available to the
current picker and language service before the manuscript labels ranking as Current. Until then,
ranking is product direction, not an existing guarantee.

Catalog evolution follows the same preserve-and-explain philosophy as schema evolution. A node
upgrade should migrate compatible changes automatically. When a change cannot be migrated safely,
the node should remain on the Board with an error annotation so the author can see and repair the
broken boundary; an upgrade must not silently erase the operation that needs attention.

Current catalog synchronization only partly satisfies that contract. It preserves the node and
same-named compatible pins, adds new pins, and retains wires when a type widens to Generic. For an
ordinary removed or renamed pin, however, it currently removes the pin; for an incompatible type
change it clears connections and resets the default, then clears the prior node error. Dynamic
schema-derived pins have stronger stale-wire protection. The manuscript must present uniform
preserve-and-annotate behavior as the desired contract and name static-node migration as a gap.

### Expressions and lossless-sugar doctrine for Chapter 9

Operator syntax must not choose silently between materially different meanings. For example,
`"5" + 1` could mean numeric addition producing `6` or text concatenation producing `"51"`.
Editorially, the safer contract is a diagnostic with explicit repair choices rather than an
automatically chosen Try Convert node. An editor may offer “convert left to int” and “convert right
to string” actions, but a fallible conversion and its failure behavior must remain visible in the
applied Flow and its rewritten source. An `any` operand likewise needs narrowing or an explicit
conversion before an overloaded operator can have one reliable meaning.

Readable sugar is permitted only when it preserves the complete node contract. If an operator-like
node has additional non-default configuration that the short syntax cannot express, the renderer
should keep the explicit catalog call and show those inputs. Familiar notation is a view of the
graph, not permission to discard configuration for prettier source.

Struct field assignment is temporal rebinding over explicit node values. `incident.status =
"closed"` creates or selects the updated Struct value for subsequent uses of `incident`. Consumers
already wired before that assignment retain the earlier value; later references use the rebound
value. If there is no later use, the updated value is simply an unused copy. Runtime pins carry the
current value of their operation, while the source variable name describes which producer later
expressions resolve to.

The current implementation already rejects a known String/Integer operator mismatch and applies
no partial Board commands when reconciliation reports it. It does not yet provide the proposed
intent-specific conversion quick fixes. The relevant generic catalog operation is named **Try
Transform**, not Try Convert. Its target is shaped by a typed consumer; failure produces `null` and
`success = false` rather than a node error, so a repair must keep and handle that result. Prefer a
specific typed parser such as String to Integer when the desired target is already known.

Current Board rendering preserves an explicit operator call when extra inputs are wired or carry
edited nonzero configuration. The audit also found release gaps that must not be mistaken for
doctrine: Float equality and inequality can currently lower to operators even though text
reconciliation and existing-source reuse refuse that shorthand because tolerance matters;
Integer division's operator result metadata conflicts with the live node; and Get Field lowering
can confuse its Boolean `found` output with the selected field value. Types Select has exactly the
three inputs represented by a ternary today, but its renderer is not yet guarded against meaningful
inputs added in a future node version.

### Later fact-checks, not blockers for the language interview

1. Which syntax and round-trip compatibility promise should be made across releases?
2. Does the team want to identify a concrete Python-derived feature, or remove Python from the
   short lineage description?
3. **Resolved as doctrine:** reject every mismatch the known pin, container, or schema contract
   can prove; diagnose unknown external drift at the responsible runtime node. A release-specific
   compatibility matrix remains an implementation fact-check.
4. **Resolved as doctrine:** discover catalog operations by compatibility and search in the graph,
   and by type-aware completion in FlowScript; rank good candidates when trustworthy signals are
   available; migrate node upgrades safely and retain visible error nodes when they are not.
5. **Resolved editorial recommendation:** ambiguous mixed-type operators fail with explicit
   conversion repairs rather than silently choosing semantics; all sugar must preserve the full
   node configuration, and field assignment rebinds later uses to an updated Struct value.

## INT-04 — Runtime reliability, incidents, execution, and observability

### Editorial record

Failure behavior is part of a Flow's design, not one platform-wide switch. A straight
execution chain expresses sequence. When a node makes several successors ready, the runtime
may advance those branches concurrently; an unhandled failure ends that branch but does not
cancel its siblings. The ordinary executor records the aggregate run as failed after the
remaining branches finish. Authors can also use dedicated sequence, parallel, loop, and gather
operations when that intent should be unmistakable.

The implementation currently needs one qualification: the dedicated Parallel and Sequence
control nodes catch or discard some errors from the child chains they trigger. Those errors can
remain visible in node-attributed evidence without consistently failing the outer run. The book
should teach the intended model while marking uniform aggregate status across these control
nodes as a release fact-check, not a settled guarantee.

There are two different kinds of failure path. A node author can model an expected negative
outcome as an ordinary execution output. The built-in HTTP API Call, for example, selects its
Error output for a completed non-2xx response and its Success output for a 2xx response. The
node itself has completed its contract; the Flow decides whether to retry, create a ticket,
return a domain error, or rejoin the main path. Transport and response-reading failures are
unexpected node errors rather than HTTP Error-arm outcomes.

For unexpected errors on an executable node—including a node whose normal contract already
has an Error arm—Studio's **Handle Errors** option adds an **On Error** execution output and an
**Error** string output. When the recovery chain completes, the runtime treats the unexpected
node error as handled while leaving the original node in its Error state for that execution.
Persisted Error messages retain their node attribution afterward. Handling an error therefore
does not erase the evidence that the operation failed. An unwired handler or a failing recovery
chain allows the error to propagate. Terminal execution status and highest logged severity
remain separate signals: handled Error evidence can coexist with a run that the Core executor
did not mark failed.

The founder's desired node evidence includes duration, the error message, and any additional
diagnostic information the node author chooses to expose. The current persisted contract is
more precise: a log entry has a message, severity, and timestamps, with optional node,
operation, and usage fields. The normal execution path attributes messages to their node, so
the interface can mark that block, open its filtered logs, or focus it from a log line. At the
Debug log level, the runtime adds a timed execution record for each node; total run timing is
stored separately. The runtime does **not** automatically persist every input, output, pin
value, or in-memory Debug variable snapshot; selected values appear only when a node
deliberately emits them.

Log content is an author and organization policy decision. Log-level configuration controls
volume and permissions control readers. Desktop can automatically remove completed local run
logs under a configurable retention policy; hosted retention remains an operator and
organization concern. Secret-variable access avoids printing the secret value, but there is
no universal redaction pass over arbitrary author-written log messages. Authors must expose
enough context to diagnose an operation without copying credentials or restricted data into
the evidence.

The founder defines **Re-Run** as reusing a run's recorded invocation payload against the system
as it exists now. Logic is expected to evolve. The useful question is therefore, “How does
today's Flow handle yesterday's case?” rather than, “Can the platform reconstruct every
dependency exactly as it was?” A local Runs-panel Re-Run already follows that shape: it reuses
the stored payload, omits a Board version, and therefore executes against Latest.
Runtime-variable values are a separate invocation field and are resolved again rather than
archived in the local run metadata.

This is deliberately different from replay. Reconstructing a historical universe would mean
snapshotting Board logic, packages, runtime values, profiles, credentials, databases, files,
external responses, and model behavior. At application scale, that would duplicate enormous
amounts of data without making nondeterministic or external systems truly reproducible. The
platform should retain the evidence and versions needed for investigation, while allowing
operators to clean up history under an explicit retention policy—not silently turn every run
into a permanent copy of its world.

Stored data has its own version and retention model. In the inspected desktop implementation,
native Data Studio tables are backed by LanceDB, whose mutations create storage-level dataset
versions. Optimization can prune that history before compaction; even the current **Keep
Versions** path follows the engine's bounded retention behavior rather than promising permanent
history. The VectorStore interface does not expose arbitrary version checkout. Separately,
supported Delta and Iceberg lake formats provide explicit time-travel operations. These are
deliberate data-lifecycle capabilities, not database snapshots bound to every run. Version
history consumes storage, so teams should retain what their recovery, audit, and compliance
needs require and periodically optimize or prune the rest.

The book will use this formulation:

> Re-Run asks today's Flow to process a recorded invocation payload again. It is a regression
> tool, not a time machine. If an investigation needs old logic, invoke an immutable numbered
> Board version deliberately; the surrounding packages, configuration, data, external systems,
> and model behavior are still resolved under their own lifecycles.

There is also a current remote gap: asynchronous execution can store input payloads encrypted,
but the remote Runs response returns an empty payload and the interface has no retrieval path
for it. That prevents the remote path from meeting even the intentionally narrow payload-only
Re-Run contract today. A pinned Board preserves an explicit authored model identifier, while
profile/default model selection and model responses are resolved again. It also freezes
authored graph and configuration, not necessarily the implementation of its catalog nodes
across a platform upgrade.

The intended aggregate failure rule is now explicit: an unhandled failure in any sequential
or parallel child path makes the aggregate terminal status Failed after sibling work has
settled. A failure routed through a completed handler remains visible on the responsible node
but does not fail the run. The current Parallel and Sequence control-node implementations do not always
propagate child errors into the enclosing status, so this is a required invariant rather than
a claim that every current path already conforms.

No customer incident will be invented for the manuscript. Until an approved real case exists,
the book will return to the original 3 a.m. incident as a clearly labeled counterfactual:
node-attributed evidence describes what would have changed, not an outcome Flow-Like historically
delivered on that call.

### Remaining questions

- **Q1:** What runtime invariant matters most when a Flow has hundreds or thousands of nodes?
- **Q2:** Describe pure-node evaluation and impure execution order in the language you want
  readers to remember.
- **Q5:** Which identifiers should connect an incident, run, caller, Board/Event version, and
  node? The internal Trace ID is not persisted today; which run, correlation, operation, and
  node identifiers should form the supported chain?
- **Q7:** What problem motivated the compiled-board artifact? Provide representative size and
  preparation/execution measurements with version and hardware.
- **Q8:** Which cancellation, timeout, retry, idempotency, and backpressure patterns should
  every production chapter teach?
- **Q10:** Which observability features answer the original 3 a.m. incident today, and which
  remain aspirational?

### Product decisions exposed by the code check

1. **Resolved:** Re-Run reuses the recorded invocation payload against today's Flow; it is not
   historical replay.
2. **Resolved:** the run record should not snapshot packages, configuration, databases,
   external responses, or model behavior merely to support Re-Run.
3. **Resolved:** any unhandled child error must make the aggregate run fail after siblings
   settle; a successfully handled error remains visible but does not fail the run.
4. **Implementation follow-up:** remote run history needs a permission-checked path to recover
   the stored payload before it can support the same payload-only Re-Run contract as local runs.

## INT-05 — WASM threat model and the no-inline-code decision

### Questions

1. Give the strongest version of the argument against JavaScript/Python code blocks inside a
   workflow.
2. What legitimate use cases must a custom node support without becoming an opaque sub-app?
3. Define the threat model: accidental bugs, runaway CPU/memory, malicious packages, data
   exfiltration, supply-chain replacement, expensive model/network use, and host compromise.
4. Which boundaries are enforced in every execution mode today?
5. Are Wasmtime memory/table/instance limiters wired for both component and core-module paths?
6. How should the book describe the component CLI fallback and inherited process
   environment/stdio?
7. What policy governs scheduled, API, internal, and agent-initiated runs where an interactive
   consent dialog may not be present?
8. Should remembered trust be keyed by package version rather than package ID?
9. What must a package author test before publication? What must an operator enforce outside
   the WASM runtime?
10. Which security claims are you comfortable putting in a book without qualifiers?

## INT-06 — Governance, packages, scores, and publication

### Questions

1. Are node scores “higher is better” or “higher is greater risk/impact”? Current source
   comments and board aggregation conflict.
2. Who assigns each score, what evidence supports it, and can an organization override it?
3. How should node scores aggregate into a Flow/App score when one weak node determines the
   boundary?
4. Which governance documentation is derived automatically today?
5. Which evaluations are rules, heuristics, model judgments, or human review?
6. What is the rationale for private, prototype, public, and request-access states?
7. Why can private packages activate without platform review, and what responsibilities stay
   with the private owner?
8. What guarantees does a public package review provide—and explicitly not provide?
9. Which compliance frameworks should the book avoid naming until verified?
10. Tell one example where the governance model prevented duplicate or unsafe work.

## INT-07 — Deployment support and enterprise operating model

### Questions

1. Which targets can the book call production-supported today: local, Compose, Kubernetes,
   AWS, Azure, GCP, Cloudflare, StackIT, and mobile/on-device?
2. Is the StackIT work private/external, in another repository, deployed for a customer, or
   roadmap?
3. Is Cloudflare a full runtime target, an R2/storage and frontend target, or both?
4. What turnkey provisioning exists for AWS, Azure, and GCP beyond checked-in service code?
5. Which execution backends are proven end to end, and which are extension points?
6. Provide the workload, region, services, and measurements behind “AWS is cheapest” and
   “Azure is most expensive,” or recast these as experience rather than universal fact.
7. Explain the “one to three cloud rooms for 10,000 Apps” model. What is a cloud room, and what
   isolation, blast-radius, quota, and noisy-neighbor controls make it safe?
8. How are dev/integration/production promotion, canaries, rollback, secrets, and audit handled
   consistently across targets?
9. What should a regulated operator still configure and validate outside Flow-Like?
10. Give one representative deployment profile with actual App/run scale and operating effort.

## INT-08 — Data Studio, capstone, and AI authorship

### Questions

1. Walk through the existing upload-and-chat App as a user, not as its author.
2. Which FlowScript fixture is its canonical source, and which pieces should be simplified for
   the book?
3. What data is stored for each file, chunk, embedding, source, and conversation?
4. Which parts work fully offline with local models today?
5. What makes this one App rather than a collection of external services?
6. Which Data Studio features are stable enough for hands-on book exercises?
7. What measured data size and concurrency have been tested?
8. How do ontology actions become governed tools, and when is an ontology unnecessary?
9. Tell one successful AI-authored Flow story from request through runtime verification.
10. Tell one failed generation story and which constraint or diagnostic made it recoverable.
11. What is the ideal library of parametrized/forkable Apps five years from now?
12. What should AI never be allowed to approve on its own?

## INT-09 — Tradeoffs, limits, contribution, and future

### Questions

1. Name three shortcuts the team rejected because the final product would have been worse.
2. Name three constraints you would relax if the platform could preserve its guarantees.
3. What applications should not be built in Flow-Like?
4. Where does the visual representation become less useful, even with FlowScript available?
5. What breaking language or graph change taught the team the most?
6. What compatibility promise do package authors and FlowScript authors deserve?
7. Which current implementation gap most threatens the “two equal views” claim?
8. How should contributors decide whether new convenience syntax is honest?
9. What would make FlowScript successful even if Flow-Like never became the largest platform?
10. Return to the 3 a.m. call: what would the ideal responder see and do in the first five
    minutes?

## Fact-check queue discovered in the repository

These issues must be resolved before the relevant prose is drafted as fact:

1. **Governance score direction.** `NodeScores` comments describe higher values as higher
   risk/impact, while board aggregation treats higher values as better and flags low values.
2. **WASM memory enforcement.** Manifest tiers exist, but complete Wasmtime limiter wiring
   needs verification across runtime paths.
3. **WASM non-interactive consent.** Interactive prompts are implemented; universal policy for
   schedules/APIs is not established by that UI behavior.
4. **Component runtime caveats.** Current documentation notes inherited executor
   environment/stdio and a fallback path outside ordinary fuel/epoch/memory controls.
5. **Manifest version.** documentation describes manifest version 1 while code/examples use
   newer shapes; the book must target and test one version.
6. **Deployment maturity.** service code exists for several clouds, but turnkey and
   production-supported status is not uniform. StackIT is not present in this checkout;
   Cloudflare evidence is narrower than a full backend.
7. **Execution backend gaps.** Kubernetes job dispatch is represented, but the checked-in
   job-once runner is documented as incomplete.
8. **Scale and cost.** provider cost rankings, “10,000 Apps,” and large-data claims require
   measured case studies.
9. **Audit guarantees.** configurable hash/signing code exists, but current failure,
   signature-verification, and permission behavior should not be called compliance-grade
   until closed and tested.
10. **Mobile and embedded status.** generated mobile targets and shared UI exist; the book
    needs product-status language rather than inferring maturity from directories.
11. **FlowScript limitations.** current gaps include unsupported array destructuring, schema
    forms not expressible as interfaces, selected round-trip edge cases, and dynamic-contract
    limitations.
12. **Licensing language.** use the repository's current source-available/BSL terminology
    unless the project license changes.
