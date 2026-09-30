---
title: Package widget contract
description: Define typed inputs, events, queries, capabilities, and host communication
---

A package widget's contract is the interface between its JavaScript and the
Flow that hosts it. Inputs carry values into the widget, events carry
notifications out, and queries let the Flow request a result or an acknowledged
state change. The bundler extracts `contract.json` from TypeScript types in
`widget.config.ts`.

Start with the [counter package](/dev/package-widgets/overview/) to build and
preview this contract.

## Define inputs, events, and queries

```typescript title="widget.config.ts"
import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
  /** Widget headline @default "Hello from Vanilla TS" */
  title: string;
  /** Counter start value @minimum 0 @maximum 100 @default 0 */
  count: number;
}

interface Events {
  increased: { value: number };
}

interface Queries {
  getCount: { args: void; returns: number };
}

export default defineWidget<Inputs, Events, Queries>({
  id: "hello-widget",
  name: "Hello Widget",
  description: "A counter with typed inputs and events",
  sizing: { defaultHeight: 240, resizable: true },
});
```

| Declaration | Effect |
| --- | --- |
| Required input with `@default` | Supplies the initial preview value and generated pin default |
| Optional input (`field?: T`) | Allows the host to omit that input |
| String union | Creates enum choices |
| `@minimum`, `@maximum` | Defines numeric bounds |
| Event payload | Validated value sent with `bridge.emit()` |
| Query `args` and `returns` | Validated request and response for `bridge.onQuery()` |
| `void` payload or query argument | Produces a null schema |
| Query with `@mutation` | Requires a live acknowledgement for a state-changing call |

Non-optional inputs without defaults produce a build warning. Keep widget IDs,
input names, event names, and query names stable after consumers bind them.
Review compatibility when changing their types or required fields.

## Mount and communicate

```typescript title="bridge.ts"
import { mountFlowWidget } from "@flow-like/widget-sdk";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
let count = bridge.$props.get().count;

bridge.$props.subscribe((props) => {
  count = props.count;
});
bridge.onQuery("getCount", () => count);

function increment() {
  count += 1;
  bridge.emit("increased", { value: count });
}
```

Call `increment()` from your UI's click handler. The host delivers input
updates through `$props`; an emitted event only requests the page's configured
action. Widgets do not start arbitrary workflows directly.

The SDK performs the `flw/1` handshake through `postMessage`, validates values,
and applies host theme tokens to the document. Use CSS variables such as
`var(--background)`, `var(--foreground)`, and `var(--primary)`. The host can
change the theme while the widget is open. The bridge reports height changes
unless the contract sets `sizing.resizable: false`.

React uses the optional `@flow-like/widget-sdk/react` hooks, including
`useWidgetProps(bridge)` and `useWidgetTheme(bridge)`. Other frameworks can
subscribe to the bridge's stores with their nanostores bindings.

## Connect a Flow

| Flow node | Contract use |
| --- | --- |
| **Instantiate Widget** | Creates an instance and exposes its typed input pins |
| **Update Widget Inputs** | Sends a partial update to a mounted package widget |
| **Query Widget** | Calls a named query with its argument and result schema |

For a state-changing query, annotate the query declaration:

```typescript
interface Queries {
  /** @mutation */
  setCount: { args: { value: number }; returns: void };
}
```

Register its handler with `bridge.onQuery("setCount", ({ value }) => {
count = value; })`. A mutation needs the live widget to acknowledge the call;
test unmounted and disconnected cases in the Flow as well as successful calls.

## Geometry and model values

Use the SDK's `GeoPoint`, `GeoLineString`, `GeoPolygon`, or other geometry
types to produce native Geometry pins. Coordinates use two-dimensional WGS 84
GeoJSON in `[longitude, latitude]` order. Feature and FeatureCollection
wrappers are outside this contract. See [Geometry](/reference/geometry/).

For model values, use `LlmHistory`, `LlmResponse`, and `LlmResponseChunk`.
Their contract schemas match Flow-Like's model nodes. Arrays keep their
collection shape; `@uniqueItems true` marks set inputs, and `Record<string, T>`
represents maps. An explicit nullable union remains JSON-shaped.

The bundler inlines schema references. Recursive types cannot be represented;
keep transport values finite and serializable.

## Capabilities and contract versions

The `capabilities` object opts into `workers`, `wasm`, `media`, `microphone`,
and `downloads`. These declarations affect the host sandbox and user consent.
They do not grant network destinations. Declare those in `csp` purpose groups
as described in [Network access](/dev/package-widgets/network/).

Contracts without `csp` use version 1. Contracts with `csp` use version 2;
older hosts reject them. The bundler writes the version and generated schemas,
so rebuild after changing declarations instead of editing packed JSON.

## Validate a change

From the package root:

```bash
bunx @flow-like/widget-bundler validate .
bunx @flow-like/widget-bundler validate widgets.flwb
```

The first command checks source contracts. The second checks the built bundle.
Rebuild and repack between them when the source changed. Then test input
updates, emitted payloads, query results, and theme changes in the host preview.
