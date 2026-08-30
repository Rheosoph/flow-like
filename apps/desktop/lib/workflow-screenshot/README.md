# Workflow screenshot CLI

This command turns any catalog-valid FlowScript document into the real rendered Studio
workflow and writes a deterministic screenshot. It is intended for FlowBook illustrations,
documentation, visual regression fixtures, and other automated publishing workflows.

Run it from the repository root:

```sh
bun run workflow:screenshot -- \
  apps/book/examples/incident-triage/triage.flow \
  --output apps/book/src/assets/workflows/incident-triage.webp \
  --layout balanced \
  --theme light
```

The pipeline uses the production pieces in their normal order:

1. The Rust helper applies the source to an empty Board through
   `apply_flowscript_to_board` and the complete built-in catalog. Parse or reconcile
   diagnostics stop the command before a browser starts.
2. The resulting Board is formatted with the same `computeFlowLayoutDetailed` engine used
   by Studio. Root and nested/function-layer canvases are all laid out.
3. An ephemeral offline app and Board are exposed to the desktop frontend through the
   documentation Tauri fixture bridge. No real profile, app, or Board is created or changed.
4. The existing documentation screenshot runner opens `/flow` in Chromium and writes a
   lossless WebP/PNG (or JPEG) at the requested viewport and DPR.

## Focus one node or layer

`--focus-node` accepts an exact reconciled ID, a node/layer identity anchor such as
`//@n:abc123` or `//@l:function123`, a unique catalog node name, a friendly name, or a
layer name. The normal `/flow?...&node=<id>` navigation opens the owning layer and frames
the target with Studio's focus behavior.

Generated ids are easiest to discover with:

```sh
bun run workflow:screenshot -- path/to/workflow.flow --list-nodes
```

Then render the detail:

```sh
bun run workflow:screenshot -- path/to/workflow.flow \
  --focus-node normalize \
  --output tmp/workflow-screenshots/normalize.webp
```

An ambiguous selector fails and prints the matching ids instead of silently choosing one.
When a document contains only function declarations, the renderer automatically opens the first
function by stable name/id order so the root's intentionally hidden function layers cannot produce
an empty capture.

## Show generic error handling

`--handle-errors` adds the same `On Error` Execution output and `Error` String output as
Studio's Handle Errors toggle. It accepts the same node ids, node anchors, catalog names, and
friendly names as `--focus-node`; layers and pure nodes are rejected. The adjusted node is focused
automatically unless `--focus-node` explicitly selects another target.

```sh
bun run workflow:screenshot -- path/to/workflow.flow \
  --handle-errors "API Call" \
  --output tmp/workflow-screenshots/api-error.webp
```

The outputs are added only to the ephemeral reconciled Board used for rendering. The FlowScript
source and any real Flow-Like profile remain unchanged.

## Layout and image controls

- `--layout compact|balanced|expanded` selects Studio's layout style. `balanced` is the
  default for book-friendly spacing.
- `--viewport 1624x1060`, `--dpr 2`, and `--theme light|dark` control the deterministic
  browser surface.
- `.webp` and `.png` are lossless. `.jpg`/`.jpeg` can use `--quality`.
- `--frontend-url http://127.0.0.1:3000` reuses an already running desktop frontend.
- `--json` returns the screenshot hash, dimensions, resolved focus id, and nested capture
  result on stdout. Progress and server logs stay on stderr.

The default output is `tmp/workflow-screenshots/<input-name>.webp`.

For repeated captures after building the helper once, set
`FLOW_LIKE_FLOWSCRIPT_RENDER_DATA_BIN=target/debug/flowscript-render-data` to bypass Cargo's
workspace lock and invoke that exact binary directly.
