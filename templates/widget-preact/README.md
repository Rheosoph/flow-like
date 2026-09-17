# Flow-Like Widget Template (Preact)

Framework group for Flow-Like micro widgets: one Vite app hosting 1..n widget
entrypoints under `src/widgets/<id>/`. The `flowLikeWidgets()` Vite plugin
discovers every `src/widgets/*/index.html` as a build input, splits the Preact
runtime and shared code into content-hashed `shared/` chunks, and injects the
typed contract derived from each `widget.config.ts`.

Preact is the recommended default for small widgets — React DX at ~11 KB
gzipped. The SDK has no dedicated `/preact` adapter subpath, so components
bind to the core `bridge.$props` / `bridge.$theme` stores via
`@nanostores/preact`.

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
bunx @flow-like/widget-bundler add my-widget --group .
```

Each widget is a folder containing `widget.config.ts` (the typed contract via
`defineWidget<Inputs, Events, Queries>`), a thin `index.html`, and an entry
module. Style with the host theme tokens (`var(--primary)`,
`var(--background)`, `var(--foreground)`, `var(--radius)`, ...) so light/dark
host themes apply — no hardcoded colors.

Geometry inputs use types such as `GeoPoint` and `GeoPolygon` from
`@flow-like/widget-sdk`. The bundler turns them into Geometry pins. Geometry
values use `[longitude, latitude]` coordinate order.

## Network access (CSP)

Widgets run sandboxed with no network access. To call an external service,
declare its origin in `widget.config.ts`. `src/widgets/weather-widget/` is a
working example:

```ts
export default defineWidget<Inputs, Events>({
	id: "weather-widget",
	// …
	csp: {
		connectSrc: ["https://api.open-meteo.com"],
	},
});
```

- `connectSrc` covers `fetch`, `EventSource` and WebSockets (`https://` or
  `wss://`). `imgSrc`, `fontSrc`, `mediaSrc` and `styleSrc` take `https://`
  origins.
- Exact origins only: no paths, ports, wildcards, IP addresses or `localhost`,
  and at most 16 per widget. Values must be string literals, because the
  bundler reads the config statically and rejects anything invalid.
- A widget that declares `csp` is published with `contractVersion: 2`. Before
  it loads, Flow-Like shows the viewer the listed sites and asks for approval.
  If the viewer runs it without approval, and in store previews, requests fail,
  so show a useful error.
- Every listed site can receive anything the widget sees. List only what the
  widget needs.

## Building & packing

`mise run build` emits `dist/` with one thin document per widget plus shared
chunks. The root package project packs every framework group into the
publishable `widgets.flwb` artifact:

```bash
bunx @flow-like/widget-bundler pack --project . --out widgets.flwb
```
