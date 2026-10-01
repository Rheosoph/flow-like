---
title: Build a package widget
description: Build, preview, and distribute a typed JavaScript widget in a Flow-Like package
---

A package micro widget is a small web interface that runs inside a sandboxed
iframe. Write it in TypeScript or a UI framework, declare its inputs, events,
and queries, then distribute its built files in a Flow-Like package. A Flow
supplies data and handles the events that the widget emits.

Use an [A2UI Widget](/dev/a2ui/widgets/) when a reusable declarative component
graph fits the task. Use a package widget when you need custom rendering or a
framework library. Both can appear in an app interface, but they have separate
authoring and distribution paths.

## Prepare a counter package

Install Bun and clone the Flow-Like repository as described in
[Building from Source](/dev/build/). The following commands run from that
checkout and create a sibling project:

```bash
mkdir -p ../hello-widgets/widgets
cp -R templates/widget-vanilla ../hello-widgets/widgets/vanilla
cd ../hello-widgets
```

Create `flow-like.toml` at this project's root:

```toml title="flow-like.toml"
manifest_version = 2
id = "com.example.hello-widgets"
name = "Hello Widgets"
version = "0.1.0"
description = "A counter with typed inputs and events"
keywords = []
widget_bundle_path = "widgets.flwb"

[permissions]
memory = "standard"
timeout = "standard"
```

The directory structure matters. The bundler discovers framework groups under
`widgets/<group>/`, and widgets under each group's `src/widgets/<id>/`.
The copied template includes a counter and a weather example; start with the
counter, which needs no network access.

```text
hello-widgets/
  flow-like.toml
  widgets/vanilla/
    package.json
    vite.config.ts
    src/widgets/hello-widget/
      widget.config.ts
      index.html
      index.ts
```

## Preview and inspect the contract

Install the group's dependencies and start its Vite server:

```bash
bun install --cwd widgets/vanilla
bun run --cwd widgets/vanilla dev
```

Open `http://localhost:5173/src/widgets/hello-widget/index.html` (or the port
Vite prints). You should see **Hello from Vanilla TS** and a **Count: 0**
button. Clicking it increments the count. Without a Flow-Like host, the SDK
uses contract defaults and logs the `increased` event to the browser console.
Run `window.__flw.query("getCount")` in that console to read the counter.

For a host preview, stop Vite and run from the package root:

```bash
bunx @flow-like/widget-bundler dev --project . --port 4700
```

Open `http://localhost:4700/`. Select `hello-widget`, change its `title` and
`count` inputs, click the counter, and inspect the `increased` event. Invoke
`getCount` to check the result. The harness also provides theme and viewport
controls. See the [contract guide](/dev/package-widgets/contract/) before
changing these inputs or events.

## Build and validate

Stop the development server, then run from the package root:

```bash
bun run --cwd widgets/vanilla build
bunx @flow-like/widget-bundler validate .
bunx @flow-like/widget-bundler pack --project . --out widgets.flwb
bunx @flow-like/widget-bundler validate widgets.flwb
```

The build writes the framework output into `widgets/vanilla/dist/`. Packing
combines it with the extracted contracts into `widgets.flwb`. Rebuild the
group before packing whenever its source changes. The manifest's package ID
and version are copied into the bundle.

## Preview in Desktop and install

1. Enable [Developer Mode](/start/developer-mode/), then open **Packages → Mine**.
2. Select **Add folder** and choose the `hello-widgets` root containing
   `flow-like.toml`.
3. Open the package's **Debug & test** view and use **Preview widgets** to
   inspect the built counter through the desktop widget host.
4. Choose a package ID you control before publishing. Build and pack again if
   you change the ID or version.
5. Select **Publish…**, review the extracted widgets, and follow the
   [registry workflow](/dev/wasm-nodes/registry/). Publish only when you intend
   to distribute that version.
6. Install the available package version from **Packages** on the target
   device and add it to the app's packages. See
   [installed packages](/start/packages-library/) for device and app scope.

Use **Instantiate Widget** to select the package widget in a Flow. Its
contract supplies the input pins. **Update Widget Inputs** sends new values;
**Query Widget** invokes declared queries. Bind the widget's emitted actions
to the app's handler before relying on them to change application data.

## Add framework or network features

The repository also has React, Preact, Vue, Svelte, Solid, and Lit templates.
Each framework group builds separately before the package is packed. Keep
widget IDs unique across groups and use host theme variables for colors.

Network destinations require purpose declarations and viewer approval. Follow
[Network access and consent](/dev/package-widgets/network/) and test the
declined-permission state as well as the successful request.
