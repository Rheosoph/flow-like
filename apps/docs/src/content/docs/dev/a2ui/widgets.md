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

A micro widget runs in a sandboxed iframe with an opaque origin. It can load its own bundle files and use bytes it already holds, and it cannot reach any site on the network. To reach sites, a widget declares Content Security Policy (CSP) sources, grouped by purpose. A declaration has no effect until the person viewing the widget approves it, and every viewer approves for themselves.

### Local data needs no approval

Every widget, with or without a declaration, may load bytes it already holds from `data:` and `blob:` URLs:

| Directive | `data:` | `blob:` |
| --- | --- | --- |
| `connect-src` | `fetch("data:…;base64,…")` into an `ArrayBuffer`, `Blob` or JSON; XHR; JSON and text module imports | `fetch(URL.createObjectURL(blob))`, for loaders that only take URLs (three.js, pdf.js, MapLibre `addProtocol`, hls.js on Media Source Extensions) |
| `media-src` | `<audio>` and `<video>` with `data:` sources, `<track src="data:text/vtt,…">` | object URLs of received bytes, `URL.createObjectURL(mediaSource)` for MSE players |
| `img-src` | yes | yes |
| `font-src` | yes | no |

These schemes never reach the network, need no approval, and are never part of the declaration. A `csp` list that contains `data:`, `blob:` or `filesystem:` fails the build. Scripts, frames and workers stay limited to the bundle: `blob:` workers need the `workers` capability, and WebAssembly needs `wasm`.

Pass bytes as base64 in a `string` input or a field of a `json` input. Base64 adds a third, and on web every value travels through the run's event stream, page state and every later props update. Inline bytes suit icons, small GeoJSON and short sounds, up to a few hundred KB that rarely change. Load anything larger or frequently updated from the customer's host through a [runtime input](#addresses-known-only-at-runtime). `blob:` URLs die with the document, so a remount invalidates them. The host never passes its own `blob:` URLs into a widget; it sends `Blob` or `ArrayBuffer` values.

### Declare purpose groups

Declare network sources per widget in `widget.config.ts`, as a list of purpose groups. Each group says why the widget needs the network (`reason`) and which sources it may use: fixed addresses, widget inputs that carry addresses at runtime, or both.

```ts
import { defineWidget } from "@flow-like/widget-sdk";

export default defineWidget({
  id: "live-map",
  name: "Live map",
  description: "Map with live vehicle positions",
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

The bundler reads `csp` statically, so every group must be an object literal and every value a string literal. A group can hold these keys:

| Key | CSP directive | Schemes | Covers |
| --- | --- | --- | --- |
| `reason` | | | One sentence the viewer reads (required) |
| `connectSrc` | `connect-src` | `https`, `wss` | `fetch`, `XMLHttpRequest`, WebSocket, EventSource |
| `imgSrc` | `img-src` | `https` | Images |
| `fontSrc` | `font-src` | `https` | Web fonts |
| `mediaSrc` | `media-src` | `https` | Audio and video |
| `styleSrc` | `style-src` | `https` | Stylesheets |
| `inputs` | | | Widget inputs whose values carry addresses ([Addresses known only at runtime](#addresses-known-only-at-runtime)) |

No other directive can be extended. That includes `default-src`, `script-src`, `worker-src`, `child-src`, `frame-src`, `object-src`, `manifest-src`, `base-uri`, `form-action` and the `sandbox` flags. Remote scripts would replace reviewed bundle code at runtime. An approved third-party frame would get its own network context outside the widget's policy. Workers, WebAssembly, audio playback, microphone and downloads stay under `capabilities`.

| Limit | Value |
| --- | --- |
| Purpose groups | 1–8 |
| Sources across all groups, counted once per directive they appear under | 16 |
| Source bytes, Σ(length + 1) | 1536 |
| Each group | at least one source or one input |
| A source | belongs to one group; it may appear under several directives of that group |
| Network inputs across all groups | 8, each path once |

The enforced policy is the union of all groups per directive. Groups, reasons and levels are for display only: they never change what the widget may reach, and editing a reason alone does not ask viewers again.

### Reasons

The viewer sees the reason last on each group's card, labelled **Publisher:**, below what Flow-Like says about the sources. Write one short sentence about what the widget does with the network. The bundler normalizes it (NFC, runs of whitespace become one space, trimmed); hubs and desktop hosts accept or reject it without normalizing. A reason is rejected when it:

- is not 8 to 120 characters long, or has fewer than 3 letters;
- uses anything other than letters, combining marks, numbers, spaces and `, . : ; ( ) - ' / &` (no quotes, emoji, `!`, `?`, `%` or `_`);
- mixes ASCII letters with letters of another script in one word, such as a Greek omicron in "Flοw";
- contains a web address, an email address or a domain name (`cesium.com`, `www.`, `https://`); "Node.js" and "e.g." are fine;
- mentions Flow-Like, or claims that sources are verified, trusted, approved, official, certified, secure or safe;
- in a group with `inputs`, claims who supplies the addresses (app, admin, organization, company, workspace, project, owner and similar words);
- reads the same as another group's reason.

The address rule needs the Public Suffix List, so it runs in the bundler and at hub publish only.

### Sources

A source is `scheme://host` or `scheme://*.host`, with nothing after the host:

```text
source = scheme "://" [ "*." ] host
host   = label *( "." label )     ; at least 2 labels, at most 253 characters
label  = [a-z0-9] ( [a-z0-9-]{0,61} [a-z0-9] )?
tld    = [a-z]{2,63} / "xn--" [a-z0-9-]{1,59}   ; never numeric
```

Before validating, the bundler lowercases each source, converts internationalized hosts and wildcard bases to punycode, and sorts and deduplicates each list. Hubs and desktop hosts do not normalize. They accept a source exactly as written in `contract.json`, or reject it, so a hand-written contract with uppercase letters fails.

A source is rejected when it has any of the following:

- a `*` anywhere but a leading `*.` label (`*`, `https://*`, `https://a.*.example.com`, `https://*a.example.com`);
- a port (including `:443`), a path, a query, a fragment or userinfo;
- a keyword, nonce or hash, such as `'self'`;
- only a scheme (`https:`, `data:`, `blob:`), or the `http://` or `ws://` scheme;
- an IP address;
- non-ASCII characters, whitespace, `;`, `,` or quotes;
- a host that is, or ends in, `localhost`, `local`, `internal`, `lan`, `home.arpa`, `test`, `example`, `invalid` or `onion`, or an IP-mapping name (`nip.io`, `sslip.io`, `traefik.me`, `localtest.me`, `lvh.me`, `localhost.direct`);
- a wildcard whose base is a public suffix or spans one (`https://*.co.uk`, `https://*.kawasaki.jp`). A base with a single label, such as `https://*.com`, is not a valid host at all.

A rejected source fails the build and names the widget, the group, the directive and the reason:

```text
Invalid widget csp source "https://api.maptiler.com:443" in csp[0].connectSrc for widget live-map: ports are not allowed
```

Hosts also refuse sources that point back at the deployment. On web these are the hub domain, the address the app is served from, configured frontend origins, and the names listed in the API's `WIDGET_RESERVED_HOSTS` (for example a CloudFront distribution domain or a Lambda Function URL of a self-hosted deployment). On desktop they are the hub host of every profile. A matching host, a subdomain of one, or a declared wildcard that covers one makes the **whole** policy invalid. The widget then runs without any extensions, and the viewer sees a notice that its permissions could not be verified. Declared sources are never dropped one at a time.

### Wildcards

`https://*.tiles.example.org` matches every host that ends in `.tiles.example.org`, at any depth, and never `tiles.example.org` itself; declare that host separately when the widget needs it. Only declared sources can be wildcards. Addresses from runtime inputs are always exact hosts.

Prefer exact hosts. Every engine handles them the same way, while an engine version whose wildcard matching failed the probe is served the policy without its wildcard sources (see **Engine gates** under [How the policy is enforced](#how-the-policy-is-enforced)). The consent dialog then says the browser cannot allow them.

### Warning levels

Flow-Like classifies every source, declared or runtime, from the Public Suffix List and a curated catalog of services compiled into the desktop app and the API. Publishers cannot influence the level. Levels say how many unrelated parties can be on the other end, not what the publisher intends.

| Level | Label | Meaning | Examples | Presentation |
| --- | --- | --- | --- | --- |
| `known` | Identified service | One named operator; no account holder can receive requests or publish content there. | `https://gibs.earthdata.nasa.gov`, `https://tile.googleapis.com` | Neutral |
| `external` | External site | One party receives, and Flow-Like cannot verify who: an unknown site, one customer's space on shared hosting, or a named service whose account holders can receive through its API. | `https://tiles.customer-maps.com`, `https://mybucket.s3.eu-central-1.amazonaws.com`, `https://api.cesium.com` | Neutral |
| `shared` | Hosts user content | Many parties publish content there; they cannot receive requests. | `https://assets.ion.cesium.com`, `https://cdn.jsdelivr.net`, `https://*.github.io` | Neutral, with an info icon |
| `broad` | Anyone can receive | Open hosting where anyone who signs up can receive what the widget sends, request-capture services and tunnels. | `https://*.s3.eu-central-1.amazonaws.com`, `https://s3.eu-central-1.amazonaws.com`, `https://storage.googleapis.com`, `https://*.pages.dev`, `https://webhook.site` | Destructive |

Each group shows its highest level, and the dialog takes the highest level of all groups. The `downloads` capability counts as `broad`, because a download can be fetched from any site.

- **Up to `shared`,** the dialog stays calm: a neutral lead line, **Allow this time** as the primary button, and one sentence per card, such as "Data goes to Cesium ion." or "Cesium ion hosts content its users upload." The longer explanations (who could receive, who could publish) are in **Details**.
- **Only `broad`** uses destructive styling: a destructive banner, **Don't allow** focused, and an inline confirmation before **Always allow for this project**.
- **Every network dialog** ends its lead with "Anything the widget can see, including values this app gives it, can leave your device."

Avoid `broad` when a narrower source works: the bucket's own host instead of the regional endpoint or a bucket wildcard, or an input that carries the bucket's URL.

Classification data can change between releases. A source whose level rises asks the viewer again once; a lower level does not. A build whose Public Suffix List is more than 180 days old fails closed: it raises wildcards under a single site or a single customer's space to `broad`.

### Addresses known only at runtime

Some addresses exist only while the app runs: a customer's CDN, a bucket picked in a flow, signed URLs that expire. Declare the inputs that carry them instead of the addresses:

```ts
interface Inputs {
  /** Tile URL template, e.g. https://{s}.tiles.customer-maps.com/{z}/{x}/{y}.png @default "" */
  tileUrl: string;
  /** Characters substituted for {s} @default "abc" */
  tileSubdomains: string;
}

export default defineWidget<Inputs>({
  id: "tile-map",
  name: "Tile map",
  description: "Map with tiles from the customer's tile server",
  csp: [
    {
      reason: "Loads map tiles from tile servers given to it at runtime",
      inputs: [
        {
          path: "tileUrl",
          directives: ["imgSrc", "connectSrc"],
          template: { subdomainsInput: "tileSubdomains" },
        },
      ],
    },
  ],
});
```

- `path` is `root *( "." key / "[]" / ".*" )`, at most 6 segments. The root is a widget input: a `string` input must be the whole path, a `json` input may descend with `.key`, `[]` (array items) and `.*` (map values). The path must reach `type: string` in the input's generated JSON Schema, and `.*` needs a `Record<string, …>`. Number, boolean and enum inputs cannot carry addresses; declare fixed choices as static sources. `publicMediaGrants` is reserved by the host.
- `directives` lists what the addresses may be used for. Players built on hls.js or MSE fetch playlists and segments through `connectSrc`; only native HLS uses `mediaSrc`. The build warns when an input feeds `mediaSrc` without `connectSrc`.
- `@default` values are merged inside the widget, so the host never sees or approves them. The build warns when a default URL's host is not declared as a static source.

**What the host extracts.** Flow-Like's host reads the input values it forwards to the widget, from page JSON, flow updates or **Update Widget Inputs**, and keeps only `scheme://host` of each URL:

- The scheme must be `https` or `wss`. Hosts are converted to lowercase punycode.
- Paths, queries, fragments and signatures are dropped before anything leaves the viewer's device. Rotating a signed URL on the same host therefore needs no approval and no remount.
- `data:`, `blob:` and relative values are skipped. Ports, userinfo, IP addresses, trailing dots, protocol-relative values, other schemes (including desktop `asset://` URLs), values over 8192 characters, reserved and local-only names, and hosts of the deployment itself are rejected one by one. The widget shows "N addresses given to this widget can't be allowed" with the reason per host, never the full URL.
- At most 8 addresses per input and 8 in total. More than that and the input, or all inputs, contribute nothing.

**Templates.** Without `template`, a `{` in the host is rejected. With it, `{s}` expands from a fixed list (`template: { subdomains: ["a", "b", "c"] }`) or from another `string` or `json` input (`subdomainsInput`; a string gives its characters, a string array its items). Ranges such as `{a-c}` and `{0-9}` and lists such as `{switch:a,b}` expand as well, up to 16 hosts per value. Any other placeholder in the host, such as `{tenant}`, is rejected: runtime wildcards do not exist.

**This app's Flow-Like storage.** A URL into the deployment's content storage, for example from the **Sign URLs** node, is not approved as a host, because that host serves every app. The host turns it into a path scope for this app's files only, such as `https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/{appId}/`. The shape follows the deployment's content bucket (virtual-hosted or path-style S3, a custom S3 endpoint, Google Cloud Storage, Azure Blob Storage) and is published by the hub. The scope classifies as `known` and is shown as "this app's files". It needs a project: a widget shown outside a project cannot use it, and a URL outside `/apps/{appId}/` on the storage host is rejected. CSP path matching confines the requests; browsers ignore paths after a redirect, which storage services do not issue inside a correctly addressed container.

**Approval.** The backend validates and classifies the extracted addresses again. The viewer then approves them:

- On first mount they appear in the consent dialog inside their group's card, marked **Provided while the app runs**, with **Also allow the N addresses provided while the app runs**. The box is checked by default up to `external` and unchecked for `shared` and `broad`.
- A new address while the widget runs shows a non-modal banner, **Wants to load from N new sites**, with **Review**. The widget keeps running without it until the viewer allows it; allowing reloads the widget, because a document's CSP cannot change. **Don't allow** blocks the addresses for this session. **Stop asking for this widget** stops asking until **Ask again** in the widget permissions.
- Approvals are stored only on the viewer's device: for the project with **Always allow for this project**, otherwise for the session. They never expire. **Revoke**, **Don't allow** and **Clear widget permissions on this device** are the only ways an approval ends. Each widget keeps at most 64 runtime addresses; the least recently used drops out.
- No approval is stored on a server, and nobody can approve for another viewer: not the publisher, not the app's admins.

**How approved addresses reach the document.** Desktop keeps the effective policy in its in-memory grant registry. On web, the approved slots travel in the frame URL, `frame/{widgetId}/{token}~{runtime}`, where `runtime` is the base64url canonical JSON of the slots (at most 1 KB before encoding). The signed token binds the digest of that component, the wrapper pins the exact URL, and the document route recomputes the digest from the path. A missing or altered component falls back to the declared sources. Nothing is written on the server, so anonymous viewers of public apps can use runtime addresses too.

At most 8 runtime sources are accepted, and the served document CSP stays under 3584 bytes. Beyond that, or on a server that cannot carry runtime addresses, the widget runs with its declared sources and says so.

### Contract version 2

The bundler writes the canonical declaration into `contract.json`:

```json
{
  "contractVersion": 2,
  "id": "live-map",
  "capabilities": { "workers": true },
  "csp": [
    {
      "reason": "Loads map tiles and live vehicle positions",
      "connectSrc": ["https://api.maptiler.com", "wss://live.example.com"],
      "imgSrc": ["https://a.tile.openstreetmap.org", "https://b.tile.openstreetmap.org"]
    },
    {
      "reason": "Loads web fonts for map labels",
      "fontSrc": ["https://fonts.gstatic.com"],
      "styleSrc": ["https://fonts.googleapis.com"]
    }
  ]
}
```

- `contractVersion` is `2` exactly when `csp` is present, and `1` otherwise. The Rust and TypeScript validators enforce both directions.
- Groups keep their declaration order, which is display order only. Keys follow `reason`, `connectSrc`, `imgSrc`, `fontSrc`, `mediaSrc`, `styleSrc`, `inputs`. Source lists are sorted bytewise (so `https://*.…` comes first), deduplicated and left out when empty. `inputs` is sorted by `path`, and each input's `directives` follow the table order above.
- `csp: []` is invalid; a widget without network sources leaves `csp` out. Groups reject unknown keys, so `scriptSrc` or `frameSrc` fails to parse.

The version bump is deliberate. Hubs, desktop builds and bundlers released before `csp` support reject a version 2 contract with an explicit error, instead of silently dropping the declaration. Widgets without `csp` stay at version 1 and keep working everywhere. An older `@flow-like/widget-bundler` does not know `csp` and drops it, so the widget ends up with no network access. Update the bundler if a declaration seems to have no effect.

### Consent

The host decides how to mount a widget like this:

1. It asks its backend for the widget's policy. Desktop reads the `contract.json` of the installed bundle. Web reads the published package version. The contract copy stored in page JSON is never used for consent.
2. A widget that declares no capabilities and no `csp` mounts right away, without a prompt.
3. Otherwise the viewer sees a dialog, one at a time when several widgets on a page ask. It names the widget, the package and where the package comes from. It shows one card per purpose group: the level, what the sources may be used for, every address in monospace (a wildcard marked as "any address under"), one sentence from Flow-Like about the addresses, and last the publisher's reason. Other capabilities follow under **Also asks for**. **Details** lists every address with its directives, level and explanation, and the effective sandbox policy.
4. The viewer chooses:
   - **Allow this time** keeps the approval for the current session.
   - **Always allow for this project** stores the approval on this device for the current app. It replaces any earlier approval for the widget, so hosts that a newer version dropped do not linger. It is only offered inside a project, and at `broad` it asks for a confirmation first.
   - **Don't allow** clears any approval. The blocked card offers **Review** and **Run without these permissions**, which mounts the widget with no extensions.
5. After approval, the host requests a short-lived grant and loads the widget from a URL that carries it. The backend derives the policy again from the bundle, together with the runtime addresses the viewer approved, and never accepts a policy from the caller. If the policy changed since the dialog was shown, the backend refuses the grant and the viewer is asked again.

A widget update that adds a capability or a source, or a source whose level rises, prompts again. Removing one does not. A code-only update that keeps the same sources does not prompt, because approvals are bound to the package, widget and sources, not to the bundle hash.

Approvals belong to one viewer on one device. Project admins cannot approve sources on behalf of their users.

To revoke an approval, use the provenance header of the widget in the Inspector or the **Widget permissions** sheet on the app's packages page. **Clear widget permissions on this device** under **Settings → Registry** (developer mode) revokes every approval at once. Revoking unmounts the widget in every open tab.

| | Desktop | Web |
| --- | --- | --- |
| Grant | Random id held in memory by the app | Signed token, plus the runtime component in the URL |
| Lifetime | 24 hours, renewed when the same grant is reused | 1 hour |
| Revocation | Immediate, also on uninstall | Cannot recall issued tokens. They expire within an hour and only work for the exact widget document they were issued for. |

The grant only carries what the viewer approved. The approval itself never expires; the host mints a new grant when one runs out.

If the server does not support widget policies yet, a widget with `csp` shows a card asking for a newer server. Widgets without `csp` keep loading as before.

Before install, the store shows each widget's network summary on its card and the purpose cards in the package's **Permissions** tab, including the inputs that can add addresses at runtime.

### Previews run without network access

Widget previews, such as the live widget cards in the store, have `csp`, runtime inputs, `media` and `microphone` stripped on the server before a grant is issued. A preview never reaches the network, even if the viewer approved the same widget in an app. Local `data:` and `blob:` bytes still work. Test network features in a real mount: a page, or **Developer → Test Widgets** on desktop.

### How the policy is enforced

The response that delivers the widget document enforces the policy. The dialog only decides which policy that response carries.

- **Wrapper.** The host frames `frame/{widgetId}/{grant}`, `frame/{widgetId}/{grant}~{runtime}` on web with runtime addresses, or `frame/{widgetId}/0` without a grant. The wrapper's CSP pins `frame-src` to the single document URL that the grant allows. The wrapper attaches the widget iframe inside a closed shadow root, so no other frame can reach the widget through `window.frames`.
- **Document.** `widgets/{widgetId}/index.{grant}.html` is served with a CSP header built from the approved policy. Without a valid grant, `index.0.html` gets the baseline: local bytes only, no hosts and no capabilities.
- **Everything else** in the bundle (chunks, SVGs, other HTML files) is served with `default-src 'none'; sandbox`.
- **Top-level visits.** On web, opening a wrapper or widget document as a page returns 403.

The document policy has this shape. Square brackets mark parts that only appear when approved:

```text
default-src 'none';
script-src 'unsafe-inline' ['wasm-unsafe-eval'] B;
style-src 'unsafe-inline' B [styleSrc];
img-src data: blob: B [imgSrc];
font-src data: B [fontSrc];
connect-src data: blob: [B if workers] [connectSrc];
worker-src [blob: B if workers]                    ('none' when empty);
media-src data: blob: [B if media] [mediaSrc];
frame-src 'none'; child-src 'none'; object-src 'none'; manifest-src 'none';
base-uri 'none'; form-action 'none';
sandbox allow-scripts [allow-downloads]
```

`[connectSrc]` and the other lists are the declared sources of all groups plus the approved runtime addresses. `B` is the widget's own bundle prefix and never `'self'`:

- desktop: `flow-widget://localhost/{package}/{hash}/`, or `http://flow-widget.localhost/{package}/{hash}/` on Windows and Android;
- web: `{api origin}/api/v1/registry/package/{package}/widget-sandbox/{version}/`.

**Engine gates.** Engines differ in how they enforce CSP. `packages/wasm/schema/data/widget_engine_gates.json` lists engine versions that failed a check of the `examples/widget-csp-probe` package. The file ships empty, and an engine that matches no row gets every feature. A matching row can drop wildcard sources, runtime addresses, `data:` and `blob:` from `media-src`, or all declared sources from the served policy. It never adds anything. Desktop evaluates it once for its webview, web per request from the `User-Agent`. The policy description reports what the engine supports, so the dialog never asks for something that will be dropped. Desktop webview versions and browser versions are on different scales; the probe's README explains how to write rows for each.

**The injected meta.** When a server delivers a widget document, it removes every `<meta http-equiv="Content-Security-Policy">` tag from it. It then inserts one copy of the header policy, without `sandbox` (which a meta tag cannot carry), right after the doctype. This is defense in depth. The fetch directives stay enforced if an edge proxy overwrites the header, or if an engine ignores headers on custom-scheme frames.

The meta that `flow-like-widgets pack` writes is built from `capabilities` only and never lists `csp` sources. A server that predates `csp` therefore still keeps a version 2 widget away from its hosts. Do not add your own CSP meta to widget HTML: servers replace it. The dev harness enforces no CSP at all, so a request that works under `flow-like-widgets dev` can still be blocked in Flow-Like until it is declared and approved.

:::caution[Self-hosted deployments]
Proxies and CDNs in front of the API must not overwrite or drop `Content-Security-Policy` on `/api/v1/registry/package/*/widget-sandbox/*`. Custom response headers there must be appended, not replaced. The edge must also route raw paths without decoding `%2F`; otherwise a bundle prefix could match other API paths. List the deployment's other public names (CDN distribution, function URLs, API gateway endpoints) in `WIDGET_RESERVED_HOSTS` so no widget can declare them.
:::

### Recipes

#### Cesium ion

CesiumJS loads imagery through XHR into an `ImageBitmap` and falls back to `<img>`, so tile hosts need both `connectSrc` and `imgSrc`. Its image feature probe fetches a `data:` URL, which every widget may do. All hosts below send `access-control-allow-origin: *`.

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

- That is 13 of the 16 sources, all exact hosts. `https://*.tiles.virtualearth.net` (also `known`) replaces the eight `ecn.t*` entries with two.
- `api.cesium.com` is `external`, because account holders can write assets with their own token. `assets.ion.cesium.com` is `shared`, because it serves user uploads. The Bing and Google hosts are `known`. The dialog stays calm.
- Drop the groups for ion assets you don't use.
- Cesium starts workers through a `blob:` shim when the worker URL is cross-origin, and `blob:` workers inherit the document policy. Bundle-served worker scripts get the asset CSP (no fetch, no WebAssembly), so libraries must use `blob:` workers. Confirm in a real mount.

#### NASA GIBS

Tiles live at `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{Layer}/default/{Time}/GoogleMapsCompatible_Level{N}/{z}/{y}/{x}.jpg`.

```ts
csp: [{ reason: "Loads satellite imagery tiles from NASA GIBS",
        imgSrc: ["https://gibs.earthdata.nasa.gov"],
        connectSrc: ["https://gibs.earthdata.nasa.gov"] }],
```

The level is `known`, and the card only says "Data goes to NASA GIBS." Add `gibs-a`, `gibs-b` and `gibs-c.earthdata.nasa.gov` only if the map client shards requests.

#### S3, GCS and Azure signed URLs

Declare an input and let a flow sign the URLs and update it:

```ts
csp: [{ reason: "Shows map layers from storage files given to it at runtime",
        inputs: [{ path: "layers[].url", directives: ["imgSrc", "connectSrc"] }] }],
```

| URL the flow produces | Approved source | Level |
| --- | --- | --- |
| `https://acme-tiles.s3.eu-central-1.amazonaws.com/t/1/2/3.png?X-Amz-…` (virtual-hosted, regional) | `https://acme-tiles.s3.eu-central-1.amazonaws.com` | `external` (one bucket) |
| `https://acme-tiles.s3.dualstack.eu-central-1.amazonaws.com/…` | the same form | `external` |
| `https://s3.eu-central-1.amazonaws.com/acme-tiles/…` (path-style) | `https://s3.eu-central-1.amazonaws.com` | `broad`: every bucket in the region |
| `https://acme-tiles.s3.amazonaws.com/…` (legacy global host) | `https://acme-tiles.s3.amazonaws.com` | `external`, but S3 answers with a redirect (older regions) or 400 (regions launched after March 2019), and CSP checks the redirect target, which was never approved. Use the regional host. |
| `https://acme-tiles.storage.googleapis.com/…` (sign against this host) | `https://acme-tiles.storage.googleapis.com` | `external` |
| `https://storage.googleapis.com/acme-tiles/…?X-Goog-…` (path-style, the default) | `https://storage.googleapis.com` | `broad` |
| `https://acmetiles.blob.core.windows.net/tiles/…?sv=…&sig=…` | `https://acmetiles.blob.core.windows.net` | `external` |
| **Sign URLs** node on this app's own storage | `…/apps/{appId}/` of the content bucket | `known`, this app's files only |

- **Signing style.** Sign virtual-hosted style (the bucket in the host) against the regional endpoint. Path-style signing, the default of some SDKs, produces the shared regional host, which is `broad`. Bucket names with dots break TLS on virtual-hosted hosts; avoid them for widget content.
- **CORS.** Bucket CORS must allow `GET` from origin `*`. The sandboxed widget sends `Origin: null`, and allowing `null` is unsafe. The signature stays the capability.
- **Rotation.** Re-sign before expiry, for example 60 minutes signed and refreshed at 50. The host stays the same, so there is no prompt and no remount.
- **Redirects.** Don't redirect from an API to a presigned URL on another host. The redirect target is checked against CSP, and the viewer never approved it.

#### CloudFront signed URLs on a customer domain

- **Address.** `https://media.customer.com/video/master.m3u8?Policy=…&Signature=…&Key-Pair-Id=…` becomes `https://media.customer.com` (`external`). The default `d111111abcdef8.cloudfront.net` domain is one customer's space on CloudFront, also `external`.
- **Declaration.** `inputs: [{ path: "videoUrl", directives: ["connectSrc", "mediaSrc"] }]`. hls.js fetches playlists and segments through `connectSrc`, native WebKit HLS through `mediaSrc`.
- **Segments.** Every segment URL must be on an approved host and carry a valid signature. Use a custom policy with a wildcard resource (`https://media.customer.com/video/*`) and append the same query in the player's loader, or sign each playlist entry. Signed cookies do not work: the widget frame is an opaque third-party context.
- **Other hosts.** Playlists that name other hosts cannot be approved at runtime. Declare those hosts statically or pass them as inputs.

#### Customer CDN known only at runtime

Use the `tileUrl` example from [Addresses known only at runtime](#addresses-known-only-at-runtime). Page JSON or a flow sets `tileUrl` to `https://{s}.tiles.customer-maps.com/{z}/{x}/{y}.png` and `tileSubdomains` to `abc`. The host extracts `https://a.tiles.customer-maps.com`, `https://b.…` and `https://c.…`, three exact `external` addresses.

The first mount asks once. A later page of the same project with another CDN host shows the **Wants to load from N new sites** banner. Approved hosts stay approved until the viewer revokes them. `{tenant}` in the host is rejected (no runtime wildcards), and ports, IP addresses, `localhost`, `nip.io` or `sslip.io` names and hosts under the hub domain are rejected per address and listed in the widget's notice.

### What CSP does not cover

Approving a source is a trust decision. Browsers cannot block every channel, and these remain open:

- **Channels outside CSP.** WebRTC (STUN, TURN and ICE), DNS prefetch, preconnect and legacy prerender are not governed by CSP. In WebView2, WKWebView and browsers, any mounted widget can leak data through them, including a widget without extensions. WebKitGTK has WebRTC turned off by default.
- **Downloads.** With the `downloads` capability, `<a download href="https://…">` can send a request to any site.
- **Channels through the host.** A widget can encode data in the order in which it plays approved media items, and in contract events that flows handle.
- **Approved sources are full channels.** Anything the widget can see, including values the app passes to it, can be sent to an approved source. Because `'unsafe-inline'` is allowed, the widget can also run text fetched from one. Consent covers sources, not code.
- **Account-mediated receivers.** A `known` or `external` service can still carry data to the publisher through the publisher's own credentials, for example ion asset writes or per-key usage metrics. Levels describe hosts, not credentials.
- **Heuristic classification.** The Public Suffix List and the catalog cannot be complete. A new multi-tenant service classifies as `external` until the catalog lists it.
- **Runtime addresses come from values the publisher may influence.** Inputs are set by pages and flows, and flows may run the publisher's own nodes. Approving a runtime address may approve a host the publisher chose; the dialog says so.
- **Ownership changes.** Buckets, storage accounts, apps and custom domains can change hands or be taken over. Approvals do not expire, so revoke them when a customer resource is retired.
- **Silent fallback.** When a web document's runtime component is missing, altered or over budget, the widget runs with its declared sources only, and runtime loads fail until the next grant.
- **Remounts.** A new runtime address remounts the widget, which loses its state. Widget frames get no HTTP cache on Chromium engines, so keep in-memory caches.
- **Cookies.** Approved hosts receive the viewer's `SameSite=None` cookies on Chromium engines.
- **Native media pipelines.** HLS and DASH from `data:` and `blob:` rely on engine loaders honoring `media-src`; the probe checks each engine and a failing engine loses local media. Any widget can set the text of the operating system's media controls, and AirPlay and Cast are outside CSP.
- **Addresses discovered after load**, in manifests, stylesheets or redirects, cannot be approved at runtime. CORS with `Origin: null` forces `*` on customer buckets.
- **Storage path scopes** rely on CSP path matching, and browsers ignore paths after a redirect.
- **DNS rebinding.** An approved public hostname can resolve to a loopback or private network address. Web Chromium blocks such requests through Local Network Access; no desktop webview does.
- **WebKit scheme matching.** In WebKit, `wss://host` also allows `https://host`.
- **The web engine gate trusts the `User-Agent`.** A browser that lies about its engine only affects its own viewer.
- **Reason rules** are English-centric word lists. The labelled publisher block below Flow-Like's own text is the real control.
- **Frames outside Flow-Like.** A widget can `postMessage` to a third-party page that a page author embedded with the generic Iframe component.
- **Cached legacy documents.** Browsers may keep widget documents that older servers delivered without a CSP header for up to a year. No current wrapper frames them.

The `examples/widget-csp-probe` package in the repository checks approved and blocked requests, wildcard and runtime sources, local `data:` and `blob:` media against a canary server, pinned navigation, frame isolation and IPC reachability inside each real webview. Its results decide the engine gate rows.

### Migrate from the object form

Prerelease bundlers accepted `csp` as one object of directive lists. Current bundlers, hubs and hosts only accept purpose groups, and the bundler fails with:

```text
Invalid widget csp in widgets/react/src/widgets/live-map/widget.config.ts for widget live-map: must be an array of purpose groups, e.g. csp: [{ reason: "Loads map tiles from MapTiler", connectSrc: ["https://api.maptiler.com"] }]
```

Split the hosts by what the widget uses them for, give every group a reason, and keep each source in exactly one group:

```diff
- csp: {
-   connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
-   imgSrc: ["https://a.tile.openstreetmap.org"],
-   fontSrc: ["https://fonts.gstatic.com"],
-   styleSrc: ["https://fonts.googleapis.com"],
- },
+ csp: [
+   { reason: "Loads map tiles and live vehicle positions",
+     connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
+     imgSrc: ["https://a.tile.openstreetmap.org"] },
+   { reason: "Loads web fonts for map labels",
+     styleSrc: ["https://fonts.googleapis.com"],
+     fontSrc: ["https://fonts.gstatic.com"] },
+ ],
```

Replace a host that is only known at runtime with an input, rebuild and run `flow-like-widgets validate`. A `contract.json` packed with the object form fails validation at hub publish and desktop install; rebuild it with the current bundler. The contract stays at version 2.

### Migrate from `--connect`

`flow-like-widgets pack --connect <host>` baked hosts into the packed meta of every widget in the package. Those hosts were not part of any contract and were never shown to viewers. The flag has been removed:

```text
error: The --connect flag was removed: declare network sources per widget in widget.config.ts "csp"
```

Calling `pack()` with the `connectHosts` option fails the same way.

To migrate:

1. Remove `--connect` from `mise.toml` and any build scripts.
2. Add a `csp` group to each widget that needs the network, with a reason and only the hosts that widget uses. A `--connect` host only ever went into `connect-src`, so it becomes a `connectSrc` entry. It must now follow the [source rules](#sources): `http://` hosts, ports and paths are rejected. Images, fonts, media and stylesheets from other sites were never allowed before; declare them under their own directives if the widget needs them.
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
  csp: [{ reason: "Loads map tiles from MapTiler", connectSrc: ["https://api.maptiler.com"] }],
});
```

Bundles packed with `--connect` still load, but current servers replace their meta, so those hosts stop working until the widget declares them in `csp`.

## Related Guides

- [Pages](/dev/a2ui/pages/) — compose Widgets into app experiences
- [Visual Builder](/dev/a2ui/visual-builder/) — edit the component graph
- [A2UI overview](/dev/a2ui/overview/) — understand surfaces and actions
