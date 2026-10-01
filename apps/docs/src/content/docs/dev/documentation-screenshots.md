---
title: Capture documentation screenshots
description: Capture application states and rendered FlowScript workflows with repository tools
---

Capture Desktop or Web screens with `docs:screenshot`, or render a FlowScript
workflow in Studio with `workflow:screenshot`. Both commands write images for
the documentation site or FlowBook.

Install the [repository toolchain and dependencies](/dev/build/) first.
The screenshot runner uses Puppeteer and its Chromium browser; workflow
captures also compile a Rust helper. Run commands from the repository root.
See [documentation assets](/dev/documentation-assets/) for publication conventions.

## Capture the onboarding example

The checked-in plan opens onboarding with `capture=docs`, captures the initial
profile grid, selects the fourth profile, and captures both the selected and
completed states:

```sh
bun run docs:screenshot -- \
  --plan apps/desktop/lib/doc-screenshot/examples/onboarding.plan.json \
  --output-dir tmp/doc-screenshots/onboarding \
  --json
```

The three lossless WebP files are written below
`tmp/doc-screenshots/onboarding`.

Plan paths use these bases:

- `tauriFixture` and `httpFixture` are resolved relative to the plan file. This
  keeps a plan and its fixtures portable when the command is launched from
  another directory.
- `outputDir` is resolved relative to the process working directory. From the
  repository root, the example therefore writes to
  `tmp/doc-screenshots/onboarding`.

## Refresh the checked-in documentation screenshots

Capture to a temporary directory, inspect the output, then publish reviewed
images. The checked-in documentation plans default to `apps/docs/src/assets`,
so always pass `--output-dir` during review. Most plans start in dark mode with
a 1624 by 1060 CSS-pixel viewport, DPR 2, and lossless WebP. Scenarios can
override those defaults or crop an element to show a specific control.

For example, capture the Start guide images with their checked-in fixtures:

```sh
bun run docs:screenshot -- \
  --plan apps/desktop/lib/doc-screenshot/examples/docs-start.plan.json \
  --output-dir tmp/doc-screenshots/start \
  --json
```

Plans for `apps`, `ontology`, `ontology-sharing`, `sharing`, `roles`, `studio`,
`reference`, `developer-mode`, `models`, `quickstart`, `setup`, `chat`, `runs`,
and `versions` live in the same directory. Substitute their names and use a
different output directory for each batch. Each completed run writes
`capture-result.json` beside its images. Each completed scenario also writes
`<scenario-name>.result.json`, so its evidence survives a later batch failure.

To rerun one named scenario while fixing a recipe:

```sh
bun run docs:screenshot -- \
  --plan apps/desktop/lib/doc-screenshot/examples/docs-start.plan.json \
  --scenario home-and-profile-menu \
  --output-dir tmp/doc-screenshots/start-review
```

Each plan starts from application routes and performs the navigation and UI
interactions needed to expose the documented state. A failed wait or action
fails that scenario. Unexpected browser errors also fail it. A passing result
still needs visual review: inspect every image for the intended heading,
populated fixture content, active panel, legible controls, and loading or error
overlays. Compare each image with the procedure on its documentation page.

### Publish reviewed images

After inspecting the batch, publish it with:

```sh
bun apps/desktop/scripts/publish-doc-screenshots.ts \
  --reviewed tmp/doc-screenshots/start/capture-result.json

bun apps/desktop/scripts/publish-doc-screenshots.ts --check
```

The publisher requires passing scenarios with provenance and checks each
image's captured hash. It copies images to `apps/docs/src/assets` and records
them in `docs-screenshots.manifest.json`. The manifest includes the plan,
scenario, fixture hashes, source commit, dirty-working-tree flag, route,
capture and review dates, browser, render settings, dimensions, and image hash.
`--check` verifies the image, fixture hashes, and the fingerprint of the
scenario with its inherited defaults and app settings. The full plan hash is
retained as history; changing an unrelated scenario does not invalidate every
image from that plan.

You can publish a reviewed passing scenario's `.result.json` checkpoint with
the same command. To select one passing scenario from a failed batch result,
use `--reviewed <capture-result.json> --scenario <name>`. A failed scenario
itself is never eligible for publication.

The `--reviewed` flag records that the caller inspected the images; the script
cannot judge whether a screenshot teaches the intended task. A commit plus a
dirty flag also cannot reconstruct uncommitted source edits. Keep the reviewed
source change with the published capture.

When navigation, the editor shell, a plan, or its fixture changes, review the
affected screenshots again. Reuse the existing asset names when their purpose
is unchanged, and use focused captures when one overview no longer serves the
procedure.

### Scheduled review captures

The [Documentation screenshots workflow](https://github.com/Rheosoph/flow-like/blob/dev/.github/workflows/docs-screenshots.yml)
runs the documentation plans when relevant UI or capture files change, on a
weekly schedule, and when started manually in GitHub Actions. It runs the screenshot-runner tests and uploads the
capture outputs and results for review, including available diagnostics from
failed jobs. It does not publish images automatically.

Use those artifacts to identify stale or failed captures. Reproduce the
affected plan locally for publication; result files refer to the image paths
on the machine that captured them.

### Maintain fixtures

Ordinary captures use the checked-in fixtures. If the fixture data itself
needs rebuilding, the repository provides these generators:

```sh
bun apps/desktop/scripts/generate-doc-screenshot-fixtures.ts
bun apps/desktop/scripts/generate-doc-studio-screenshot-fixture.ts
```

The first generator writes the Apps, Sharing, Roles, Ontology, and Setup
fixtures. The second writes Studio, Quickstart, Runs, and Versions fixtures.
The Models fixture is maintained separately. The Ontology sharing plan uses
the Ontology fixture with its own strict browser HTTP responses.

Review their changes before capturing. Some fixtures also contain maintained
HTTP responses and scenario-specific state; regenerating data is a source
change, not a prerequisite for every screenshot run.

## Direct capture

Use direct mode for one route and one capture:

```sh
bun run docs:screenshot -- \
  --app web \
  --path /onboarding \
  --query capture=docs \
  --output tmp/doc-screenshots/onboarding/direct-web.webp \
  --viewport 1624x1060 \
  --dpr 2 \
  --theme light \
  --wait-for h1 \
  --json
```

`--query` accepts a `key=value` pair and can be repeated. Use `--full-page` for
a full document capture, or `--selector <css-selector>` to capture one
element. The output extension selects PNG, WebP, or JPEG.

The CLI starts the selected Next app automatically. Use
`--frontend-url http://127.0.0.1:PORT` to reuse a running loopback server,
`--port <number>` to change the automatically started server's port, or
`--keep-server` to leave a server started by the CLI running.

If the development server stalls while serving compiled assets, set
`DOC_SCREENSHOT_STATIC_DIR=apps/desktop/.next/dev/static` for Desktop captures
(or `apps/web/.next/dev/static` for Web). The runner can serve those assets
from disk. Use the directory from the same running server and current source;
stale compiled assets can produce a misleading screenshot. Results record the
chosen directory as `staticAssetsFromDisk`.

## Plan format

Plans use the `flow-like.doc-screenshot-plan/v1` schema. A plan declares its
app, output directory, optional desktop Tauri fixture, optional browser HTTP
fixture, render defaults, and one or more scenarios. Each scenario starts at
`path` plus an optional `query` object and runs its `steps` in order.

The supported steps are:

| Step | Fields | Behavior |
| --- | --- | --- |
| `waitFor` | one of `selector`, `urlIncludes`, or `text`; optional `state`, `timeoutMs` | Waits for a DOM, URL, or text condition. Selector states are `attached`, `visible`, `hidden`, and `detached`. |
| `click` | `selector`, optional `index`, `button`, `clickCount`, `modifiers` | Clicks the matching element. `index` is zero-based. `modifiers` accepts a unique subset of `Alt`, `Control`, `Meta`, and `Shift`. |
| `drag` | `selector`, `targetSelector`, optional `index`, `targetIndex`, `steps`, `button`, `release` | Drags between the centers of two matching elements. Both centers must be visible and unobscured. `steps` is 1–100 (default 20). `release` defaults to `true`; set it to `false` only when the next step captures the held-pointer state. The button is released after that capture. |
| `fill` | `selector`, `value` or `valueEnv`, optional `index` | Replaces the value of an input. |
| `type` | `selector`, `value` or `valueEnv`, optional `index`, `delayMs` | Types into an input. |
| `press` | `key`, optional `selector`, `index` | Sends a keyboard key globally or to an element. |
| `select` | `selector`, `values`, optional `index` | Selects one or more values in a native select element. |
| `check` | `selector`, optional `index`, `checked` | Sets a checkbox or radio control's checked state. |
| `hover` | `selector`, optional `index` | Moves the pointer over an element. |
| `scroll` | optional `selector`, `index`, `x`, `y` | Scrolls the page or a matching element. |
| `goto` | `path`, optional `query` | Navigates to another same-app route and waits for it to settle. |
| `seedIndexedDB` | `database`, `stores` | Writes JSON fixture records to existing object stores in the scenario's browser context. Each store accepts at most 1,000 records. |
| `delay` | `ms` | Waits for an explicitly bounded interval. Prefer a semantic `waitFor` when possible. |
| `capture` | `name`, optional `mode`, `selector`, `index`, `padding`, `output`, `format`, `quality`, `hideSelectors` | Writes a named `viewport`, `fullPage`, or `element` screenshot. Element mode requires a selector and scrolls the target into view before measuring it. |

One complete working example is in
[`examples/onboarding.plan.json`](https://github.com/Rheosoph/flow-like/blob/dev/apps/desktop/lib/doc-screenshot/examples/onboarding.plan.json). Prefer a plan
when documentation needs multiple states: it is easier to review and rerun
than a sequence of shell commands.

Before each `capture`, use semantic `waitFor` steps for the expected heading,
fixture content, and selected panel. Wait for loading indicators and error
overlays to be hidden. A fixed delay can help rendering settle, but it does not
prove that the intended screen loaded.

Use `seedIndexedDB` for client-owned state that backend fixtures cannot provide,
such as the synthetic conversation in `docs-chat.plan.json`. Wait for the app
to create the database first, then seed its existing stores and reload the
route. The step accepts data only. Label synthetic examples in the captured
content so readers can distinguish them from a live execution.

### Browser diagnostics

Unexpected console errors, page errors, and failed requests make a scenario
fail even when it produced an image. Inspect the diagnostic entries in
`capture-result.json` and fix the application or fixture before publishing.

A scenario may declare `diagnosticAllowlist` for a known, bounded diagnostic:

| Field | Required value |
| --- | --- |
| `kind` | `console`, `page`, or `request` |
| `message` | The exact diagnostic message recorded by the runner |
| `reason` | Why this diagnostic is expected for this scenario |
| `maxCount` | The allowed number of matching entries, from 1 to 100 |

Messages are exact matches, with no wildcard or regular-expression option.
An entry beyond `maxCount` fails the scenario. Keep allowances narrow and
remove them when their cause is fixed.

## Image quality and determinism

The onboarding example uses a 1624 by 1060 CSS-pixel viewport at device scale
factor 2, producing a 3248 by 2120 pixel viewport image. It also fixes the
theme to light, disables CSS animations and transitions, hides scrollbars,
allows 250 ms of settling after actions, and gives cold desktop hydration up
to 120 seconds.

PNG and WebP output are encoded losslessly. JPEG alone uses the optional
numeric `quality` setting. The tool waits for the requested selector and the
page render boundary before capture; it does not upscale screenshots after
capture. Keep fonts, thumbnails, and icons local when repeatability matters.

The capture runtime suppresses page-reload messages from the Next development
server's HMR socket so a background rebuild cannot refresh a scenario midway
through its actions. Application WebSockets retain their normal behavior.

## Desktop Tauri fixtures

Browser Chromium does not have a native Tauri runtime. A desktop plan can
provide a JSON fixture which installs a deterministic IPC mock before any app
code runs. The onboarding fixture uses only checked-in `/swimlanes/*.jpg`
thumbnails and `/flow/icons/*.svg` bit icons, so profile cards do not depend on
remote media.

Fixtures use this shape:

```json
{
  "schema": "flow-like.doc-screenshot-tauri-fixture/v1",
  "strict": true,
  "responses": {
    "get_profiles": {},
    "get_bit_size": 6291456
  }
}
```

Ordinary `responses` entries are keyed by the exact Tauri command name. Every
call to that command receives the same JSON response, independent of its
arguments. Values must therefore be immutable fixture data. To return a field
from the command's JSON arguments, use a response such as
`{ "$argument": "bit.size" }`. The Models fixture uses this to return each
model's size, including zero for hosted models. The field path reads data;
it does not evaluate code. Apart from the built-in SQL, event, and HTTP bridges
described below, `strict: true` rejects
unlisted commands. With `strict: false`, an unlisted command resolves to `null`;
list important calls explicitly even when their response is only a no-op.

A response may model a bounded asynchronous command and emit Tauri events while
it is pending. This is useful for real progress UI such as model downloads:

```json
{
  "$value": [],
  "$delayMs": 60000,
  "$events": [
    {
      "afterMs": 500,
      "name": "download:model-id",
      "payload": {
        "downloaded": 671088640,
        "max": 2147483648
      }
    }
  ]
}
```

Event delays and the command delay are capped at 120 seconds, and at most 100
events are scheduled for one invocation.

The browser fixture supplies disposable in-memory SQLite databases for the
desktop persistence bridge. They let the frontend save state during a scenario
without reading or changing the user's desktop stores. Databases are discarded
when the scenario closes. Browser application storage and cookies are cleared
between scenarios, while the plan reuses a browser context and its static HTTP
cache to avoid downloading the same frontend modules repeatedly.

Tauri HTTP uses a request resource, response metadata, and streamed body reads.
The fixture bridge implements `plugin:http|fetch`, `plugin:http|fetch_send`,
and `plugin:http|fetch_read_body`. Declare JSON bodies under `responses.$http`,
keyed by request pathname:

```json
{
  "$http": {
    "/api/v1/info/features": {},
    "/api/v1/info/home-defaults": { "main": null, "profile": null }
  }
}
```

This excerpt belongs inside `responses`. The bridge returns status 200 and a
JSON body for the selected pathname; it does not distinguish HTTP methods,
query strings, or authentication. Unmatched paths can fall back to legacy
body fixture data or `null`. Use the browser HTTP fixture below when an exact
browser request/response contract is part of the capture.

See
[`fixtures/onboarding.tauri.json`](https://github.com/Rheosoph/flow-like/blob/dev/apps/desktop/lib/doc-screenshot/fixtures/onboarding.tauri.json) for realistic
profile, bit, download, event, updater, notification, registry, tray, and HTTP
responses.

## Browser HTTP fixtures

A plan can set `httpFixture` to serve deterministic browser responses without
starting an API. This is separate from Tauri IPC HTTP mocking and works for
normal `fetch`, XHR, images, and other Chromium requests.

Fixtures use exact request matches:

```json
{
  "schema": "flow-like.doc-screenshot-http-fixture/v1",
  "strict": true,
  "blockedOrigins": [
    "https://telemetry.example.test"
  ],
  "blockedEndpoints": [
    "http://localhost:8080/api/v1/og"
  ],
  "routes": [
    {
      "request": {
        "method": "GET",
        "url": "http://localhost:8080/api/v1/auth/openid"
      },
      "response": {
        "status": 200,
        "headers": {
          "access-control-allow-origin": "*"
        },
        "json": {
          "authority": "http://localhost:8080",
          "client_id": "flow-like-doc-screenshot"
        }
      }
    }
  ]
}
```

A match compares the uppercase HTTP method, canonical absolute URL (including
query order), and, when declared, the raw request body. Omitting `request.body`
accepts any body for that exact method and URL, which is useful for
non-deterministic telemetry envelopes that should be absorbed rather than sent.
There are no URL wildcard or regular-expression matches. A response can
contain either a raw string `body` or a JSON-serializable `json` value; JSON
responses receive an `application/json` content type unless the fixture
declares one.

Same-origin frontend requests always continue so Next.js pages, chunks, and
local assets can load. `blockedOrigins` lists exact HTTP origins whose requests
are intentionally aborted without adding a failed-request diagnostic; use it
for product telemetry that must never leave a documentation capture. `blockedEndpoints`
does the same for one exact origin and path while ignoring its query string,
which is useful for non-essential preview endpoints with dynamic URL
parameters. With `strict: true`, any other unmatched cross-origin HTTP request
is blocked and fails the scenario with its method and redacted URL.
`strict: false` lets unmatched cross-origin requests use the network and should
be reserved for exploratory captures. Cross-origin requests that trigger CORS
preflight need an exact `OPTIONS` route as well as the application request.

The reference plan uses
[`fixtures/docs-reference.http.json`](https://github.com/Rheosoph/flow-like/blob/dev/apps/desktop/lib/doc-screenshot/fixtures/docs-reference.http.json) to
provide the OpenID configuration required by
`/debug/markdown`. The route is public, including when opened without the
plan's `capture=docs` marker, but the app's OpenID fetch remains mandatory: a
missing, mismatched, or invalid fixture response still fails instead of
falling back to an unauthenticated render.

## Results and fixture boundaries

Pass `--json` for a `flow-like.doc-screenshot-result/v1` result on stdout for
scripts or CI. The same result is saved as `capture-result.json`. It reports
scenario and step status, final URL, output files, dimensions, byte counts,
SHA-256 hashes, timings, diagnostics, and capture provenance. Exit code `0`
means every scenario passed, `1` means a scenario, diagnostic, action, or
capture failed, and `2` means the CLI, server, browser, or input contract failed.

Fixture captures verify frontend rendering against declared data. They do not
verify native workflow execution, real authentication, model responses, or
deployed server behavior. Test those paths separately before describing a
capture as evidence that a workflow or integration works.

All input formats are versioned and validated before the browser starts:

- Plans: `flow-like.doc-screenshot-plan/v1`
- Tauri fixtures: `flow-like.doc-screenshot-tauri-fixture/v1`
- Browser HTTP fixtures: `flow-like.doc-screenshot-http-fixture/v1`

Plans are declarative by design. They cannot execute JavaScript or shell
commands. Navigation is limited to application routes, selectors and waits
are timeout-bounded, output names cannot escape the configured output
directory, and fixture files contain JSON only.

Do not place passwords, access tokens, private headers, or other secrets in a
plan, query string, fixture, selector, or filename. These inputs can appear in
logs and result metadata, and the rendered page itself becomes part of the
screenshot. Use synthetic fixture data for documentation captures.

## Render a workflow

`workflow:screenshot` turns a catalog-valid FlowScript document into a rendered
Studio workflow. Use it for FlowBook illustrations and workflow reference images.

Run it from the repository root:

```sh
bun run workflow:screenshot -- \
  apps/book/examples/incident-triage/triage.flow \
  --output apps/book/src/assets/workflows/incident-triage.webp \
  --layout balanced \
  --theme light
```

The pipeline uses the production pieces in their normal order:

1. The Rust helper applies the source to an empty Board through
   `apply_flowscript_to_board` and the complete built-in catalog. Parse or reconcile
   diagnostics stop the command before a browser starts.
2. The resulting Board is formatted with the same `computeFlowLayoutDetailed` engine used
   by Studio. Root and nested/function-layer canvases are all laid out.
3. An ephemeral offline app and Board are exposed to the desktop frontend through the
   documentation Tauri fixture bridge. No real profile, app, or Board is created or changed.
4. The existing documentation screenshot runner opens `/flow` in Chromium and writes a
   lossless WebP/PNG (or JPEG) at the requested viewport and DPR.

### Focus one node or layer

`--focus-node` accepts an exact reconciled ID, a node/layer identity anchor such as
`//@n:abc123` or `//@l:function123`, a unique catalog node name, a friendly name, or a
layer name. The normal `/flow?...&node=<id>` navigation opens the owning layer and frames
the target with Studio's focus behavior.

Generated ids are easiest to discover with:

```sh
bun run workflow:screenshot -- path/to/workflow.flow --list-nodes
```

Then render the detail:

```sh
bun run workflow:screenshot -- path/to/workflow.flow \
  --focus-node normalize \
  --output tmp/workflow-screenshots/normalize.webp
```

An ambiguous selector fails and prints the matching ids instead of silently choosing one.
When a document contains only function declarations, the renderer automatically opens the first
function by stable name/id order so the root's intentionally hidden function layers cannot produce
an empty capture.

### Show generic error handling

`--handle-errors` adds the same `On Error` Execution output and `Error` String output as
Studio's Handle Errors toggle. It accepts the same node ids, node anchors, catalog names, and
friendly names as `--focus-node`; layers and pure nodes are rejected. The adjusted node is focused
automatically unless `--focus-node` explicitly selects another target.

```sh
bun run workflow:screenshot -- path/to/workflow.flow \
  --handle-errors "API Call" \
  --output tmp/workflow-screenshots/api-error.webp
```

The outputs are added only to the ephemeral reconciled Board used for rendering. The FlowScript
source and any real Flow-Like profile remain unchanged.

### Layout and image controls

- `--layout compact|balanced|expanded` selects Studio's layout style. `balanced` is the
  default for book-friendly spacing.
- `--viewport 1624x1060`, `--dpr 2`, and `--theme light|dark` control the deterministic
  browser surface.
- `.webp` and `.png` are lossless. `.jpg`/`.jpeg` can use `--quality`.
- `--frontend-url http://127.0.0.1:3000` reuses an already running desktop frontend.
- `--json` returns the screenshot hash, dimensions, resolved focus id, and nested capture
  result on stdout. Progress and server logs stay on stderr.

The default output is `tmp/workflow-screenshots/<input-name>.webp`.

For repeated captures after building the helper once, set
`FLOW_LIKE_FLOWSCRIPT_RENDER_DATA_BIN=target/debug/flowscript-render-data` to bypass Cargo's
workspace lock and invoke that exact binary directly.
