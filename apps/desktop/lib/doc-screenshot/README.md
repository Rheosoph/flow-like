# Documentation screenshot CLI

This CLI opens real Flow-Like routes in Chromium, performs a small,
declarative interaction plan, and writes deterministic, documentation-ready
screenshots. It is intended for agents such as Codex and Claude Code as well as
local use.

Run commands from the repository root.

## Capture the onboarding example

The checked-in plan opens onboarding with `capture=docs`, captures the initial
profile grid, selects the fourth profile, and captures both the selected and
completed states:

```sh
bun run docs:screenshot -- \
  --plan apps/desktop/lib/doc-screenshot/examples/onboarding.plan.json \
  --json
```

The three lossless WebP files are written below
`tmp/doc-screenshots/onboarding`.

Plan paths have two deliberately different bases:

- `tauriFixture` and `httpFixture` are resolved relative to the plan file. This
  keeps a plan and its fixtures portable when the command is launched from
  another directory.
- `outputDir` is resolved relative to the process working directory. From the
  repository root, the example therefore writes to
  `tmp/doc-screenshots/onboarding`.

## Refresh the checked-in documentation screenshots

The documentation plans write directly to `apps/docs/src/assets` and use dark
mode, a 1624 by 1060 CSS-pixel viewport, DPR 2, and lossless WebP:

```sh
bun apps/desktop/scripts/generate-doc-screenshot-fixtures.ts
bun apps/desktop/scripts/generate-doc-studio-screenshot-fixture.ts

bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-start.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-apps.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-ontology.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-sharing.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-roles.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-studio.plan.json
bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/docs-reference.plan.json
```

Each plan starts from application routes and performs the navigation and UI
interactions needed to expose the documented state. A failed wait or action
fails that scenario instead of silently writing a loading or error screen.

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
| `delay` | `ms` | Waits for an explicitly bounded interval. Prefer a semantic `waitFor` when possible. |
| `capture` | `name`, optional `mode`, `selector`, `index`, `padding`, `output`, `format`, `quality`, `hideSelectors` | Writes a named `viewport`, `fullPage`, or `element` screenshot. Element mode requires a selector and scrolls the target into view before measuring it. |

One complete working example is in
[`examples/onboarding.plan.json`](examples/onboarding.plan.json). Prefer a plan
when documentation needs multiple states: it is easier to review and rerun
than a sequence of shell commands.

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

`responses` is keyed by the exact Tauri command name. Every call to that
command receives the same JSON response, independent of its arguments. Values
must therefore be immutable fixture data, not stateful behavior. With
`strict: true`, an unlisted command fails the scenario. With `strict: false`,
an unlisted command resolves to `null`; list important calls explicitly even
when their response is only a no-op.

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

Tauri HTTP uses two IPC commands. `plugin:http|fetch` returns a request resource
ID, and `plugin:http|fetch_send` returns response metadata such as status,
headers, URL, and a response resource ID. A `204 No Content` response avoids a
body-read command and is useful for deterministic background requests.

See
[`fixtures/onboarding.tauri.json`](fixtures/onboarding.tauri.json) for realistic
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
are intentionally aborted without failing the scenario; use it for product
telemetry that must never leave a documentation capture. `blockedEndpoints`
does the same for one exact origin and path while ignoring its query string,
which is useful for non-essential preview endpoints with dynamic URL
parameters. With `strict: true`, any other unmatched cross-origin HTTP request
is blocked and fails the scenario with its method and redacted URL.
`strict: false` lets unmatched cross-origin requests use the network and should
be reserved for exploratory captures. Cross-origin requests that trigger CORS
preflight need an exact `OPTIONS` route as well as the application request.

The reference plan uses
[`fixtures/docs-reference.http.json`](fixtures/docs-reference.http.json) to
provide the OpenID configuration required by
`/debug/markdown`. The route is public, including when opened without the
plan's `capture=docs` marker, but the app's OpenID fetch remains mandatory: a
missing, mismatched, or invalid fixture response still fails instead of
falling back to an unauthenticated render.

## JSON output and safety

Pass `--json` for a `flow-like.doc-screenshot-result/v1` result suitable for
another agent or a CI job. It reports scenario and step status, final URL,
output files, dimensions, byte counts, SHA-256 hashes, timings, and bounded
page error counts. Exit code `0` means every scenario passed, `1` means a
scenario, action, or capture failed, and `2` means the CLI, server, browser, or
input contract failed.

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
