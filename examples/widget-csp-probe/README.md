# Widget CSP probe

A test package, not a sample to copy into apps. It checks inside a real webview that the widget network policy holds:

- approved network sources work;
- every other source is blocked;
- a widget document cannot navigate itself to another document;
- widget frames cannot reach each other;
- the desktop IPC entry points are out of reach.

CSP enforcement differs between engines, so this has to run on every target before a package with `csp` is approved. The contract is described in the [Network access and CSP](../../apps/docs/src/content/docs/dev/a2ui/widgets.md#network-access-and-csp) docs.

## What it declares

`csp-probe` asks for `workers` and these network sources:

| Role | Origin | `connectSrc` | `imgSrc` | Used for |
| --- | --- | --- | --- | --- |
| Granted | `https://httpbin.org` | yes | yes | `/get`, `/image/png`, `/html` in a nested frame |
| Connect only | `https://httpbingo.org` | yes | no | `/get`, and `/image/png` must stay blocked |
| Undeclared | `https://www.w3.org` | no | no | `/`, `/Icons/w3c_home.png` |

`csp-probe-sibling` declares nothing. It is the target of the cross-widget navigation checks. Mount it next to the probe so the frame isolation checks see more than one widget.

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
        └── widgets/
            ├── csp-probe/          # the probe
            └── csp-probe-sibling/  # navigation target, no permissions
```

## Build

```bash
mise run build      # widgets.flwb at the project root
mise run validate   # contracts + packed bundle
```

The mock-host dev harness (`bunx flow-like-widgets dev`) and the plain Vite dev server enforce no CSP, so almost every check fails there. That is expected. Only results from a real host count.

## Running the probe

**Desktop.** Add this folder as a project in **Developer**, run `mise run build`, and open **Test Widgets**. The consent source shows as a local package. To test the registry source, publish the package to a test hub and install it.

**Web.** Publish the package to a test hub, add both widgets to one page of a test app, and open the page in each browser.

For each engine:

1. **Granted.** Mount the probe. The consent dialog must list background workers, both hosts under sending and receiving data, and only `https://httpbin.org` under loading images. Choose **Allow this time**, then **Run checks**. The report expects `granted`. Any `fail` is a finding.
2. **Navigation.** Every navigation case can replace the probe document, so remount the widget (reload the page) before each attempt. Pick a case and select **Attempt navigation**:
   - **Pass:** the probe is still running 5 seconds later and reports `pass`, or the frame shows the engine's own error page. Chromium shows its error page for a blocked frame navigation; Firefox and WebKit keep the document running.
   - **Fail:** a red `NAVIGATION REACHED` banner appears, the red SVG marker appears, or a `fail` report arrives. The SVG cannot run script, so that case is visual only.
   - `nav.baselineDocument` only runs from a granted document.
   - `nav.replaceStateReload` and `nav.historyBack` pass straight away when `history.replaceState` refuses the URL, as WebKit does. Chromium and Firefox accept it, so the probe goes on to reload, or to navigate and call `history.back()`.
3. **Revoked.** Revoke the widget's permissions from the Inspector provenance header or the app's **Widget permissions** sheet, or clear them all with **Clear widget permissions on this device** under **Settings → Registry**. Revoking unmounts the widget. When the dialog appears again, block the widget, choose **Run without these permissions** on the blocked card, and run the checks again. The document is now `index.0.html`, the report expects `baseline`, and `fetch.granted` and `img.granted` must be blocked.
4. **Preview.** Open the probe from the store preview and run the checks. The report expects `preview`: workers start, all network access stays blocked. The SDK emits no events in previews, so read the results from the table.

Set the `expectation` input to override the automatic choice, which comes from the document URL: `index.0.html` means baseline, a grant means granted or preview. Set `autoRun` to run the checks as soon as the host initializes the widget.

## Reading results

Every run emits a `result` event, and `getReport` returns the latest report. Both carry `widgetId`, `phase` (`suite` or `navigation`), `expectation`, `document` (grants redacted), `userAgent`, `passed`, `summary` and the `checks` map:

| Status | Meaning |
| --- | --- |
| `pass` | Behaved as expected. |
| `fail` | Violated the policy. |
| `review` | The probe could not decide on its own. Read `observed`. For example, a request failed without any CSP violation event, which can also mean the host was unreachable. |
| `skip` | Not applicable in this context. |

A navigation attempt first reports `review` for its case, then `pass` or `fail` if the document survives or the target loads.

WebKit blocks requests from workers without dispatching a `securitypolicyviolation` event, so `worker.fetchUndeclared` reports `review` there. Confirm the "Refused to connect" message in the Web Inspector console.

| Check | What it does | granted | preview | baseline |
| --- | --- | --- | --- | --- |
| `setup.declaration` | Compares the declared `csp` with `src/lib/hosts.ts` | pass | pass | pass |
| `document.opaqueOrigin` | `self.origin === "null"` | pass | pass | pass |
| `document.noReferrer` | `document.referrer` is empty | pass | pass | pass |
| `document.cspMeta` | One CSP meta, no `'self'`, `connect-src` lists the granted host only when granted | pass | pass | pass |
| `fetch.granted` | `no-cors` fetch to the granted host | allowed | blocked | blocked |
| `fetch.connectOnly` | Fetch to the connect-only host | allowed | blocked | blocked |
| `fetch.undeclared` | Fetch to the undeclared host | blocked | blocked | blocked |
| `img.granted` | Image from the granted host | allowed | blocked | blocked |
| `img.connectOnly` | Image from the connect-only host (wrong directive) | blocked | blocked | blocked |
| `img.undeclared` | Image from the undeclared host | blocked | blocked | blocked |
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

| Navigation case | Target |
| --- | --- |
| `nav.baselineDocument` | `index.0.html` of the probe, from a granted document |
| `nav.otherGrant` | `index.{grant}.html` with a grant no host issued |
| `nav.sharedSvg` | `../../shared/csp-probe-marker.svg` |
| `nav.siblingWidget` | `../csp-probe-sibling/index.0.html` |
| `nav.replaceStateReload` | `history.replaceState` to the sibling URL, then `location.reload()` |
| `nav.historyBack` | `history.replaceState` to the sibling URL, navigate to the probe's own URL, then `history.back()` |

The probe cannot observe channels that CSP does not govern, such as WebRTC, dns-prefetch or preconnect. Those remain documented residual risks.

## Platform matrix

Copy this table into the PR and fill in each cell with `pass`, `fail (<check ids>)` or `review (<note>)`. **Any engine that fails keeps network sources disabled on that target.**

| Engine | Build / user agent | Suite: granted | Suite: revoked | Suite: preview | Navigation cases | Frame isolation | IPC | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| macOS WKWebView (desktop) | | | | | | | | |
| iOS WKWebView (mobile) | | | | | | | | |
| Windows WebView2 with `ICoreWebView2_22` | | | | | | | | |
| Windows WebView2 without `ICoreWebView2_22` | | | | | | | | |
| Linux WebKitGTK | | | | | | | | |
| Android WebView | | | | | | | | |
| Web: Chromium | | | | | | | n/a | |
| Web: Firefox | | | | | | | n/a | |
| Web: Safari | | | | | | | n/a | |
