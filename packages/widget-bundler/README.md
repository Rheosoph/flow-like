# @flow-like/widget-bundler

Builds `.flwb` (Flow-Like Widget Bundle) artifacts for package projects: extracts typed contracts from plain TypeScript widget configs, inlines framework build output into self-contained documents, and packs everything into a deterministic ZIP with `bundle.json`.

The Rust source of truth for the emitted formats is `packages/wasm/schema/src/widget.rs` (`contract.json`) and `packages/wasm/schema/src/widget_bundle.rs` (`bundle.json` / archive layout).

## CLI

```bash
# Pack all framework groups of a project into widgets.flwb
bunx @flow-like/widget-bundler pack --project . --out widgets.flwb \
  [--serving-prefix flow-widget://pkg@hash/] [--created-at 2026-07-21T12:00:00Z]

# Mock-host dev harness (design §8.4 Layer 1): starts every framework group's
# own dev script and serves a browser page that is a real flw/1 host
bunx @flow-like/widget-bundler dev [--project .] [--port 4700]

# Validate widget contracts of a project, or a built bundle
bunx @flow-like/widget-bundler validate .
bunx @flow-like/widget-bundler validate widgets.flwb

# Scaffold a new widget inside a framework group
bunx @flow-like/widget-bundler add kpi-card --group widgets/react
```

`dev` spawns `bun run dev -- --port <n> --strictPort` per framework group (ports assigned from the harness port upward; the child's `Local:` stdout line overrides the assignment if the dev script ignores the flags) and serves the harness on `http://localhost:4700/`: every widget in a sandboxed iframe speaking `flw/1`, with a contract-generated props panel, `dev.fixtures` presets, event log with contract validation, query invoker, theme toggle + token editor, viewport/preview controls, and a raw protocol trace. Contracts are re-extracted per request, cached by `widget.config.ts` mtime (`GET /api/contract/<group>/<id>`).

`pack` requires each framework group (`widgets/<group>/`) to be built first (`bun run build` producing `dist/`). `createdAt` is omitted from `bundle.json` unless `--created-at` or `SOURCE_DATE_EPOCH` is set, keeping builds byte-for-byte deterministic. Every file under a group's `dist/shared/` is packed (chunks can import each other); each widget's `assets` lists the chunks its document references directly.

Packed documents carry a `<meta>` CSP built from the widget's `capabilities`
only. It allows bundle assets from their own web origin and Flow-Like's desktop
widget protocol, local `data:` and `blob:` bytes, and `--serving-prefix` adds
another asset source. It never lists network hosts. Declare those per widget in `csp` (see
[Network access](#network-access)).

The `--connect` flag was removed. `pack --connect` now fails with
`The --connect flag was removed: declare network sources per widget in widget.config.ts "csp"`,
and the programmatic `connectHosts` pack option fails the same way. Move each
host into the `csp` of the widgets that need it.

## Vite plugin

```ts
// widgets/react/vite.config.ts
import { defineConfig } from "vite";
import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";

export default defineConfig({ plugins: [flowLikeWidgets()] });
```

Discovers `src/widgets/*/index.html` as one build input per widget id, routes entry/chunk/asset output into `dist/.../shared/`, and injects the extracted `__FLW_CONTRACT__` script during dev and build (pack re-injects it authoritatively).

## Widget authoring

```ts
// widgets/react/src/widgets/sales-chart/widget.config.ts
import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** Chart headline @default "Sales" */
	title: string;
}
interface Events {
	refreshRequested: void;
}
interface Queries {
	getValue: { args: void; returns: string };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "sales-chart",
	name: "Sales Chart",
	description: "Interactive chart",
	sizing: { defaultHeight: 320, resizable: true },
});
```

Contracts are derived statically (TypeScript compiler API + `ts-json-schema-generator`): JSDoc `@default` / `@minimum` / `@maximum` become pin defaults and bounds, string unions become enum choices, and `void` payloads become `null` schemas. Geometry annotations select the shared geometry validator; other schemas have their `$ref`s inlined and cannot be recursive. Non-optional inputs without a `@default` produce a warning because standalone dev and generated pins have no initial value.

Geometry types from `@flow-like/widget-sdk` produce native Geometry pins while
keeping the same TypeScript contract for widget props, events, and queries:

```ts
import {
	defineWidget,
	type GeoPoint,
	type GeoPolygon,
	type GeoPosition,
} from "@flow-like/widget-sdk";

/** @geometry Point */
type SearchCenter = {
	type: "Point";
	coordinates: GeoPosition;
};

interface Inputs {
	center?: SearchCenter;
	/** @default [] */
	stops: GeoPoint[];
	/** @uniqueItems true @default [] */
	visited: GeoPoint[];
	areas?: Record<string, GeoPolygon>;
}
```

The generated contract represents a point input as
`{"type":"json","schema":{"type":"object","x-flow-like-type":"geometry","x-geometry":"Point"}}`.
`Any` omits `x-geometry`. The host converts that metadata into the Geometry
pins used by Instantiate Widget and Update Widget Inputs.

The SDK also exports `GeoLineString`, `GeoMultiPoint`,
`GeoMultiLineString`, `GeoMultiPolygon`, `GeoGeometryCollection`, and
`GeoGeometry`. Custom scalar geometry types can use `@geometry` with one of
those concrete GeoJSON kinds or `Any`. Put the annotation on the element type
for arrays, sets, and maps. The annotation replaces the structural schema with
the geometry profile and retains descriptions and defaults. Extraction rejects
incompatible type annotations and invalid geometry defaults.
Geometry is two-dimensional WGS 84 GeoJSON with
positions in `[longitude, latitude]` order. Feature wrappers are unsupported.
An explicit nullable union stays JSON-shaped and does not produce a native
Geometry pin.

## Network access

A widget runs with no network access by default. It can always read bytes it
already holds: `fetch("data:…")`, object URLs from `URL.createObjectURL`,
`<audio>`/`<video>` with `data:` or `blob:` sources, and `data:`/`blob:` images
and `data:` fonts. These local schemes are never network access, need no
approval and cannot be declared. Scripts, frames and workers always stay limited
to the bundle.

To reach other sites, declare `csp` as a list of purpose groups. Each group
says why the widget needs the network (`reason`) and which sources it may use,
either as fixed addresses or as widget inputs that carry addresses at runtime.
Every viewer approves the sources for themselves before the widget can reach
them.

```ts
export default defineWidget<Inputs, Events, Queries>({
	id: "live-map",
	name: "Live map",
	capabilities: { workers: true },
	csp: [
		{
			reason: "Loads map tiles and live vehicle positions",
			connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
			imgSrc: ["https://a.tile.openstreetmap.org", "https://b.tile.openstreetmap.org"],
		},
		{
			reason: "Loads web fonts for map labels",
			styleSrc: ["https://fonts.googleapis.com"],
			fontSrc: ["https://fonts.gstatic.com"],
		},
	],
});
```

| Key | Directive | Allowed schemes | Used for |
| --- | --- | --- | --- |
| `connectSrc` | `connect-src` | `https`, `wss` | `fetch`, `XMLHttpRequest`, WebSocket, EventSource |
| `imgSrc` | `img-src` | `https` | images |
| `fontSrc` | `font-src` | `https` | web fonts |
| `mediaSrc` | `media-src` | `https` | audio and video |
| `styleSrc` | `style-src` | `https` | stylesheets |

A group may also hold `inputs` (see [Addresses known only at runtime](#addresses-known-only-at-runtime)).
No other key is allowed.

### Reasons

The viewer sees the reason next to the sources, labelled as text from the
publisher and placed below what Flow-Like says about each source. Write one
short sentence about what the widget does with the network. Extraction
normalizes it (NFC, runs of whitespace become one space, trimmed) and the build
fails when it:

- is not 8 to 120 characters long, or has fewer than 3 letters;
- uses anything other than letters, marks, numbers, spaces and `, . : ; ( ) - ' / &`
  (no quotes, emoji, `!`, `?`, `%` or `_`);
- mixes ASCII letters with letters of another script in one word;
- contains a web address, an email address or a domain name (`cesium.com`,
  `www.`, `https://`); "Node.js" and "e.g." are fine;
- mentions Flow-Like, or claims that sources are verified, trusted, approved,
  official, certified, secure or safe;
- in a group with `inputs`, claims who supplies the addresses (app, admin,
  organization, company, workspace, project, owner, …);
- repeats the reason of another group.

Reasons, groups and levels are for display only. They never change what the
widget may reach, and editing a reason alone does not ask viewers again.

### Sources

Each source is `scheme://host` or `scheme://*.host`. Extraction lowercases
sources, converts internationalized hosts (and wildcard bases) to punycode,
sorts and deduplicates each list, and checks the grammar shared with the Rust
schema (`packages/wasm/schema/src/widget_policy.rs`). A source is rejected,
and the build fails with
`Invalid widget csp source "<src>" in csp[<n>].<directive> for widget <id>: <reason>`,
when it has any of the following:

- a `*` anywhere but a leading `*.` label (`*`, `https://*`, `https://a.*.example.com`);
- a port (including `:443`), a path, a query, a fragment or userinfo;
- a keyword, nonce or hash (`'self'`, `'unsafe-inline'`, `'nonce-…'`);
- only a scheme (`https:`, `data:`, `blob:`), or `http://` or `ws://`;
- an IP literal;
- a host equal to or ending in `localhost`, `local`, `internal`, `lan`,
  `home.arpa`, `test`, `example`, `invalid`, `onion`, or an IP-mapping name
  (`nip.io`, `sslip.io`, `traefik.me`, `localtest.me`, `lvh.me`,
  `localhost.direct`);
- a wildcard over a public suffix (`https://*.co.uk`, `https://*.kawasaki.jp`),
  checked against the Public Suffix List compiled into the bundler.

| Limit | Value |
| --- | --- |
| Purpose groups | 1–8 |
| Sources across all groups (a wildcard counts once per directive) | 16 |
| Source bytes, Σ(length + 1) | 1536 |
| Each group | at least one source or one input |
| A source | belongs to one group (it may appear under several directives of that group) |

Hosts can also reject a source that points at their own domain. In that case
the widget runs without its network permissions and the viewer sees a notice.

### Wildcards

`https://*.tiles.example.com` matches every host that ends in
`.tiles.example.com`, at any depth, and never `tiles.example.com` itself;
declare that host separately when the widget needs it. Prefer exact hosts: they
work on every engine, while hosts whose engine mishandles wildcards can
withhold wildcard sources.

### Warning levels

Flow-Like classifies every source from the Public Suffix List and a curated
catalog of services. Publishers cannot influence the level; the viewer sees it
on each source and the group shows its highest level.

| Level | Label | Meaning | Examples |
| --- | --- | --- | --- |
| `known` | Identified service | one named operator; no account holder can receive requests or publish content there | `https://gibs.earthdata.nasa.gov`, `https://tile.googleapis.com` |
| `external` | External site | one party receives, and Flow-Like cannot verify who | `https://tiles.customer-maps.com`, `https://mybucket.s3.eu-central-1.amazonaws.com`, `https://api.cesium.com` |
| `shared` | Hosts user content | many parties publish content there; they cannot receive requests | `https://assets.ion.cesium.com`, `https://cdn.jsdelivr.net`, `https://*.github.io` |
| `broad` | Anyone can receive | open hosting: anyone who signs up can receive what the widget sends | `https://*.s3.eu-central-1.amazonaws.com`, `https://s3.eu-central-1.amazonaws.com`, `https://storage.googleapis.com`, `https://*.pages.dev`, request-capture and tunnel services |

Every level up to `shared` is presented calmly with a one-line explanation.
Only `broad` gets destructive styling, a confirmation for "Always allow" and
"Don't allow" as the default button, so avoid it when a narrower source works:
a bucket's own host instead of the regional endpoint or a bucket wildcard, or
an input that carries the bucket's URL.

### Addresses known only at runtime

Some addresses exist only while the app runs: a customer's CDN, a bucket
chosen in a flow, signed URLs that expire. Declare the inputs that carry them:

```ts
csp: [
	{
		reason: "Loads map tiles from tile servers given to it at runtime",
		inputs: [
			{ path: "tileUrl", directives: ["imgSrc", "connectSrc"],
			  template: { subdomainsInput: "tileSubdomains" } },
		],
	},
],
```

- `path` is `root *( "." key / "[]" / ".*" )`, at most 6 segments. The root is
  a widget input: a `string` input must be the whole path, a `json` input may
  descend with `.key`, `[]` (array items) and `.*` (map values). The path must
  reach `type: string` in the input's generated JSON Schema, and `.*` needs an
  object type with `additionalProperties` or `patternProperties`
  (`Record<string, …>`). `publicMediaGrants` is reserved by the host.
- The host reads the input values it forwards to the widget and keeps only
  `scheme://host` of each URL (`https` or `wss`). Paths, queries and signatures
  never matter, so rotating a signed URL on the same host needs no new
  approval. Ports, IP addresses, userinfo and local names are rejected;
  relative, `data:` and `blob:` values are skipped.
- Each viewer approves new hosts for themselves; nothing is approved for other
  viewers or by the publisher.
- `template` expands `{s}` in the host from a fixed list (`subdomains: ["a", "b", "c"]`)
  or from another `string`/`json` input (`subdomainsInput`). Other placeholders
  and runtime wildcards are not supported.
- At most 8 inputs across all groups, and each path appears once.
- `@default` values are merged inside the widget, so the host never sees them.
  The build warns when a default URL's host is not declared as a static source.
- The build warns when an input feeds `mediaSrc` without `connectSrc`: hls.js
  and MSE players fetch playlists and segments through `connectSrc`, only
  native HLS uses `mediaSrc`.

### Recipes

**Cesium ion.** Tile hosts need `connectSrc` and `imgSrc` because CesiumJS
loads imagery through XHR and falls back to `<img>`. Its image feature probe
fetches a `data:` URL, which every widget may do. Drop groups for ion assets you
don't use.

```ts
capabilities: { workers: true, wasm: true },
csp: [
	{ reason: "Loads globe terrain, imagery and 3D tiles from Cesium ion",
	  connectSrc: ["https://api.cesium.com", "https://assets.ion.cesium.com"],
	  imgSrc: ["https://assets.ion.cesium.com"] },
	{ reason: "Loads Bing Maps aerial imagery that Cesium ion points to",
	  connectSrc: ["https://dev.virtualearth.net", "https://ecn.t0.tiles.virtualearth.net",
	               "https://ecn.t1.tiles.virtualearth.net", "https://ecn.t2.tiles.virtualearth.net",
	               "https://ecn.t3.tiles.virtualearth.net"],
	  imgSrc: ["https://ecn.t0.tiles.virtualearth.net", "https://ecn.t1.tiles.virtualearth.net",
	           "https://ecn.t2.tiles.virtualearth.net", "https://ecn.t3.tiles.virtualearth.net"] },
	{ reason: "Loads Google photorealistic 3D tiles through Cesium ion",
	  connectSrc: ["https://tile.googleapis.com"] },
],
```

That is 13 of the 16 sources, all exact hosts. `api.cesium.com` is `external`
(account holders can write assets with their own token),
`assets.ion.cesium.com` is `shared` (user uploads), the Bing and Google hosts
are `known`.

**NASA GIBS.** Level `known`. Add `gibs-a`, `gibs-b` and `gibs-c` only if the
map client shards requests.

```ts
csp: [{ reason: "Loads satellite imagery tiles from NASA GIBS",
        imgSrc: ["https://gibs.earthdata.nasa.gov"],
        connectSrc: ["https://gibs.earthdata.nasa.gov"] }],
```

**S3, GCS and Azure signed URLs.** A flow signs the URLs and updates the input:

```ts
csp: [{ reason: "Shows map layers from storage files given to it at runtime",
        inputs: [{ path: "layers[].url", directives: ["imgSrc", "connectSrc"] }] }],
```

| URL the flow produces | Approved source | Level |
| --- | --- | --- |
| `https://acme-tiles.s3.eu-central-1.amazonaws.com/t/1/2/3.png?X-Amz-…` | `https://acme-tiles.s3.eu-central-1.amazonaws.com` | `external` |
| `https://s3.eu-central-1.amazonaws.com/acme-tiles/…` (path-style) | `https://s3.eu-central-1.amazonaws.com` | `broad` (every bucket in the region) |
| `https://acme-tiles.storage.googleapis.com/…` | `https://acme-tiles.storage.googleapis.com` | `external` |
| `https://storage.googleapis.com/acme-tiles/…?X-Goog-…` | `https://storage.googleapis.com` | `broad` |
| `https://acmetiles.blob.core.windows.net/tiles/…?sv=…&sig=…` | `https://acmetiles.blob.core.windows.net` | `external` |

- Sign virtual-hosted style (the bucket in the host) and use regional
  endpoints. The legacy global S3 host redirects, and the redirect target was
  never approved.
- Bucket CORS must allow `GET` from origin `*`: the sandboxed widget sends
  `Origin: null`, and allowing `null` is unsafe. The signature stays the
  capability.
- Re-sign before expiry (for example 60 minutes signed, refreshed at 50).
- Don't redirect from an API to a presigned URL on another host.

### Contract and enforcement

A widget that declares `csp` gets `contractVersion: 2` in `contract.json`, with
each group canonical (sources sorted bytewise, inputs sorted by path,
directives in the order of the table above). Widgets without it stay at
version 1. Hosts and hubs released before `csp` support reject version 2
bundles instead of silently dropping the declaration. Older bundler releases
do drop `csp`, and the widget then has no network access.

The pack-time `<meta>` CSP is not a security control. It allows
`connect-src data: blob:` and `media-src data: blob:` plus bundle assets, and
never contains `csp` sources, so a server that sends no header still keeps the
widget away from the declared sites. The host sends the enforced policy with
the document, including the approved sources and the granted capabilities, and
replaces the packed meta. Store previews never get `csp` sources. The dev
harness does not enforce CSP either, so requests that work under `dev` can
still be blocked in Flow-Like until a source is declared and approved.

`validate` checks the purpose rules, reasons, source grammar, wildcard bases,
input paths and the version rule, for a project and for a built `.flwb`.

## Mise integration (design §8.2)

```toml
[tasks."bundle:widgets"]
depends = ["build:widgets"]
run = "bunx @flow-like/widget-bundler pack --project . --out widgets.flwb"
```

## Programmatic API

```ts
import { pack, validateProject, validateBundle, extractContract } from "@flow-like/widget-bundler";
```
