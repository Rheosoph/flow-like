# @flow-like/widget-bundler

Builds `.flwb` (Flow-Like Widget Bundle) artifacts for package projects: extracts typed contracts from plain TypeScript widget configs, inlines framework build output into self-contained documents, and packs everything into a deterministic ZIP with `bundle.json`.

The Rust source of truth for the emitted formats is `packages/wasm/src/widget.rs` (`contract.json`) and `packages/wasm/src/widget_bundle.rs` (`bundle.json` / archive layout).

## CLI

```bash
# Pack all framework groups of a project into widgets.flwb
bunx flow-like-widgets pack --project . --out widgets.flwb \
  [--serving-prefix flow-widget://pkg@hash/] [--connect https://api.example.com ...] \
  [--created-at 2026-07-21T12:00:00Z]

# Mock-host dev harness (design §8.4 Layer 1): starts every framework group's
# own dev script and serves a browser page that is a real flw/1 host
bunx flow-like-widgets dev [--project .] [--port 4700]

# Validate widget contracts of a project, or a built bundle
bunx flow-like-widgets validate .
bunx flow-like-widgets validate widgets.flwb

# Scaffold a new widget inside a framework group
bunx flow-like-widgets add kpi-card --group widgets/react
```

`dev` spawns `bun run dev -- --port <n> --strictPort` per framework group (ports assigned from the harness port upward; the child's `Local:` stdout line overrides the assignment if the dev script ignores the flags) and serves the harness on `http://localhost:4700/`: every widget in a sandboxed iframe speaking `flw/1`, with a contract-generated props panel, `dev.fixtures` presets, event log with contract validation, query invoker, theme toggle + token editor, viewport/preview controls, and a raw protocol trace. Contracts are re-extracted per request, cached by `widget.config.ts` mtime (`GET /api/contract/<group>/<id>`).

`pack` requires each framework group (`widgets/<group>/`) to be built first (`bun run build` producing `dist/`). `createdAt` is omitted from `bundle.json` unless `--created-at` or `SOURCE_DATE_EPOCH` is set, keeping builds byte-for-byte deterministic. Every file under a group's `dist/shared/` is packed (chunks can import each other); each widget's `assets` lists the chunks its document references directly.

Packed documents allow bundle assets from their own web origin and Flow-Like's
desktop widget protocol by default. `--serving-prefix` adds another asset source;
`--connect` remains required for every network host a widget may contact.

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

Contracts are derived statically (TypeScript compiler API + `ts-json-schema-generator`): JSDoc `@default` / `@minimum` / `@maximum` become pin defaults and bounds, string unions become enum choices, `void` payloads become `null` schemas, and all `$ref`s are inlined (recursive types are rejected). Non-optional inputs without a `@default` produce a warning — they break standalone dev and pin defaults.

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
