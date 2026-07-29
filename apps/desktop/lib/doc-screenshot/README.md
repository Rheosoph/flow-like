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

- `tauriFixture` is resolved relative to the plan file. This keeps a plan and
  its fixtures portable when the command is launched from another directory.
- `outputDir` is resolved relative to the process working directory. From the
  repository root, the example therefore writes to
  `tmp/doc-screenshots/onboarding`.

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
app, output directory, optional desktop Tauri fixture, render defaults, and
one or more scenarios. Each scenario starts at `path` plus an optional `query`
object and runs its `steps` in order.

The supported steps are:

| Step | Fields | Behavior |
| --- | --- | --- |
| `waitFor` | one of `selector`, `urlIncludes`, or `text`; optional `state`, `timeoutMs` | Waits for a DOM, URL, or text condition. Selector states are `attached`, `visible`, `hidden`, and `detached`. |
| `click` | `selector`, optional `index`, `button`, `clickCount` | Clicks the matching element. `index` is zero-based. |
| `fill` | `selector`, `value` or `valueEnv`, optional `index` | Replaces the value of an input. |
| `type` | `selector`, `value` or `valueEnv`, optional `index`, `delayMs` | Types into an input. |
| `press` | `key`, optional `selector`, `index` | Sends a keyboard key globally or to an element. |
| `select` | `selector`, `values`, optional `index` | Selects one or more values in a native select element. |
| `check` | `selector`, optional `index`, `checked` | Sets a checkbox or radio control's checked state. |
| `hover` | `selector`, optional `index` | Moves the pointer over an element. |
| `scroll` | optional `selector`, `index`, `x`, `y` | Scrolls the page or a matching element. |
| `goto` | `path`, optional `query` | Navigates to another same-app route and waits for it to settle. |
| `delay` | `ms` | Waits for an explicitly bounded interval. Prefer a semantic `waitFor` when possible. |
| `capture` | `name`, optional `mode`, `selector`, `index`, `padding`, `output`, `format`, `quality`, `hideSelectors` | Writes a named `viewport`, `fullPage`, or `element` screenshot. Element mode requires a selector. |

The full working contract is in
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

Tauri HTTP uses two IPC commands. `plugin:http|fetch` returns a request resource
ID, and `plugin:http|fetch_send` returns response metadata such as status,
headers, URL, and a response resource ID. A `204 No Content` response avoids a
body-read command and is useful for deterministic background requests.

See
[`fixtures/onboarding.tauri.json`](fixtures/onboarding.tauri.json) for realistic
profile, bit, download, event, updater, notification, registry, tray, and HTTP
responses.

## JSON output and safety

Pass `--json` for a `flow-like.doc-screenshot-result/v1` result suitable for
another agent or a CI job. It reports scenario and step status, final URL,
output files, dimensions, byte counts, SHA-256 hashes, timings, and bounded
page error counts. Exit code `0` means every scenario passed, `1` means a
scenario, action, or capture failed, and `2` means the CLI, server, browser, or
input contract failed.

Both input formats are versioned and validated before the browser starts:

- Plans: `flow-like.doc-screenshot-plan/v1`
- Tauri fixtures: `flow-like.doc-screenshot-tauri-fixture/v1`

Plans are declarative by design. They cannot execute JavaScript or shell
commands. Navigation is limited to application routes, selectors and waits
are timeout-bounded, output names cannot escape the configured output
directory, and fixture files contain JSON only.

Do not place passwords, access tokens, private headers, or other secrets in a
plan, query string, fixture, selector, or filename. These inputs can appear in
logs and result metadata, and the rendered page itself becomes part of the
screenshot. Use synthetic fixture data for documentation captures.
