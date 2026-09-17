---
title: Widgets
description: Build, version, and reuse A2UI component groups
sidebar:
  order: 2
---

A Widget is a reusable A2UI component graph stored in an app. It can be
rendered on its own for preview, inserted into a Page as a Widget instance,
and configured with named action IDs.

Widgets are managed in the app's **Widgets** workspace.

![The Widgets workspace in Flow-Like Desktop](../../../../assets/WidgetsOverview.webp)

## Widget Lifecycle

1. Create a Widget with a name and optional description.
2. Open its details view to preview it and edit its metadata.
3. Open the Visual Builder to compose its A2UI surface.
4. Define named Widget events when the block needs to expose interaction.
5. Create Patch, Minor, or Major snapshots when you need a stable version.
6. Add the Widget from a Page's component palette.

There is no `.widget` export/import flow in the current UI. Reuse happens through the Widget records available to the signed-in user and the references stored with a Page.

## Details and Builder

The Widget details view and Widget Builder serve different purposes.

| View | Purpose |
| --- | --- |
| **Widget details** | Rendered preview and descriptive metadata |
| **Widget Builder** | Component graph, data bindings, styles, actions, and Widget settings |

Select **Open Builder** from the details view.

![The Widget Builder editing a reusable A2UI surface in dark mode](../../../../assets/WidgetBuilder.webp)

The builder uses the same Components, Hierarchy, Canvas, Inspector, Dev Mode, and responsive Preview as a Page. See the [Visual Builder guide](/dev/a2ui/visual-builder/) for those controls.

## Current Widget Model

```typescript
interface IWidget {
  id: string;
  name: string;
  description?: string;
  rootComponentId: string;
  components: SurfaceComponent[];
  dataModel: DataEntry[];
  customizationOptions: CustomizationOption[];
  exposedProps?: ExposedProp[];
  actions?: WidgetAction[];
  tags: string[];
  catalogId?: string;
  thumbnail?: string;
  version?: [number, number, number];
  createdAt: string;
  updatedAt: string;
}
```

The core surface fields are:

- `rootComponentId` — entry point into the flat component graph;
- `components` — the A2UI components rendered by the Widget;
- `dataModel` — values that bound component properties can read;
- `actions` — named interactions exposed by the Widget;
- `customizationOptions` and `exposedProps` — stored instance-configuration metadata;
- `version` — the loaded or latest semantic version tuple.

New Widgets start with `rootComponentId: "root"` and empty component, data-model, customization, action, and tag collections.

## Widget Settings

Select **Settings** in the Widget Builder header.

### General

Edit the name, description, and tags, then select **Save Metadata**. These settings describe the Widget in the library; they do not change an instance on a Page.

### Events

Widget events define IDs that elements inside the Widget can select:

```typescript
interface WidgetAction {
  id: string;
  label: string;
  description?: string;
  icon?: string;
  contextSchema: WidgetActionContextField[];
}
```

The current settings editor creates an ID, label, optional description, and an initially empty context schema.

To use an event:

1. Add it in **Widget Settings → Events**.
2. Select a Button or another interactive component.
3. Select the event in the action editor. The stored action name is
   `widget_event`, and `context.actionId` is the Widget event's `id`.
4. Insert the Widget into a Page.
5. Populate the Widget instance's action binding through a host that mounts the instance editor, or through validated JSON tooling.

For example:

```json
{
  "actions": [
    {
      "name": "widget_event",
      "context": {
        "actionId": "acknowledge"
      }
    }
  ]
}
```

Do not put `acknowledge` in the action's `name` field. The current action
handler only resolves a Widget binding when the name is exactly
`widget_event`.

This separates reusable presentation from the Page-specific workflow that
handles it.

### Versions

The Versions tab can:

- load an existing Widget version;
- show version history;
- create a Patch, Minor, or Major snapshot.

Creating a version first saves the current Widget and then records a new version. The editor URL represents a selected version as:

```text
/widget?id=<widgetId>&app=<appId>&version=<major>_<minor>_<patch>
```

Use the version type to communicate compatibility:

| Version type | Use when |
| --- | --- |
| **Patch** | Correcting a compatible implementation detail |
| **Minor** | Adding compatible behavior or options |
| **Major** | Changing behavior in a way existing Pages may need to review |

### Advanced

Advanced displays the Widget ID, root component ID, current version, timestamps, component count, and data-model entry count. These fields are informational in the current panel.

## Saving

Component changes update local editor state immediately and save after a short debounce. The header shows **Saving**, **Unsaved changes**, or **Saved** and offers **Save Now** when changes are pending.

Metadata and Widget events are saved explicitly from their Settings tabs.

## Insert a Widget into a Page

The component palette loads Widgets available to the user and groups them by project. Drag a Widget onto a compatible Page container.

The builder then:

1. loads the Widget definition;
2. stores it in the Page's `widgetRefs` under a new instance ID;
3. creates a `widgetInstance` component;
4. adds that component ID to the selected parent;
5. initializes empty exposed-property values and action bindings.

The Page owns the instance configuration while retaining a reference to the reusable Widget definition.

The inserted component currently has this shape:

```json
{
  "type": "widgetInstance",
  "instanceId": "widget-status-card-…",
  "widgetId": "status-card",
  "appId": "app-id",
  "exposedPropValues": {},
  "actionBindings": {}
}
```

At runtime, the renderer looks up the Widget by the instance ID in the Page's `widgetRefs`. If no inline reference exists, it can fetch the Widget by app and Widget ID. It applies `exposedPropValues` to the definition's exposed properties and provides `actionBindings` to the Widget's action context. For `widget_event`, the current handler looks up `actionBindings[actionId]` and executes workflow bindings; command bindings are not executed by this path.

:::note[Current editor boundary]
The repository exports a dedicated `WidgetInstanceInspector` with customization and workflow/command binding controls, but the current Page Builder does not mount that inspector. The normal insertion flow therefore creates empty instance values and bindings. Do not document those controls as available in the Page Builder until the host wires them in.
:::

## State API

```typescript
interface IWidgetState {
  getWidgets(
    appId: string,
    language?: string,
  ): Promise<[string, string, IMetadata | undefined][]>;
  getWidget(
    appId: string,
    widgetId: string,
    version?: [number, number, number],
  ): Promise<IWidget>;
  createWidget(
    appId: string,
    widgetId: string,
    name: string,
    description?: string,
  ): Promise<IWidget>;
  updateWidget(appId: string, widget: IWidget): Promise<void>;
  deleteWidget(appId: string, widgetId: string): Promise<void>;
  createWidgetVersion(
    appId: string,
    widgetId: string,
    versionType: "Major" | "Minor" | "Patch",
  ): Promise<[number, number, number]>;
  getWidgetVersions(
    appId: string,
    widgetId: string,
  ): Promise<[number, number, number][]>;
}
```

Metadata has separate `getWidgetMeta` and `pushWidgetMeta` operations so library copy can be localized independently of the Widget surface.

## Network access and CSP

This section covers **package micro widgets**, not the A2UI Widgets described above. Micro widgets are built with `@flow-like/widget-sdk`, packed into a package's `widgets.flwb` with `flow-like-widgets pack`, and talk to Flow-Like only through their typed contract.

A micro widget runs in a sandboxed iframe with an opaque origin. By default its document has no network access. It can load its own bundle files and `data:` and `blob:` URLs, and nothing else. A widget can ask for more by declaring Content Security Policy (CSP) extensions. A declaration has no effect until the person viewing the widget approves it.

### Declare network sources

Declare sources per widget in `widget.config.ts`:

```ts
import { defineWidget } from "@flow-like/widget-sdk";

export default defineWidget({
  id: "live-map",
  name: "Live map",
  description: "Map with live vehicle positions",
  capabilities: { workers: true },
  csp: {
    connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
    imgSrc: ["https://a.tile.openstreetmap.org", "https://b.tile.openstreetmap.org"],
    styleSrc: ["https://fonts.googleapis.com"],
    fontSrc: ["https://fonts.gstatic.com"],
  },
});
```

Five keys can be extended:

| Key | CSP directive | Schemes | Covers |
| --- | --- | --- | --- |
| `connectSrc` | `connect-src` | `https`, `wss` | `fetch`, `XMLHttpRequest`, WebSocket, EventSource |
| `imgSrc` | `img-src` | `https` | Images |
| `fontSrc` | `font-src` | `https` | Web fonts |
| `mediaSrc` | `media-src` | `https` | Audio and video |
| `styleSrc` | `style-src` | `https` | Stylesheets |

No other directive can be extended. That includes `default-src`, `script-src`, `worker-src`, `child-src`, `frame-src`, `object-src`, `manifest-src`, `base-uri`, `form-action` and the `sandbox` flags. Remote scripts would replace reviewed bundle code at runtime. An approved third-party frame would get its own network context outside the widget's policy. Workers, WebAssembly, audio playback, microphone and downloads stay under `capabilities`.

The bundler reads `csp` statically, so each directive must be an array of string literals.

### Source rules

A source is an exact origin with nothing after the host:

```text
source = scheme "://" host
host   = label *( "." label )     ; at least 2 labels, at most 253 characters
label  = [a-z0-9] ( [a-z0-9-]{0,61} [a-z0-9] )?
tld    = [a-z]{2,63} / "xn--" [a-z0-9-]{1,59}   ; never numeric
```

Before validating, the bundler lowercases each source, converts internationalized hosts to punycode, and sorts and deduplicates each list. Hubs and desktop hosts do not normalize. They accept a source exactly as written in `contract.json`, or reject it, so a hand-written contract with uppercase letters fails.

A source is rejected when it has any of the following:

- a wildcard (`*`, `https://*.example.com`);
- a port (including `:443`), a path, a query, a fragment or userinfo;
- a keyword, nonce or hash, such as `'self'`;
- only a scheme (`https:`, `data:`), or the `http://` or `ws://` scheme;
- an IP address;
- non-ASCII characters, whitespace, `;`, `,` or quotes;
- a host that is, or ends in, `localhost`, `local`, `internal`, `lan`, `home.arpa`, `test`, `example`, `invalid` or `onion`.

A widget can declare at most 16 sources across all directives. A rejected source fails the build and names the widget, the directive and the reason:

```text
Invalid widget csp source "https://api.maptiler.com:443" in connectSrc for widget live-map: ports are not allowed
```

Hosts also reject sources that point back at the deployment itself. On web these are the hub domain, the address the app is served from, and configured frontend origins. On desktop they are the hub host of every profile. A matching host, or a subdomain of one, makes the **whole** policy invalid. The widget then runs without any extensions, and the viewer sees a notice that its permissions could not be verified. Sources are never dropped one at a time.

### Contract version 2

The bundler writes the declaration into `contract.json`:

```json
{
  "contractVersion": 2,
  "id": "live-map",
  "capabilities": { "workers": true },
  "csp": {
    "connectSrc": ["https://api.maptiler.com", "wss://live.example.com"],
    "imgSrc": ["https://a.tile.openstreetmap.org", "https://b.tile.openstreetmap.org"],
    "fontSrc": ["https://fonts.gstatic.com"],
    "styleSrc": ["https://fonts.googleapis.com"]
  }
}
```

- `contractVersion` is `2` exactly when `csp` is present, and `1` otherwise. The Rust and TypeScript validators enforce both directions.
- Each list is sorted, deduplicated and non-empty. Empty lists are left out, and a declaration with no sources leaves out `csp` entirely.
- `csp` rejects unknown keys, so `scriptSrc` or `frameSrc` fails to parse.

The version bump is deliberate. Hubs, desktop builds and bundlers released before `csp` support reject a version 2 contract with an explicit error, instead of silently dropping the declaration. Widgets without `csp` stay at version 1 and keep working everywhere. An older `@flow-like/widget-bundler` does not know `csp` and drops it, so the widget ends up with no network access. Update the bundler if a declaration seems to have no effect.

### Consent

The host decides how to mount a widget like this:

1. It asks its backend for the widget's policy. Desktop reads the `contract.json` of the installed bundle. Web reads the published package version. The contract copy stored in page JSON is never used for consent.
2. A widget that declares no capabilities and no `csp` mounts right away, without a prompt.
3. Otherwise the viewer sees a dialog. When the widget declares `csp`, the dialog is titled **Widget requests network access**. It lists every host, grouped by what the host is allowed to do (send and receive data, load images, load fonts, stream audio and video, load stylesheets), and shows where the package comes from.
4. The viewer chooses:
   - **Allow this time** keeps the approval for the current session.
   - **Always allow for this project** stores the approval on this device for the current app. It replaces any earlier approval for the widget, so hosts that a newer version dropped do not linger.
   - **Block widget** clears any approval. The blocked card offers **Review** and **Run without these permissions**, which mounts the widget with no extensions.
5. After approval, the host requests a short-lived grant and loads the widget from a URL that carries it. The backend derives the policy again from the bundle and never accepts a policy from the caller. If the policy changed since the dialog was shown, the backend refuses the grant and the viewer is asked again.

A widget update that adds a capability or a host prompts again. Removing one does not. A code-only update that keeps the same hosts does not prompt, because approvals are bound to the package, widget and hosts, not to the bundle hash.

Approvals belong to one viewer on one device. Project admins cannot approve hosts on behalf of their users.

To revoke an approval, use the provenance header of the widget in the Inspector or the **Widget permissions** sheet on the app's packages page. **Clear widget permissions on this device** under **Settings → Registry** (developer mode) revokes every approval at once. Revoking unmounts the widget in every open tab.

| | Desktop | Web |
| --- | --- | --- |
| Grant | Random id held in memory by the app | Signed token |
| Lifetime | 24 hours | 1 hour |
| Revocation | Immediate, also on uninstall | Cannot recall issued tokens. They expire within an hour and only work for the exact widget document they were issued for. |

If the server does not support widget policies yet, a widget with `csp` shows a card asking for a newer server. Widgets without `csp` keep loading as before.

### Previews run without network access

Widget previews, such as the live widget cards in the store, have `csp`, `media` and `microphone` stripped on the server before a grant is issued. A preview never reaches the network, even if the viewer approved the same widget in an app. Test network features in a real mount: a page, or **Developer → Test Widgets** on desktop.

### How the policy is enforced

The response that delivers the widget document enforces the policy. The dialog only decides which policy that response carries.

- **Wrapper.** The host frames `frame/{widgetId}/{grant}`, or `frame/{widgetId}/0` without a grant. The wrapper's CSP pins `frame-src` to the single document URL that the grant allows. The wrapper attaches the widget iframe inside a closed shadow root, so no other frame can reach the widget through `window.frames`.
- **Document.** `widgets/{widgetId}/index.{grant}.html` is served with a CSP header built from the approved policy. Without a valid grant, `index.0.html` gets the baseline: no hosts and no capabilities.
- **Everything else** in the bundle (chunks, SVGs, other HTML files) is served with `default-src 'none'; sandbox`.
- **Top-level visits.** On web, opening a wrapper or widget document as a page returns 403.

The document policy has this shape. Square brackets mark parts that only appear when approved:

```text
default-src 'none';
script-src 'unsafe-inline' ['wasm-unsafe-eval'] B;
style-src 'unsafe-inline' B [styleSrc];
img-src data: blob: B [imgSrc];
font-src data: B [fontSrc];
connect-src [blob: B if workers] [connectSrc]      ('none' when empty);
worker-src [blob: B if workers]                    ('none' when empty);
media-src [blob: B if media] [mediaSrc]            ('none' when empty);
frame-src 'none'; child-src 'none'; object-src 'none'; manifest-src 'none';
base-uri 'none'; form-action 'none';
sandbox allow-scripts [allow-downloads]
```

`B` is the widget's own bundle prefix and never `'self'`:

- desktop: `flow-widget://localhost/{package}/{hash}/`, or `http://flow-widget.localhost/{package}/{hash}/` on Windows and Android;
- web: `{api origin}/api/v1/registry/package/{package}/widget-sandbox/{version}/`.

**The injected meta.** When a server delivers a widget document, it removes every `<meta http-equiv="Content-Security-Policy">` tag from it. It then inserts one copy of the header policy, without `sandbox` (which a meta tag cannot carry), right after the doctype. This is defense in depth. The fetch directives stay enforced if an edge proxy overwrites the header, or if an engine ignores headers on custom-scheme frames.

The meta that `flow-like-widgets pack` writes is built from `capabilities` only and never lists `csp` hosts. A server that predates `csp` therefore still keeps a version 2 widget away from its hosts. Do not add your own CSP meta to widget HTML: servers replace it. The dev harness enforces no CSP at all, so a request that works under `flow-like-widgets dev` can still be blocked in Flow-Like until it is declared and approved.

:::caution[Self-hosted deployments]
Proxies and CDNs in front of the API must not overwrite or drop `Content-Security-Policy` on `/api/v1/registry/package/*/widget-sandbox/*`. Custom response headers there must be appended, not replaced. The edge must also route raw paths without decoding `%2F`; otherwise a bundle prefix could match other API paths.
:::

### What CSP does not cover

Approving a host is a trust decision. Browsers cannot block every channel, and these remain open:

- **Channels outside CSP.** WebRTC (STUN, TURN and ICE), DNS prefetch, preconnect and legacy prerender are not governed by CSP. In WebView2, WKWebView and browsers, any mounted widget can leak data through them, including a widget without extensions. WebKitGTK has WebRTC turned off by default.
- **Downloads.** With the `downloads` capability, `<a download href="https://…">` can send a request to any site.
- **Channels through the host.** A widget can encode data in the order in which it plays approved media items, and in contract events that flows handle.
- **Approved hosts are full channels.** Anything the widget can see, including values the app passes to it, can be sent to an approved host. Because `'unsafe-inline'` is allowed, the widget can also run text fetched from an approved host. A shared, multi-tenant host amounts to broad network access. Consent covers hosts, not code.
- **DNS rebinding.** An approved public hostname can resolve to a loopback or private network address. No desktop webview blocks this.
- **WebKit scheme matching.** In WebKit, `wss://host` also allows `https://host`.
- **Frames outside Flow-Like.** A widget can `postMessage` to a third-party page that a page author embedded with the generic Iframe component.
- **Cached legacy documents.** Browsers may keep widget documents that older servers delivered without a CSP header for up to a year. No current wrapper frames them.

The `examples/widget-csp-probe` package in the repository checks approved and blocked requests, pinned navigation, frame isolation and IPC reachability inside each real webview.

### Migrate from `--connect`

`flow-like-widgets pack --connect <host>` baked hosts into the packed meta of every widget in the package. Those hosts were not part of any contract and were never shown to viewers. The flag has been removed:

```text
error: The --connect flag was removed: declare network sources per widget in widget.config.ts "csp"
```

Calling `pack()` with the `connectHosts` option fails the same way.

To migrate:

1. Remove `--connect` from `mise.toml` and any build scripts.
2. Add a `csp` block to each widget that needs the network, and list only the hosts that widget uses. A `--connect` host only ever went into `connect-src`, so it becomes a `connectSrc` entry. It must now follow the [source rules](#source-rules): `http://` hosts, ports and paths are rejected. Images, fonts, media and stylesheets from other sites were never allowed before; declare them under their own directives if the widget needs them.
3. Update `@flow-like/widget-bundler` and `@flow-like/widget-sdk`, rebuild, and run `flow-like-widgets validate`.
4. Publish a new version. The affected widgets now carry contract version 2 and need a hub and desktop release with `csp` support. Viewers are asked to approve the hosts the first time they open the widget.

```diff
- bunx flow-like-widgets pack --project . --out widgets.flwb --connect https://api.maptiler.com
+ bunx flow-like-widgets pack --project . --out widgets.flwb
```

```ts
export default defineWidget({
  id: "live-map",
  name: "Live map",
  description: "Map with live vehicle positions",
  csp: { connectSrc: ["https://api.maptiler.com"] },
});
```

Bundles packed with `--connect` still load, but current servers replace their meta, so those hosts stop working until the widget declares them in `csp`.

## Related Guides

- [Pages](/dev/a2ui/pages/) — compose Widgets into app experiences
- [Visual Builder](/dev/a2ui/visual-builder/) — edit the component graph
- [A2UI overview](/dev/a2ui/overview/) — understand surfaces and actions
