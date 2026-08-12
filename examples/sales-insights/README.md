# Sales Insights — example package

A complete Flow-Like package that ships **Rust WASM nodes and React micro widgets side by side**. It exists to demonstrate the three things a flow does with a package widget:

| Concept | What to look at |
|---|---|
| **Instantiate** | `Instantiate Widget` picks `Sales Chart`; the contract's inputs (`title`, `variant`, `rows`, `currency`, `highlight`) become typed `dyn_in_*` pins with defaults, an enum dropdown and a JSON schema for `rows`. |
| **Update** | `Update Widget Inputs` sends a typed props patch to the *live* instance — swap `rows`, retitle the chart, move `highlight` — and the iframe re-renders without being recreated. |
| **Read** | `Query Widget` calls the contract's queries (`getSelection`, `getSeries`, `getTotal`, `getValue`) and gets a typed value back from the running widget. |

Everything renders inside an opaque-origin sandboxed iframe, so the widgets need no sandbox permissions at all.

## Layout

```
sales-insights/
├── flow-like.toml            # manifest v2: wasm_path + widget_bundle_path
├── mise.toml                 # build / bundle / validate / dev orchestration
├── node/                     # Rust WASM nodes (own cargo workspace)
│   └── src/lib.rs            # sales_demo_data · apply_sales_filter · sales_summary
├── widgets/react/            # one Vite app = one framework group
│   └── src/widgets/
│       ├── sales-chart/      # display widget: updated by the flow, queried by the flow
│       └── filter-panel/     # input widget: emits events, mirrors its value
├── node.wasm                 # GENERATED artifact
└── widgets.flwb              # GENERATED artifact
```

## Build

```bash
mise run build      # node.wasm + widgets.flwb, both staged at the project root
mise run validate   # contracts + packed bundle
mise run test       # native unit tests for the Rust nodes
mise run dev        # mock-host harness with props panel, event log, query invoker
```

Under the hood: `cargo build --release --target wasm32-wasip2` for the node, `vite build` per framework group, then `flow-like-widgets pack`. The React runtime lands in `shared/` **once** and both widget documents reference it, so adding widgets costs kilobytes, not another framework copy.

## The demo flow

The nodes and widgets are designed to be wired into one board:

1. **`Sales Demo Data`** → deterministic `rows` + `categories`.
2. **`Instantiate Widget` → Sales Chart** with those rows; **`Instantiate Widget` → Filter Panel** with the categories. Push both to a page container.
3. The user edits the Filter Panel and hits *Apply* → its `applied` event triggers a workflow event node (bound by `action_id`, exactly like a declarative widget action).
4. That handler runs **`Query Widget` → Filter Panel → `getValue`** (live read of the panel's state) → **`Apply Sales Filter`** → **`Update Widget Inputs` → Sales Chart** with the filtered rows. The chart updates in place.
5. Clicking a bar emits `pointSelected`; a handler runs **`Query Widget` → Sales Chart → `getSeries { top: 3 }`** → **`Sales Summary`** → feeds `headline` back into the chart's `title` through another `Update Widget Inputs`.

Steps 4 and 5 both round-trip through the live widget-query channel; if no surface is live (a headless run), the query falls back to the `value:changed` mirror that both widgets publish via `bridge.setValues(...)`.

## Contract highlights

`sales-chart` shows the full surface of a contract:

- **enum input** → `variant: "bar" | "line"` becomes a dropdown pin with `choices`.
- **JSON input with a schema** → `rows: SalesRow[]`, whose schema is inlined into the contract and enforced on the pin.
- **typed and void events** → `pointSelected: {...}` and `refreshRequested: void`.
- **query with arguments** → `getSeries: { args: { top: number }; returns: SalesRow[] }` generates a `dyn_arg_top` pin on `Query Widget`.

`filter-panel` shows the input-widget shape: a `getValue` query (the conventional value read), a `resetRequested` void event, and `bridge.setValues(...)` on every change so the value stays readable even without a live surface.

## Using this outside the monorepo

The widget group depends on the Flow-Like packages through `workspace:*` because it lives inside this repository. A scaffolded project (`Developer → New Project`) gets the published versions instead:

```json
"@flow-like/widget-sdk": "^0.1.0",
"@flow-like/widget-bundler": "^0.1.0"
```

Vite is launched as `bun --bun vite build` so the bundler's Vite plugin is loaded by Bun's TypeScript-aware runtime; the templates ship the same script.
