# Flow-Like Widget Template (Vue)

Framework group for Flow-Like micro widgets: one Vite app hosting 1..n widget
entrypoints under `src/widgets/<id>/`. The `flowLikeWidgets()` Vite plugin
discovers every `src/widgets/*/index.html` as a build input, splits the Vue
runtime and shared code into content-hashed `shared/` chunks, and injects the
typed contract derived from each `widget.config.ts`.

Components bind to the core `bridge.$props` / `bridge.$theme` stores via
`@nanostores/vue`.

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
module (here: `index.ts` mounting `Widget.vue`). Style with the host theme
tokens (`var(--primary)`, `var(--background)`, `var(--foreground)`,
`var(--radius)`, ...) so light/dark host themes apply — no hardcoded colors.

Geometry inputs use types such as `GeoPoint` and `GeoPolygon` from
`@flow-like/widget-sdk`. The bundler turns them into Geometry pins. Geometry
values use `[longitude, latitude]` coordinate order.

## Network access (CSP)

Widgets run sandboxed. They can always use `data:` and `blob:` URLs, but they
reach no site on the network until the viewer approves it. Declare what a
widget needs in `widget.config.ts`. `src/widgets/weather-widget/` is a working
example:

```ts
export default defineWidget<Inputs, Events>({
	id: "weather-widget",
	// …
	csp: [
		{
			reason: "Fetches the current temperature from Open-Meteo",
			connectSrc: ["https://api.open-meteo.com"],
		},
	],
});
```

- Group sources by purpose. Every group needs a `reason` (8–120 characters of
  plain words, no web addresses). The viewer sees it, marked as your text.
- `connectSrc` covers `fetch`, `EventSource` and WebSockets (`https://` or
  `wss://`). `imgSrc`, `fontSrc`, `mediaSrc` and `styleSrc` take `https://`.
- Sources are origins without paths or ports, at most 16 per widget, written
  as string literals. A leading `*.` covers every subdomain, for example
  `https://*.earthdata.nasa.gov`. Open hosting such as
  `https://*.s3.eu-central-1.amazonaws.com` is allowed, but the viewer sees a
  warning because anyone can host files there.
- For hosts only known at runtime, such as a customer's CDN or signed storage
  URLs, declare the input that carries the URL instead:
  `{ reason, inputs: [{ path: "tileUrl", directives: ["imgSrc", "connectSrc"] }] }`.
  Flow-Like reads only the origin from the value and asks the viewer to approve
  it. A new signature on the same host needs no new approval.
- Before the widget loads, the viewer sees each site with an explanation and
  decides. If they run it without network access, and in store previews,
  requests fail, so show a useful error.
- Every approved site can receive anything the widget sees. Declare only what
  the widget needs.

The bundler checks all of this when it builds. The `@flow-like/widget-bundler`
README has recipes for Cesium ion, NASA GIBS and S3, GCS or Azure signed URLs.

## Building & packing

`mise run build` emits `dist/` with one thin document per widget plus shared
chunks. The root package project packs every framework group into the
publishable `widgets.flwb` artifact:

```bash
bunx @flow-like/widget-bundler pack --project . --out widgets.flwb
```
