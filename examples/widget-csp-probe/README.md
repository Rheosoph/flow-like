# Widget CSP probe

A test package, not a sample to copy into apps. It checks inside a real webview that the widget network policy holds:

- approved network sources work, and every other source is blocked;
- a wildcard covers subdomains and never its apex;
- an address approved while the app runs works, and a refused one stays blocked;
- `data:` and `blob:` bytes stay local: media manifests, captions, artwork and documents built from them never reach the network;
- a widget cannot read another document's `blob:` URLs;
- a widget document cannot navigate itself to another document;
- widget frames cannot reach each other;
- the desktop IPC entry points are out of reach.

CSP enforcement differs between engines, so this has to run on every target before a package with `csp` is approved. A failure becomes a row in `packages/wasm/schema/data/widget_engine_gates.json` (see [Engine gates](#engine-gates)). The contract is described in the [Network access and CSP](../../apps/docs/src/content/docs/dev/a2ui/widgets.md#network-access-and-csp) docs.

## What it declares

`csp-probe` asks for `workers` and declares three purpose groups:

| Purpose | Source or input | `connectSrc` | `imgSrc` | Used for |
| --- | --- | --- | --- | --- |
| Test services | `https://httpbin.org` | yes | yes | `/get`, `/image/png`, `/html` in a nested frame |
| Test services | `https://httpbingo.org` | yes | no | `/get`, and `/image/png` must stay blocked |
| Wildcard matching | `https://*.wikipedia.org` | yes | yes | `www.wikipedia.org` must load, `wikipedia.org` (the apex) must not |
| Runtime addresses | input `runtimeApprovedUrl` | yes | yes | a host the viewer approves while the widget runs |
| Runtime addresses | input `runtimeRefusedUrl` | yes | yes | a host the viewer refuses while the widget runs |
| none | `https://www.w3.org` | no | no | `/`, `/Icons/w3c_home.png` |

`https://httpbin.org` is listed in the private section of the Public Suffix List, so Flow-Like classifies it **Anyone can receive** (`broad`). Its purpose card and the whole dialog therefore use the broad layout: title "Widget requests broad network access", destructive banner, **Don't allow** focused, and an inline confirmation before **Always allow for this project**. Every other source, including both runtime examples below, must be labelled **External site**, and the wildcard and runtime cards must stay neutral. Any other level is a finding.

Inputs:

| Input | Default | Meaning |
| --- | --- | --- |
| `expectation` | `auto` | What the host should have granted. `auto` reads the document URL: `index.0.html` means baseline, a grant means granted, or preview in a store preview. |
| `autoRun` | `false` | Run **Run checks** as soon as the host initializes the widget. |
| `runtimeApprovedUrl` | empty | `https` URL of an image on a host no static source covers. Approve it when the widget asks. |
| `runtimeRefusedUrl` | empty | `https` URL of an image on another uncovered host. Choose **Don't allow** when the widget asks. |
| `canaryUrl` | empty | `https` URL of a server whose request log you can read. See [Canary server](#canary-server). |
| `foreignBlobUrls` | `[]` | `blob:` URLs from the sibling widget and the host page. See [Foreign blob URLs](#foreign-blob-urls). |

`csp-probe-sibling` declares no capabilities and no network sources. It is the target of the cross-widget navigation checks, and it holds three `blob:` URLs (text, SVG image, WAV audio) for the foreign-blob checks: it shows them in its frame, emits them once per mount as the `blobUrls` event, and returns them from the `getBlobUrls` query. Mount it next to the probe so the frame isolation checks see more than one widget.

To use other hosts, change the `csp` in `src/widgets/csp-probe/widget.config.ts` **and** `src/lib/hosts.ts`. The `setup.declaration` check fails when the two drift apart. Each host must:

- be reachable over `https` from the device under test;
- serve a real image at the image URL without a redirect;
- not be a reserved name, and not be the hub or app domain of the deployment you test against, because either makes the whole policy invalid.

## Layout

```
widget-csp-probe/
├── flow-like.toml                  # widgets-only package manifest
├── mise.toml                       # build / validate
└── widgets/vanilla/                # one Vite app, vanilla TypeScript
    ├── public/shared/csp-probe-marker.svg   # packed as shared/csp-probe-marker.svg
    └── src/
        ├── lib/                    # checks, shared by both widgets
        │   ├── suite.ts            # Run checks
        │   ├── local.ts, media.ts  # Run local-scheme checks
        │   ├── navigation.ts       # Attempt navigation
        │   └── canary.ts, hosts.ts, policy.ts, …
        └── widgets/
            ├── csp-probe/          # the probe
            └── csp-probe-sibling/  # navigation target and blob: source, no permissions
```

## Build

```bash
mise run build      # widgets.flwb at the project root
mise run validate   # contracts + packed bundle
```

The mock-host dev harness (`bunx flow-like-widgets dev`) and the plain Vite dev server enforce no CSP, so almost every check fails there. That is expected. Only results from a real host count.

## Before a run

### Canary server

A JS result cannot prove that a native media pipeline, the OS media controls or a navigated document sent nothing. Those rows point absolute URLs at a canary server, and the verdict is its request log.

1. Use any `https` server whose access log you can read: a scratch Caddy or nginx on a test domain, or a private request-capture URL such as a webhook.site token URL. Plain `http` is refused, because mixed-content blocking would hide a CSP failure.
2. The canary host must not be covered by any source of the served policy. `local.canaryUndeclared` checks this and fails otherwise.
3. Set `canaryUrl` on the probe.

Every local-scheme run and every `blob:`/`data:` navigation uses a fresh prefix `{canaryUrl}/flw-csp-probe-{run id}/{row id}/…`. The prefix is reported as `canary` in the `result` event and shown in each row's `expected`. **Any request under the prefix is a failure of the row named in the path.**

### Runtime hosts

1. Set `runtimeApprovedUrl`, for example `https://upload.wikimedia.org/wikipedia/commons/7/70/Example.png`, and mount the probe. The dialog lists it under the runtime purpose. Keep **Also allow the 1 address provided while the app runs** checked and allow.
2. Set `runtimeRefusedUrl`, for example `https://www.gstatic.com/images/branding/product/1x/googleg_48dp.png`. The widget shows **Wants to load from 1 new site**. Choose **Review**, then **Don't allow**.
3. Run the checks. The rows skip when an input is empty or covered by a static source.

Update inputs from the page builder's Inspector or from a flow with **Update Widget Inputs**. On web, the document URL then carries the approved runtime hosts as `index.{grant}~{runtime}.html`; the runtime rows print the decoded component. Desktop keeps runtime hosts in its grant registry.

### Foreign blob URLs

1. Mount `csp-probe-sibling` on the same page. Copy the array it shows into the probe's `foreignBlobUrls`, or forward its `blobUrls` event into the probe with **Update Widget Inputs**.
2. Add `blob:` URLs of the host page. Open the developer console of the page that hosts the widgets (the Flow-Like window on desktop, the browser tab on web) and create one per kind:

   ```js
   URL.createObjectURL(new Blob(["host secret"], { type: "text/plain" }));
   URL.createObjectURL(new Blob(['<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>'], { type: "image/svg+xml" }));
   URL.createObjectURL(new Blob([new Uint8Array([82,73,70,70,100,31,0,0,87,65,86,69,102,109,116,32,16,0,0,0,1,0,1,0,64,31,0,0,64,31,0,0,1,0,8,0,100,97,116,97,64,31,0,0]), new Uint8Array(8000).fill(128)], { type: "audio/wav" }));
   ```

   The last one is one second of silent WAV audio.
3. Keep the sibling mounted and the console's page open while the checks run. A revoked URL also fails to load, for a harmless reason.

## Running the probe

**Desktop.** Add this folder as a project in **Developer**, run `mise run build`, and open **Test Widgets**. The consent source shows as a local package. To test the registry source, publish the package to a test hub and install it.

**Web.** Publish the package to a test hub, add both widgets to one page of a test app, and open the page in each browser.

For each engine:

1. **Granted.** Mount the probe. The dialog must show the broad layout described in [What it declares](#what-it-declares), background workers under **Also asks for**, and three purpose cards: the two test services, the wildcard, and the runtime address once `runtimeApprovedUrl` is set. Only `https://httpbin.org` and `https://*.wikipedia.org` may load images. Choose **Allow this time**, then **Run checks**. The report expects `granted`. Any `fail` is a finding.
2. **Local schemes.** Select **Run local-scheme checks**. It plays a quiet tone for about 6 seconds (the Media Session row needs audible playback) and takes 10 to 15 seconds. Then read the canary log for the reported prefix.
3. **Navigation.** Every navigation case can replace the probe document, so remount the widget (reload the page) before each attempt. Pick a case and select **Attempt navigation**:
   - **Pass:** the probe is still running 5 seconds later and reports `pass`, or the frame shows the engine's own error page. Chromium shows its error page for a blocked frame navigation; Firefox and WebKit keep the document running.
   - **Fail:** a red `NAVIGATION REACHED` banner appears, the red SVG marker appears, or a `fail` report arrives. The SVG cannot run script, so that case is visual only.
   - `nav.blobDocument` and `nav.dataDocument` load pages without the SDK, which cannot report. When one of them turns the frame red, the frame pin did not hold: record it. The row still passes the ship gate only if the red page says the fetch to `www.w3.org` was **blocked** and the canary log has no entry for it.
   - `nav.baselineDocument` and `nav.runtimeComponent` only run from a granted document.
   - `nav.replaceStateReload` and `nav.historyBack` pass straight away when `history.replaceState` refuses the URL, as WebKit does. Chromium and Firefox accept it, so the probe goes on to reload, or to navigate and call `history.back()`.
4. **Revoked.** Revoke the widget's permissions from the Inspector provenance header or the app's **Widget permissions** sheet, or clear them all with **Clear widget permissions on this device** under **Settings → Registry**. Revoking unmounts the widget. When the dialog appears again, choose **Don't allow**, then **Run without these permissions** on the blocked card, and run the checks again. The document is now `index.0.html`, the report expects `baseline`, and every network row must be blocked.
5. **Preview.** Open the probe from the store preview and run the checks. The report expects `preview`: workers start, all network access stays blocked, runtime inputs are ignored. The SDK emits no events in previews, so read the results from the table.

## Reading results

Every run emits a `result` event, and `getReport` returns the latest report. Both carry `widgetId`, `phase` (`suite`, `local` or `navigation`), `expectation`, `document` (grants redacted as `{grant}`, runtime components as `{runtime}`), `userAgent`, `passed`, `summary`, `canary` (when a canary prefix was used) and the `checks` map:

| Status | Meaning |
| --- | --- |
| `pass` | Behaved as expected. |
| `fail` | Violated the policy. |
| `review` | The probe could not decide on its own. Read `observed`. Canary rows are always `review`: the canary log decides. |
| `skip` | Not applicable in this context, or an input is missing. `observed` says which. |

A navigation attempt first reports `review` for its case, then `pass` or `fail` if the document survives or the target loads.

WebKit blocks requests from workers without dispatching a `securitypolicyviolation` event, so `worker.fetchUndeclared` reports `review` there. Confirm the "Refused to connect" message in the Web Inspector console.

### Run checks

| Check | What it does | granted | preview | baseline |
| --- | --- | --- | --- | --- |
| `setup.declaration` | Compares the declared purposes and inputs with `src/lib/hosts.ts` | pass | pass | pass |
| `document.opaqueOrigin` | `self.origin === "null"` | pass | pass | pass |
| `document.noReferrer` | `document.referrer` is empty | pass | pass | pass |
| `document.cspMeta` | One CSP meta, no `'self'`, `connect-src` and `media-src` start with `data: blob:`, `connect-src` lists the granted host only when granted | pass | pass | pass |
| `fetch.granted` | `no-cors` fetch to the granted host | allowed | blocked | blocked |
| `fetch.connectOnly` | Fetch to the connect-only host | allowed | blocked | blocked |
| `fetch.undeclared` | Fetch to the undeclared host | blocked | blocked | blocked |
| `img.granted` | Image from the granted host | allowed | blocked | blocked |
| `img.connectOnly` | Image from the connect-only host (wrong directive) | blocked | blocked | blocked |
| `img.undeclared` | Image from the undeclared host | blocked | blocked | blocked |
| `wildcard.subdomainFetch` / `Img` | `www.wikipedia.org` under `https://*.wikipedia.org` | allowed | blocked | blocked |
| `wildcard.apexFetch` / `Img` | `wikipedia.org` itself: CSP3 never matches a wildcard against its apex (WebKit before 246729@main did) | blocked | blocked | blocked |
| `runtime.approvedFetch` / `Img` | `runtimeApprovedUrl`, whose host the viewer approved | allowed | blocked | blocked |
| `runtime.refusedFetch` / `Img` | `runtimeRefusedUrl`, whose host the viewer refused | blocked | blocked | blocked |
| `worker.start` | Starts a `blob:` worker | started | started | blocked |
| `worker.fetchGranted` | Worker fetches the granted host | allowed | blocked | skip |
| `worker.fetchUndeclared` | Worker fetches the undeclared host | blocked | blocked | skip |
| `frame.nestedHttps` | Nested `<iframe>` to the granted host | blocked | blocked | blocked |
| `frames.parentHasNoChildren` | `parent.length === 0` (closed shadow root) | pass | pass | pass |
| `frames.selfUnreachable` | Walking `top.frames` never finds this document | pass | pass | pass |
| `frames.wrappersHaveNoChildren` | No frame below `top` exposes child frames. Reports `review` when a page preview or embedded site adds frames of its own | pass | pass | pass |
| `ipc.tauriInternals` | `__TAURI_INTERNALS__` absent, or its `invoke` unusable | pass | pass | pass |
| `ipc.webkitMessageHandler` | `webkit.messageHandlers.ipc` absent | pass | pass | pass |
| `ipc.webview2` | `chrome.webview.postMessage` absent | pass | pass | pass |
| `ipc.customProtocol` | `POST` to `ipc://localhost` and `http://ipc.localhost` fails | pass | pass | pass |

The wildcard and runtime rows append whether the served policy covers the host, so a `fail` tells a missing grant (not approved, server without runtime support, engine gate) apart from an engine that ignored the policy.

### Run local-scheme checks

`data:` and `blob:` are allowed in `connect-src` and `media-src` of every widget, so every expectation gets the same results.

| Check | What it does | Expected |
| --- | --- | --- |
| `local.canaryUndeclared` | The canary host is in no directive of the served policy | pass |
| `local.fetchData` | `fetch` of a `data:` JSON URL | allowed |
| `local.fetchBlob` | `fetch` of this document's `blob:` URL | allowed |
| `local.imgBlob` | Image from this document's `blob:` SVG | allowed |
| `local.mediaData` | `<audio>` with a `data:` WAV | allowed |
| `local.mediaBlob` | `<audio>` with a `blob:` WAV | allowed |
| `local.hlsMasterData` / `Blob` | HLS master playlist with absolute `EXT-X-MEDIA` rendition and variant URIs | no canary request |
| `local.hlsMediaData` / `Blob` | HLS media playlist with absolute `EXT-X-KEY`, `EXT-X-MAP` and segment URIs | no canary request |
| `local.dashData` / `Blob` | DASH MPD with an absolute `BaseURL` | no canary request |
| `local.smoothData` / `Blob` | Smooth Streaming manifest with an absolute fragment URL | no canary request |
| `local.vttData` | `data:` WebVTT track whose `STYLE` block names an absolute image | no canary request |
| `local.mediaSessionArtwork` | Media Session artwork on the canary while `data:` audio plays | no canary request |
| `local.foreignBlob.fetch` / `img` / `video` | Each `foreignBlobUrls` entry through `fetch`, `<img>` and `<video>` | blocked |

Native HLS exists in WebKit (and Chromium builds that ship it); native DASH and Smooth Streaming only through GStreamer on WebKitGTK. `observed` lists `canPlayType` and the media events, so you can tell whether the engine parsed the manifest at all. The canary log decides either way. A foreign URL only counts for the element that matches its type: text through `fetch`, SVG through `<img>`, WAV through `<video>`.

### Navigation cases

| Navigation case | Target |
| --- | --- |
| `nav.baselineDocument` | `index.0.html` of the probe, from a granted document |
| `nav.otherGrant` | `index.{grant}.html` with a grant no host issued |
| `nav.runtimeComponent` | `index.{grant}~{runtime}.html`: this document's grant with a runtime component no host minted (the undeclared host on a declared input), from a granted document |
| `nav.sharedSvg` | `../../shared/csp-probe-marker.svg` |
| `nav.siblingWidget` | `../csp-probe-sibling/index.0.html` |
| `nav.blobDocument` | A `blob:` HTML page built by the probe. It fetches `www.w3.org` and pings the canary |
| `nav.dataDocument` | The same page as a `data:` URL |
| `nav.replaceStateReload` | `history.replaceState` to the sibling URL, then `location.reload()` |
| `nav.historyBack` | `history.replaceState` to the sibling URL, navigate to the probe's own URL, then `history.back()` |

The probe cannot observe channels that CSP does not govern, such as WebRTC, dns-prefetch or preconnect. Those remain documented residual risks.

## Engine gates

`packages/wasm/schema/data/widget_engine_gates.json` ships with no rows. Every feature is a denylist: an engine that matches no row gets all of them. Add a row only for an engine version range where a probe row **failed**:

| Failed rows | Deny |
| --- | --- |
| `wildcard.*` | `wildcardSources` |
| `runtime.*` | `runtimeSources` |
| A canary request from `local.hls*`, `local.dash*`, `local.smooth*`, `local.vttData` or `local.mediaSessionArtwork` | `localMedia` (drops `data:` and `blob:` from `media-src` only) |
| `local.fetchData`, `local.fetchBlob`, `local.imgBlob`, `local.mediaData`, `local.mediaBlob` | nothing: local bytes being refused is a functional bug, not a leak |
| Anything else: declared hosts, foreign `blob:` reads, navigation, isolation, IPC | `declaredSources` and `runtimeSources`, which keeps network sources off on that target |

Hypothetical rows, one for the macOS desktop webview and two for Safari (the `ios` copy covers iPhone and the iPad in mobile mode, the `macos` copy covers Mac Safari and the iPad in desktop mode):

```json
{
  "rows": [
    { "engine": "webkit", "platform": "macos", "minVersion": [20621, 0], "maxVersion": [20621, 999], "deny": ["localMedia"] },
    { "engine": "webkit", "platform": "macos", "minVersion": [17, 0], "maxVersion": [17, 6], "deny": ["wildcardSources"] },
    { "engine": "webkit", "platform": "ios", "minVersion": [17, 0], "maxVersion": [17, 6], "deny": ["wildcardSources"] }
  ]
}
```

`engine` is `chromium`, `webkit` or `gecko`. Versions are `[major, minor]` and both bounds are inclusive.

**Desktop and web versions are on different scales.** The desktop app gates on the webview's own version from `tauri::webview_version()`. The web API gates on the browser's `User-Agent`:

| Target | `engine` / `platform` | Version the gate compares | Example |
| --- | --- | --- | --- |
| Windows desktop | `chromium` / `windows` | WebView2 runtime version (Chromium major) | `130.0.2849.56` → `[130, 0]` |
| Android app | `chromium` / `android` | Android System WebView version | `131.0.6778.39` → `[131, 0]` |
| macOS desktop | `webkit` / `macos` | WebKit framework `CFBundleVersion` | `20621.1.15` → `[20621, 1]` (macOS 15), `21624.2.5` → `[21624, 2]` (macOS 26) |
| iOS app | `webkit` / `ios` | WebKit framework `CFBundleVersion` | `20621.1.15` → `[20621, 1]` |
| Linux desktop | `webkit` / `linux` | WebKitGTK version | `2.46.1` → `[2, 46]` |
| Web: Chrome, Edge | `chromium` / from the UA | `Chrome/` | `Chrome/130.0.0.0` → `[130, 0]` |
| Web: Safari on macOS | `webkit` / `macos` | `Version/` | `Version/17.4` → `[17, 4]` |
| Web: any browser on iOS | `webkit` / `ios` | iOS version from `OS 17_4` | `[17, 4]` |
| Web: Firefox | `gecko` / from the UA | `Firefox/` | `Firefox/131.0` → `[131, 0]` |

Rows therefore have to be platform-aware and bounded on both sides:

- A row whose engine and platform match also matches when the version is unknown (a `User-Agent` without a parsable version, or a webview that reports none). The deny then applies: a known engine that hides its version is treated as affected.
- iPadOS Safari, and other iPad browsers in desktop mode, send a macOS `User-Agent` (`Macintosh`, `Version/17.4`), so the web gate sees `webkit` / `macos` on the Safari version scale. Write every web WebKit row twice, once with `platform: "ios"` and once with `platform: "macos"`, with the same bounds.
- A `webkit` row without `platform` matches every desktop WebKit (macOS, iOS, Linux) as well as Safari.
- A `webkit` / `macos` row with only `minVersion: [17, 0]` also matches the desktop app, whose `20621` is above it. A `webkit` / `linux` row with only `maxVersion: [18, 0]` also matches WebKitGTK `2.46`. Set both `minVersion` and `maxVersion`.
- Chromium versions are on the same scale on desktop and web. A `chromium` / `windows` row gates WebView2 and Chrome on Windows of the same versions together.

Record the version the gate compares, not the desktop webview's `navigator.userAgent`. The desktop logs it at debug level as `Widget engine gate evaluated` with the engine, platform and version. On macOS, `defaults read /System/Library/Frameworks/WebKit.framework/Resources/Info.plist CFBundleVersion` prints the WebKit version too.

## Platform matrix

Copy this table into the PR and fill in each cell with `pass`, `fail (<check ids>)` or `review (<note>)`. **Any engine that fails keeps the failing feature disabled on that target through an engine gate row.**

| Engine | Gate version (engine / platform / version) | Suite: granted | Suite: revoked | Suite: preview | Wildcard apex | Runtime sources | Local schemes (canary log) | Foreign blobs | Navigation cases | Frame isolation | IPC | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| macOS WKWebView (desktop) | | | | | | | | | | | | |
| iOS WKWebView (mobile) | | | | | | | | | | | | |
| Windows WebView2 with `ICoreWebView2_22` | | | | | | | | | | | | |
| Windows WebView2 without `ICoreWebView2_22` | | | | | | | | | | | | |
| Linux WebKitGTK | | | | | | | | | | | | |
| Android WebView | | | | | | | | | | | | |
| Web: Chromium | | | | | | | | | | | n/a | |
| Web: Firefox | | | | | | | | | | | n/a | |
| Web: Safari | | | | | | | | | | | n/a | |
