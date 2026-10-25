# @flow-like/widget-sdk

SDK for Flow-Like micro widgets: typed contracts, the `flw/1` host bridge, and
framework adapters. Widgets run inside an opaque-origin sandboxed iframe and
talk to the host exclusively via `postMessage`; this SDK implements the widget
side of that protocol, plus a zero-config standalone mode for plain `vite dev`.

## Define a widget

Authors write ordinary TypeScript interfaces — `@flow-like/widget-bundler`
derives `contract.json` from the type arguments and injects it into the built
HTML as `globalThis.__FLW_CONTRACT__`.

```ts
// widget.config.ts
import { defineWidget } from "@flow-like/widget-sdk";

interface SalesRow {
	x: string;
	y: number;
}

interface Inputs {
	/** Chart headline @default "Sales" */
	title: string;
	/** @default "bar" */
	variant: "bar" | "line";
	/** @minimum 1 @maximum 500 @default 50 */
	limit: number;
	/** @default [] */
	rows: SalesRow[];
}

interface Events {
	pointSelected: SalesRow;
	refreshRequested: void;
}

interface Queries {
	getSelection: { args: void; returns: { rows: SalesRow[] } };
	getValue: { args: void; returns: string };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "sales-chart",
	name: "Sales Chart",
	description: "Interactive bar/line chart",
	sizing: { defaultHeight: 320, resizable: true },
	dev: {
		fixtures: {
			empty: { rows: [] },
			loaded: { title: "Q3 Sales", rows: [{ x: "Q1", y: 12 }] },
		},
	},
});
```

## Geometry contracts

Use the geometry types exported by `@flow-like/widget-sdk` when a widget
exchanges geometry with a workflow. They describe two-dimensional WGS 84
GeoJSON geometry objects. Every position uses `[longitude, latitude]` order.

```ts
import {
	defineWidget,
	type GeoGeometry,
	type GeoLineString,
	type GeoPoint,
	type GeoPolygon,
} from "@flow-like/widget-sdk";

interface Inputs {
	focus?: GeoPoint;
	/** @default [] */
	routes: GeoLineString[];
	/** @uniqueItems true @default [] */
	waypoints: GeoPoint[];
	regionsById?: Record<string, GeoPolygon>;
}

interface Events {
	pointSelected: GeoPoint;
}

interface Queries {
	contains: {
		args: { point: GeoPoint; area: GeoPolygon };
		returns: boolean;
	};
	getGeometry: { args: void; returns: GeoGeometry };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "route-map",
	name: "Route Map",
	description: "Routes and selectable map points",
});
```

`GeoPoint`, `GeoLineString`, `GeoPolygon`, `GeoMultiPoint`,
`GeoMultiLineString`, `GeoMultiPolygon`, `GeoGeometryCollection`, and
`GeoGeometry` carry the schema metadata that creates native Geometry pins.
`GeoPosition` is the `[longitude, latitude]` tuple used by their coordinates.
Arrays retain array shape, `@uniqueItems true` makes a set-shaped pin, and
`Record<string, GeoPoint>` makes a map-shaped pin.

For a compatible type declared in the widget, annotate the scalar type or
scalar property with `@geometry Point`, `LineString`, `Polygon`, `MultiPoint`,
`MultiLineString`, `MultiPolygon`, `GeometryCollection`, or `Any`. Container
properties get their geometry metadata from the annotated element type.

`validateSchema` and `validateInputValue` enforce the geometry profile. The
hosted bridge uses those checks for incoming `props:update` values and emitted
event payloads. Use GeoJSON geometry objects directly. GeoJSON Feature and
FeatureCollection wrappers are outside this contract. Geometry values cannot
be `null`; an explicit nullable union remains JSON-shaped rather than
producing a native Geometry pin.

## LLM contracts

Use `LlmHistory`, `LlmResponse` and `LlmResponseChunk` when a widget exchanges
chat history or model output with model and agent nodes. They follow the
serialization of Flow-Like's `History`, `Response` and `ResponseChunk`.

```ts
import {
	defineWidget,
	type LlmHistory,
	type LlmResponse,
	type LlmResponseChunk,
} from "@flow-like/widget-sdk";

interface Events {
	asked: { requestId: string; history: LlmHistory };
}

interface Queries {
	/** @mutation */
	pushChunk: { args: { requestId: string; chunk: LlmResponseChunk }; returns: void };
	/** @mutation */
	pushResponse: { args: { requestId: string; response: LlmResponse }; returns: void };
}

export default defineWidget<{}, Events, Queries>({ id: "chat", name: "Chat" });
```

The bundler writes these types into the contract as
`{"type":"object","x-flow-like-type":"llm","x-llm":"Response"}`. A Query
Widget or Update Widget Inputs pin for that marker carries exactly the schema
of the native Rust type, so outputs of model and agent nodes connect directly.
Query Widget enforces that schema on its argument and result pins. A widget
action payload is a single struct: read a `history` field with **Get Field**
and connect it to a History input.

`validateSchema` checks marked values against the native schemas in
`src/llm-schemas.ts`, which are copies of `packages/schema/llm`. After
`cargo run -p schema-gen` changes those files, copy them again; a test fails
until they match. Annotate your own object type with `@llm History`,
`Response` or `ResponseChunk` only if it serializes exactly like that type.
Like geometry, a nullable union stays JSON-shaped instead of producing a
native pin.

## Mount (hosted)

`mountFlowWidget` registers the message listener, performs the `flw/1`
handshake (`hello` → `init` → `ready`), applies the host theme as CSS custom
properties (and a `dark` class) on `document.documentElement`, and keeps the
nanostores in sync with `props:update` / `theme:change`. Auto-height is
reported via a coalesced `resize` message unless the contract sets
`sizing.resizable: false`.

```ts
import { mountFlowWidget } from "@flow-like/widget-sdk";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);

bridge.$props.subscribe((props) => render(props));
bridge.emit("pointSelected", { x: "Q1", y: 12 });
bridge.onQuery("getSelection", () => ({ rows: currentSelection() }));
bridge.setValues({ value: currentSelection() });
```

## Standalone mode

When no host answers within 300 ms (or the widget is opened top-level, e.g.
plain `vite dev`), the bridge boots standalone:

- `$props` is filled from the contract's input defaults,
- the bundled Flow-Like theme tokens are applied, following
  `prefers-color-scheme` live,
- `emit`/`setValues` log structured events to the console,
- queries are invokable from devtools via `window.__flw.query(name, args)`,
- a small "standalone" badge marks the mode.

## React

```tsx
import { useWidgetProps, useWidgetTheme } from "@flow-like/widget-sdk/react";
import { bridge } from "./main";

export function App() {
	const props = useWidgetProps(bridge);
	const theme = useWidgetTheme(bridge);
	return <h1 data-mode={theme.mode}>{props.title}</h1>;
}
```

Other frameworks use the official `@nanostores/*` bindings on `bridge.$props`
/ `bridge.$theme` / `bridge.$mode` directly (Svelte needs no adapter);
`@flow-like/widget-sdk/vanilla` re-exports the core for subscription-based
usage.

## Validation

`validateSchema` is a dependency-free JSON Schema subset validator (the repo
pins `ajv` too old to use here). Supported keywords: `type` (incl. `integer`
and type arrays), `enum`, `const`, `properties` / `required` /
`additionalProperties`, `items`, numeric bounds, string and array length,
`pattern`, `anyOf` / `oneOf` / `allOf`, and the Flow-Like geometry, LLM and
file markers.
Schemas must be pre-inlined by the bundler. `$ref` cannot be resolved at
runtime and is treated as valid.

## Host media and microphone

Widgets can declare `capabilities` in `defineWidget`: `workers`, `wasm`,
`media`, `microphone`, `downloads`, and `uploads` (see
[Host uploads](#host-uploads)). Each is opt-in. Workers, media, and
WebAssembly access stay restricted to bundled assets and local Blob URLs. The
iframe retains its opaque origin. Downloads add only `allow-downloads` to its
sandbox policy.

Widgets cannot start workflows directly. To request bytes owned by a workflow,
emit a declared contract event and let a page-defined action handle it. That
workflow returns the bytes with Query Widget on a declared contract query.
Declare that query `@mutation` so each call needs a live acknowledgement.

`bridge.captureAudio({ maxDurationMs, signal })` records through the host and
returns audio bytes and a MIME type. `bridge.stopAudioCapture()` finishes early.
A host-owned stop button stays visible during capture. The recording lasts at
most 60 seconds and ends on teardown. Send its result to an appropriate workflow
audio node; this API does not create a WebRTC session.


For public radio streams, expose `publicMediaGrants: [{ id, url }]` alongside the
`media` capability. A source node must resolve and vet these credential-free
HTTPS broadcaster URLs. The host strips the grants before delivering props and
only announces their IDs in `$capabilities.mediaIds`. `playMedia(id)` returns a
promise; `pauseMedia()` and `stopMedia()` control the host player. `$media` reports
its current ID and state. The host always shows Play/Pause/Stop controls, including
a Play button when browser autoplay policy requires a fresh user gesture.

The host rejects literal private destinations, localhost, URL userinfo, and
credential query fields. Broadcaster DNS and redirects must also be vetted by
the source workflow. Native HTML audio uses anonymous CORS; MP3/AAC broadcasts
are supported when the broadcaster permits it. Widget teardown and grant
revocation stop playback.

## Host uploads

With the `uploads` capability, a widget hands a file the viewer picked to the
host instead of sending its bytes through an event. `bridge.uploadFile(file)`
stores the file in the viewer's temporary Flow-Like storage for this app and
resolves to a `WidgetFile` handle:

```ts
interface WidgetFile {
	$flwFile: string; // opaque handle
	name: string;
	size: number;
	mimeType: string;
}
```

Put the handle into a declared event payload. The bundler writes `WidgetFile`
into the contract as `{"type":"object","x-flow-like-type":"file"}`, also as an
array element, and the bridge checks the handle form on `emit`. Before the
event reaches the workflow, the host swaps each handle for the stored file
`{ name, size, type, url, flowPath }`; an event with an unknown handle is
dropped. The widget never sees the URL or storage path. Only event payloads may
carry `WidgetFile`; the bundler rejects it in inputs and queries.

In the workflow, pass the file to **Uploaded File to Path** to get a FlowPath.
Temporary files expire after about 32 days, so copy the file to permanent
storage when the app needs to keep it.

```ts
// widget.config.ts
import { defineWidget, type WidgetFile } from "@flow-like/widget-sdk";

interface Events {
	attached: { files: WidgetFile[] };
}

export default defineWidget<{}, Events, {}>({
	id: "attachments",
	name: "Attachments",
	description: "Hands picked files to the workflow",
	capabilities: { uploads: true },
});
```

```tsx
import { WidgetUploadError, type WidgetFile } from "@flow-like/widget-sdk";
import { useStore } from "@flow-like/widget-sdk/react";
import { type ChangeEvent, useState } from "react";
import { bridge } from "./main";

export function AttachFiles() {
	const canUpload = useStore(bridge.$capabilities).uploads === true;
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function attach(event: ChangeEvent<HTMLInputElement>) {
		const picked = [...(event.target.files ?? [])];
		event.target.value = "";
		setBusy(true);
		setError(null);
		try {
			const files: WidgetFile[] = [];
			for (const file of picked) files.push(await bridge.uploadFile(file));
			if (files.length > 0) bridge.emit("attached", { files });
		} catch (e) {
			setError(e instanceof WidgetUploadError ? e.message : "Upload failed");
		} finally {
			setBusy(false);
		}
	}

	if (!canUpload) return null;
	return (
		<label>
			<input type="file" multiple disabled={busy} onChange={attach} />
			{error && <p role="alert">{error}</p>}
		</label>
	);
}
```

Uploads are for signed-in viewers only. A file may be at most 25 MB
(`MAX_WIDGET_UPLOAD_BYTES`), and at most 4 uploads run at once per widget
(`MAX_WIDGET_UPLOADS_IN_FLIGHT`). `options.name` replaces the file name, and
`options.signal` cancels the upload with an `AbortError`. Other failures reject
with a `WidgetUploadError` whose `code` is one of:

| Code | Meaning |
| --- | --- |
| `not-granted` | No `uploads` grant: undeclared, declined, preview or standalone mode |
| `too-large` | The file exceeds 25 MB |
| `busy` | Too many uploads in flight or in the last minute |
| `sign-in-required` | The viewer is not signed in |
| `unavailable` | The host cannot store files here |
| `failed` | The upload failed |
| `timeout` | No result within 10 minutes |
| `disposed` | The widget was torn down |

Never send file bytes, base64 or data URLs through events. Hosted runs cap a
request at 2 MiB, and the host repeats the payload in the action context, so
even files well below 1 MB can fail.
