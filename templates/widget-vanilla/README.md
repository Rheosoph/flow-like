# Flow-Like Widget Template (Vanilla TypeScript)

Framework group for Flow-Like micro widgets: one Vite app hosting 1..n widget
entrypoints under `src/widgets/<id>/`. The `flowLikeWidgets()` Vite plugin
discovers every `src/widgets/*/index.html` as a build input, splits shared
code into content-hashed `shared/` chunks, and injects the typed contract
derived from each `widget.config.ts`.

Zero-framework baseline and the reference SDK usage: subscribe to
`bridge.$props` directly (`$props.subscribe(...)`) and update the DOM by hand.

## Dev loop

```bash
mise run setup   # bun install
bun run dev      # plain Vite dev server
```

Open `http://localhost:5173/src/widgets/hello-widget/index.html`. Without a
host the SDK boots in standalone mode: `$props` falls back to the contract's
`@default` values, the stock Flow-Like theme tokens are applied (following
`prefers-color-scheme`), `emit()` logs to the console, and queries are
invokable from devtools via `window.__flw.query("getCount")`.

## Adding a widget

```bash
bunx flow-like-widgets add my-widget --group .
```

Each widget is a folder containing `widget.config.ts` (the typed contract via
`defineWidget<Inputs, Events, Queries>`), a thin `index.html`, and an entry
module. Style with the host theme tokens (`var(--primary)`,
`var(--background)`, `var(--foreground)`, `var(--radius)`, ...) so light/dark
host themes apply — no hardcoded colors.

## Building & packing

`mise run build` emits `dist/` with one thin document per widget plus shared
chunks. The root package project packs every framework group into the
publishable `widgets.flwb` artifact:

```bash
bunx flow-like-widgets pack --project . --out widgets.flwb
```
