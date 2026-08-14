# University management CLI

This repository-local Bun CLI lets Codex, Claude Code, and developers author
Flow-Like University courses without using the admin UI. It follows the same
agent-facing conventions as the documentation screenshot tool: versioned JSON
contracts, clean JSON on stdout, diagnostics on stderr, and deterministic exit
codes.

Run commands from the repository root.

## Set up access

The tool reads credentials only from the environment:

```sh
export FLOW_LIKE_BASE_URL="https://flow-like.example"
export FLOW_LIKE_PAT="pat_..."
```

The PAT owner needs the global `WriteCourses` permission (or Admin). Flow-Like
application API keys cannot authorize University writes. Tokens are deliberately
not accepted as command-line flags, which keeps them out of shell history and
process listings.

## Create a course from a plan

Validate the checked-in example and inspect every planned operation without
making a network request:

```sh
bun run university -- \
  --plan apps/desktop/lib/university/examples/course.plan.json \
  --dry-run \
  --json
```

Remove `--dry-run` to apply the plan, then inspect the complete remote course:

```sh
bun run university -- \
  --plan apps/desktop/lib/university/examples/course.plan.json \
  --json

bun run university -- --inspect agent-authoring-example --json
```

An apply is idempotent when IDs and asset names remain stable. The CLI writes
the full course as a draft first, uploads assets and media, upserts modules,
lessons, challenges, and app references, then reads the remote structure back
for verification. A plan with `isPublished: true` is published only after that
verification passes. A failed run is left as a draft.

Apply does not delete remote modules, lessons, challenges, or app references
that are absent from the plan. This avoids destructive pruning during an agent
retry; verification reports unexpected structural children and prevents
publication until an author removes them deliberately. A same-named asset is reused only when its metadata matches; set the
asset's `replace` field to `true` to force replacement. Without `replace`, the
API exposes no checksum, so matching metadata cannot prove byte equality.

## Add screenshots and files

The screenshot tool's JSON result contains an absolute `path` for every
artifact. Pass that path directly to the University CLI:

```sh
bun run docs:screenshot -- \
  --app web \
  --path /some/page \
  --output tmp/course/editor-overview.webp \
  --json

bun run university -- \
  --asset my-course \
  --name EditorOverview \
  --file /absolute/path/from/the/screenshot/result.webp \
  --json
```

Or put the artifact path in a plan asset:

```json
{
  "name": "EditorOverview",
  "file": "../../../../tmp/course/editor-overview.webp",
  "replace": true
}
```

Reference it from lesson Markdown as `@EditorOverview`. Images render inline;
other asset kinds render as links. Asset names must begin with a letter or
underscore and may contain letters, digits, underscores, and dashes.

## Plan format

Plans use the `flow-like.university-plan/v1` schema and contain one `course`.
Local `contentFile`, asset, icon, and banner paths are resolved relative to the
plan file. The validator reads every file and validates the entire plan before
the first API request.

Every object is strict: an unknown key is an error. Omitted values are
materialized before any API call:

| Scope         | Required                                           | Defaults and rules                                                                                                                                                                                                                                                         |
| ------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Plan          | `schema`, `course`                                 | `schema` must be exactly `flow-like.university-plan/v1`.                                                                                                                                                                                                                   |
| Course        | `name`, non-empty `modules`                        | `id` derives from `name`; `language: "en"`; `difficulty: "BEGINNER"`; `category: "GENERAL"`; `estimatedMinutes: 0`; `isPublished: false`; `tags: []`; `slug`, `position`, `description`, and `longDescription` default to `null`; `assets` and `appLinks` default to `[]`. |
| `media`       | At least one of `icon`, `banner`                   | Both are local image paths. Non-null `iconUrl` and `bannerUrl` are rejected because the API ignores them; use `media.icon` and `media.banner`.                                                                                                                             |
| Asset         | `name`, `file`                                     | `kind`, `mimeType`, `filename`, and extension are inferred from `file`; `replace: false`. Asset names use `[A-Za-z_][A-Za-z0-9_-]{0,63}`.                                                                                                                                  |
| App link      | `appId`                                            | `id` derives deterministically; `purpose: "SHARED_TEMPLATE"`; `alias: null`. Other purposes are `REFERENCE` and `PLAYGROUND`.                                                                                                                                              |
| Module        | `title`, non-empty `lessons`                       | `id` derives from course, position, and title; `position` is its zero-based array index; `description: null`.                                                                                                                                                              |
| Lesson        | `title`, exactly one of `content` or `contentFile` | `id` derives from module, position, and title; language inherits the course; `videoUrl: null`; `estimatedMinutes: 5`; zero-based `position`; `isOptional: false`; `finalAssessment: false`; `challenges: []`; `appRefs: []`.                                               |
| Challenge     | `kind`, `prompt`, `payload`                        | `id` derives from lesson, position, and prompt; `explanation: null`; `points: 10`; zero-based `position`.                                                                                                                                                                  |
| App reference | `kind`, `target`                                   | `id` derives deterministically; `appAlias`, `appId`, and `label` default to `null`. `appAlias` and `appId` are mutually exclusive.                                                                                                                                         |

IDs should be explicit and stable in version control. When omitted, course,
module, lesson, and challenge IDs are derived from their parent, normalized
position, and name, title, or prompt; app-link and app-reference IDs also use
their declaration index. All entity IDs are globally unique, start with an
alphanumeric character, are at most 128 characters, and contain only letters,
digits, dots, underscores, and dashes.

Module positions and titles must be unique in a course; lesson positions and
titles must be unique in a module; challenge positions must be unique in a
lesson. Asset names, app-link app IDs, and non-null app-link aliases must also
be unique. Tags, correct-answer IDs, and required-package IDs cannot contain
duplicates.

### Media, app links, and app references

This course fragment uploads local icon and banner files:

```json
{
  "media": {
    "icon": "media/course-icon.png",
    "banner": "media/course-banner.webp"
  }
}
```

Course media is stored as WebP: the runner uploads an existing WebP unchanged
and converts other supported image formats to WebP at quality 85 first.

Declare an application alias at course scope, then use the same alias in a
lesson reference:

```json
{
  "appLinks": [
    {
      "id": "course-basics.app-link.starter",
      "appId": "source-app-id",
      "purpose": "SHARED_TEMPLATE",
      "alias": "starter"
    }
  ]
}
```

```json
{
  "appRefs": [
    {
      "id": "lesson-welcome.ref.open-boards",
      "kind": "NAVIGATE",
      "appAlias": "starter",
		"label": "Open the starter board",
      "target": {
		"subpath": "flow",
		"params": { "id": "source-board-id" }
      }
    }
  ]
}
```

Any non-null `appAlias` in an app reference, board riddle, or execute-node challenge
must match a non-null `course.appLinks[].alias`. Aliases follow the same
64-character pattern as asset names. App-reference target shapes are:

| Kind                | Exact `target` fields                                                |
| ------------------- | -------------------------------------------------------------------- |
| `NAVIGATE`          | `subpath`: `config`, `events`, `pages`, `flow`, or `use`; optional string-valued `params` object |
| `FOCUS_NODE`        | `boardId`, `nodeId`                                                  |
| `ADD_NODE`          | `boardId`, `nodeTypeId`; optional finite `[x, y]` `coords`           |
| `CREATE_EVENT`      | JSON object `template`                                               |

`OPEN_OR_CLONE_APP` is deliberately rejected in v1 plans because the current
learner pane does not reliably resolve its clone alias. Use a `NAVIGATE`
reference with `appAlias` and `target.subpath: "use"`; opening that action
creates or reuses the course-linked app.

### Challenge payloads

Place challenge objects like these in a lesson's `challenges` array. Choice
option IDs are unique safe IDs. Single choice requires exactly one correct ID;
multiple choice requires one or more:

```json
[
  {
    "id": "lesson-check.challenge.single",
    "kind": "SINGLE_CHOICE",
    "prompt": "Which answer is correct?",
    "position": 0,
    "payload": {
      "options": [
        { "id": "answer-a", "label": "Answer A" },
        { "id": "answer-b", "label": "Answer B" }
      ],
      "correct": ["answer-a"]
    }
  },
  {
    "id": "lesson-check.challenge.multiple",
    "kind": "MULTIPLE_CHOICE",
    "prompt": "Select both safeguards.",
    "position": 1,
    "payload": {
      "options": [
        { "id": "draft-first", "label": "Create the draft first" },
        { "id": "verify", "label": "Verify before publishing" },
        { "id": "skip-validation", "label": "Skip validation" }
      ],
      "correct": ["draft-first", "verify"]
    }
  }
]
```

A board riddle requires exactly one of `appAlias` or `appId`, a `boardId`, and
one or more predicates. This valid payload demonstrates every supported
predicate operation:

```json
{
  "id": "lesson-board.challenge.riddle",
  "kind": "BOARD_RIDDLE",
  "prompt": "Build the requested flow.",
  "position": 0,
  "payload": {
    "appAlias": "starter",
    "boardId": "source-board-id",
    "predicates": [
      { "op": "requires_nodes", "args": ["package.node-required"] },
      { "op": "forbids_nodes", "args": ["package.node-forbidden"] },
      { "op": "max_nodes", "args": [12] },
      { "op": "min_nodes", "args": [2] },
      {
        "op": "has_connection",
        "args": ["package.node-source", "package.node-target"]
      },
      {
        "op": "pin_value_equals",
        "args": ["package.node-target", "value", 42]
      }
    ]
  }
}
```

An execute-node challenge uses the same target rule and requires a non-empty,
duplicate-free package proof list. `nodeId` guides the learner UI; current
server scoring proves the completed run's app, board, and streamed packages,
but does not independently verify that exact node ID:

```json
{
  "id": "lesson-run.challenge.execute",
  "kind": "EXECUTE_NODE",
  "prompt": "Run the configured node.",
  "position": 0,
  "payload": {
    "appAlias": "starter",
    "boardId": "source-board-id",
    "nodeId": "source-node-id",
    "requiredPackages": ["package.output-record"]
  }
}
```

Challenge payload keys and their camelCase spelling are exact; unknown keys or
unsupported board predicate operations are rejected.

Alias-targeted board or execute-node challenges need an app reference in the
same or an earlier lesson that opens or focuses that alias before the learner
can submit them. The backend cannot resolve an unopened course-app alias.

### Limits

- `content` and UTF-8 `contentFile` are non-empty and at most 1,500,000 bytes,
  leaving safe headroom under the API's default JSON request-body limit.
- Every media or asset path must identify a regular file of at most
  2,147,483,647 bytes. Paths are relative to the plan file unless absolute.
- Media files must have an inferable image extension. Asset extensions are
  1–10 alphanumeric characters; asset filenames and MIME types are at most 255
  UTF-8 bytes. Explicit non-document asset kinds must match their MIME family.
- Numeric positions, points, and duration fields are non-negative 32-bit
  integers. Languages use a BCP 47-style tag, slugs use lowercase words joined
  by single dashes, and non-null `videoUrl` values are credential-free absolute
  HTTP or HTTPS URLs.

All API replacement bodies are fully populated so omitted options cannot
silently reset existing values.

Apply never prunes undeclared remote children. When renaming or changing a
generated ID, verification identifies stale modules, lessons, challenges, and
references and leaves the course as a draft; remove that content deliberately
in the admin UI, then rerun the plan. Application references have no persisted position in
the current API, so do not rely on array order to select a default action.

Flow-Like currently models an end-of-course test as a lesson with challenges,
not as a separate exam entity. Mark such a lesson with `finalAssessment: true`.
The validator then requires it to be the last lesson by module and lesson
position, non-optional, and to contain at least one challenge.

See [`examples/course.plan.json`](examples/course.plan.json) for a complete
course with an asset, Markdown file, modules, lessons, and a final assessment.

## Other commands

List every course, including drafts when the PAT permits it:

```sh
bun run university -- --list --json
```

Upload or explicitly replace a single asset:

```sh
bun run university -- \
  --asset my-course \
  --name EditorOverview \
  --file tmp/course/editor-overview.webp \
  --replace \
  --json
```

Use `--api-url` to override `FLOW_LIKE_BASE_URL`, `--language` with list or
inspect, and `--timeout-ms` to set a bounded whole-command timeout.

## Machine-readable behavior

`--json` returns a versioned University result on stdout. Signed storage URLs
and authentication values are never included. Exit code `0` means success,
`1` means an API operation or remote verification failed, and `2` means CLI
usage, environment configuration, or local plan validation failed.
