# FlowBook: A Developer's Guide to Flow-Like

> Software that explains itself.

**Status:** Astro/Starlight manuscript app; first edition in drafting
**Working title:** *FlowBook: A Developer's Guide to Flow-Like*
**Working subtitle:** *Build reliable software as typed text and a visible workflow*

## Run the book locally

From the repository root:

```sh
bun run --cwd apps/book dev
```

Use `bun run --cwd apps/book check` for content and type diagnostics. The regular
`bun run --cwd apps/book build` copies the current `output/pdf/flowbook.pdf` into
`public/flowbook.pdf` before building the static site. Use
`bun run --cwd apps/book build:release` to regenerate the PDF first and leave a fully
deployable `dist` directory.

## Export the print edition

Generate the light, A4 print edition from the current manuscript:

```sh
bun run --cwd apps/book pdf
```

The command builds the dedicated `/print/` edition, calculates the contents-page
numbers from the finished body, adds running headers and page numbers, embeds the local
book fonts, writes `output/pdf/flowbook.pdf`, and publishes a generated copy to
`public/flowbook.pdf`. If `dist` is already current, use
`bun run --cwd apps/book pdf:from-dist` to run only the PDF stage.

Both deploy scripts run the full release build before Wrangler, so the hosted
`/flowbook.pdf` always comes from the manuscript being deployed.

The browser preview at `/print/` is intentionally light-only so it matches the printable
artifact rather than the dark documentation UI.

The published manuscript lives in `src/content/docs`. The introduction and first fifteen
chapters are drafted; the complete editorial plan and interview dependencies remain beside
the app.

## The thesis

FlowScript is Flow-Like's typed textual language for authoring Flows. It gives the same
program a compact code view and an editable visual workflow while keeping execution inside
the platform's governed node model. Experienced developers can work efficiently in text.
New builders, domain experts, reviewers, and operators can inspect the same logic as a graph.
When a run fails, its evidence leads back to the building block that failed.

The technically precise formulation used by this book is:

> The canvas editor and FlowScript editor inside Studio are equal authoring views over one Flow model.

FlowScript is an authoring language. Text is parsed and reconciled into the persisted Board
behind a Flow; the Flow-Like runtime executes that
graph. Immutable Flow versions can also be prepared as compact, versioned compiled-board
artifacts for the Rust executor.

## The reader promise

By the end of the main path, a reader will be able to:

- read and write useful FlowScript;
- move between text and graph without treating either as a generated afterthought;
- model types, state, branches, loops, functions, handlers, Events, data, and UI;
- run a Flow and trace a result or failure to its responsible nodes;
- review human- or AI-authored changes through the same guarded apply path;
- choose when to use the catalog and when to build a capability-scoped WASM node;
- understand how Flow-Like packages, governs, versions, and runs the resulting App; and
- distinguish current implementation guarantees from deployment policy and product
  vision.

## Who the book is for

FlowBook has one progressive core. Optional deep dives let readers leave and rejoin that path.

| Reader | Primary route |
| --- | --- |
| Domain expert or startup founder | Chapters 1–13, then 17–20 and 23; Chapters 14–16 are optional implementation mechanics |
| Enterprise or experienced developer | Full main path, especially Parts II, III, and VI |
| Node/package author | Parts I–III, then Chapters 21–23 and the internals chapter |
| Architect, operator, or security reviewer | Part I, Chapters 14–17, and Parts V–VI |
| Student or AI-assisted builder | Parts I–IV with every exercise and visual comparison |

The prose assumes basic programming familiarity but not professional software-engineering
experience. TypeScript knowledge makes the syntax familiar; it is not required.

## What this book covers

FlowBook teaches durable mental models through a narrative and worked applications. The product
manual, generated node catalog, and cloud runbooks remain the detailed reference.

- The book explains why a construct exists, how it reads, what graph it becomes, and how
  it behaves during a run.
- The documentation site remains the detailed operational and node reference.
- Generated `.flow.d` declarations remain the exact, version-matched source for callable
  node signatures.
- Volatile capabilities are labelled **Current**, **Preview**, or **Vision** instead of
  being blended into one promise.
- Security, performance, scale, compliance, and cost claims require implementation
  evidence, measurements, or a named case study.

## The two worked applications

### Incident Triage

The first tutorial uses a deterministic support/incident flow: receive a report, normalize
it, classify its severity, branch, and log or return the result. It needs no model, account,
API key, or external system. Readers can change one condition in FlowScript, see the graph
change, edit the graph, see the source change, and trace a deliberately failing run.

### Incident Room

The capstone grows the founding story into a private document assistant: upload runbooks,
index and search them, expose agent tools, answer through a chat or page, and inspect every
run. Its implementation will be distilled from the existing agentic document application
fixture rather than invented independently.

The `sales-insights` WASM and widget example becomes a later extension case study.

## Editorial covenant

Every chapter follows these rules:

1. **Show the outcome first.** Readers see a useful result before its machinery.
2. **Show both faces.** Every meaningful FlowScript example is paired with its graph or a
   precise graph sketch.
3. **Explain the lowering.** Familiar syntax is connected to the nodes, pins, and execution
   behavior it represents.
4. **Run the failure path.** Examples include observable failures, not only happy-path
   screenshots.
5. **Keep examples real.** Book fixtures should parse, reconcile, and where practical run in
   automated checks against the matching Flow-Like version.
6. **Separate fact from intent.** Source-backed behavior, founder rationale, case-study
   evidence, and future direction are labelled distinctly.
7. **Use candid language.** Constraints and unsupported edges are part of the design story,
   not footnotes to hide.
8. **Prefer concepts over catalog tours.** The catalog changes; the principles behind typed,
   visible, governable building blocks endure.

## Render workflow screenshots

Book diagrams can be generated directly from the checked-in `.flow` examples with the real
reconciler, Studio auto-layout, and desktop workflow canvas:

```sh
bun run workflow:screenshot -- \
  apps/book/examples/incident-triage/triage.flow \
  --output apps/book/src/assets/workflows/incident-triage.webp \
  --layout balanced \
  --theme light
```

Use `--list-nodes` to inspect reconciled IDs, then `--focus-node <selector>` for a
zoomed detail. Keep exploratory captures under `tmp/workflow-screenshots`; committed book
art belongs under `apps/book/src/assets/workflows`.

## Editorial files

- [STRUCTURE.md](STRUCTURE.md): parts, chapters, subchapters, summaries, evidence, and
  interview dependencies
- [INTERVIEWS.md](INTERVIEWS.md): completed source interviews, future sessions, and the
  open fact-check queue
- [SOURCE_MAP.md](SOURCE_MAP.md): repository evidence and implementation caveats to use
  while drafting
