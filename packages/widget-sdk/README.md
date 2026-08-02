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
`pattern`, `anyOf` / `oneOf` / `allOf`. Schemas must be pre-inlined by the
bundler — `$ref` cannot be resolved at runtime and is treated as valid.
