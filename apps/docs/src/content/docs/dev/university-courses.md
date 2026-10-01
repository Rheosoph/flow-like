---
title: Author University courses
description: Create and verify University courses with JSON plans, media assets, and assessment challenges
---

Use a versioned JSON plan to author a University course, upload its media,
and verify the saved lessons before publication. The repository CLI can also
inspect courses and upload individual assets.

Install the [repository dependencies](/dev/build/) and run commands from the
repository root. `--json` reserves stdout for results; diagnostics go to stderr.

## Check the curriculum locally

Run the repository-wide check before applying edited courses:

```sh
bun run university:check
bun run university:check -- --json
```

The checker discovers every `course.plan.json` beneath
`apps/desktop/lib/university/courses`, loads its lesson files and assets, and
checks IDs across courses. A course's `estimatedMinutes` must equal the sum
of its required lessons. Output includes required and optional time, lesson
and question counts, and approximate whitespace-based word counts. It makes
no API requests.

`apps/desktop/lib/university/curriculum.json` defines six learning paths by
learner goal. Each path lists ordered `courses` and optional `electives` as
plan paths relative to the catalog. The checker resolves those references
against the discovered plans and reports each path's time. Use
`--catalog path/to/curriculum.json` to validate another catalog with the same
schema and a neighboring `courses/` directory. The default command validates
local files without publishing them.

## Keep a course focused

Give each course one outcome. State prerequisites and the required environment
in `longDescription`, followed by core time and any optional time. Teach a
concept in its owning course; later courses should use it in a new exercise
and link to the reference instead of repeating the introduction.

Mark elective classes with `isOptional: true` and exclude their time from the
course estimate. Keep the final assessment required and last, after any
optional classes. Use a short concept check where it helps and a small final
assessment that applies the skill to a new case.

For practice, specify the actual node names and pin connections, synthetic
inputs, and expected outputs. Put downloadable inputs or source under the
course's `fixtures/` directory and register them as named `DOCUMENT` assets.
Include an empty or failing case when it teaches a meaningful boundary.
Check FlowScript against current declarations and compiler diagnostics before
asking learners to apply it.

Manual checks must be described as manual. Use `BOARD_RIDDLE` or
`EXECUTE_NODE` only when the plan has real, resolvable App and board targets;
a fixture download alone does not create an automatically scored exercise.
Preserve IDs when the meaning remains the same, so progress and references
remain connected to the intended lesson or challenge.

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

By default, apply keeps remote children absent from the plan. Verification
reports every unexpected module, lesson, challenge, App reference, and App
link in `data.retirements` and prevents publication. Use the explicit
`--prune` option below to retire them. A same-named asset is reused only when its metadata matches; set the
asset's `replace` field to `true` to force replacement. Without `replace`, the
API exposes no checksum, so matching metadata cannot prove byte equality.

### Compatibility with older media workers

Older media workers process course downloads as images and delete formats such
as JSON and ZIP. Until the updated worker is deployed, add
`--legacy-media-assets` to a plan apply or a single-asset upload:

```sh
bun run university -- --plan path/to/course.plan.json --legacy-media-assets --json
bun run university -- --asset <course-id> --name Cases --file cases.json --replace --legacy-media-assets --json
```

This opt-in mode requests a `.webp` storage key for `DOCUMENT` assets because
the older worker leaves that extension untouched. The original filename, MIME
type, asset kind and bytes are preserved. Uploads set `Content-Disposition`
with the original download filename, including a UTF-8 filename parameter.
Other asset kinds keep their normal storage extensions.

If an existing document still uses an affected key, explicitly replace it
with the single-asset command above or the plan asset's `replace: true` field.
The flag does not repair files already deleted by the worker without that
replacement. After deploying the corrected media worker, omit the flag for
new uploads. Existing compatibility uploads remain usable; replacing one
without the flag restores its normal storage extension.

## Review removals before applying a shorter course

Compare the current course plans with a Git commit before reconciling existing
remote content:

```sh
bun run university:check -- --compare-ref HEAD --json
```

The `migration` result lists removed module, lesson, challenge, App-link, and
App-reference IDs, plus IDs moved to another parent. Each entry includes its
source location; moved entries also include the destination. This compares
repository versions, not the remote database. It does not delete content or
transfer learner progress.

The API cannot move an existing module, lesson, challenge, or App reference
to another parent. Give moved entities a new stable ID, such as the former ID
with `-v2` appended. When a lesson receives a new ID, its retained challenges
and App references also need new IDs. Keep IDs whose direct parent remains
unchanged. A new lesson starts with new lesson-level progress; retiring its
former ID deletes the old progress and challenge attempts.

Inspect the remote course and review the retirement list. To apply the plan
and delete its obsolete children:

```sh
bun run university -- --plan path/to/course.plan.json --prune --json
```

The runner drafts the course and upserts the target content before inspecting
all existing descendants. It refuses to delete a parent containing an ID
retained elsewhere in the plan, or to delete anything when a target ID is
missing. It then deletes obsolete roots through the course API and verifies
the resulting course before publication. Module and lesson deletion cascades
to their children and learner records. Course assets are retained. The JSON
result lists each requested retirement and its status; a parent retirement
includes its descendants through that cascade. Avoid concurrent authoring
while applying a plan because the API has no conditional update token.

If deletion returns `202`, the runner stops and leaves the course as a draft.
The error includes the deletion job ID. An administrator must confirm `DONE`
at `GET /admin/deletions/{job_id}` before retrying. Pending and failed jobs
are hidden from course structure, so a later import cannot detect them from
that view. The CLI does not automatically retry or cancel deletion jobs.

`--prune --dry-run` stays offline and shows the retirement step; discovering
remote IDs requires an apply or `--inspect`. Authors can also retire content
through the authoring interface. `--compare-ref` cannot be combined with
`--apply-paths`, so the comparison remains read-only.

## Publish the learning paths

Apply and publish the referenced course plans first, then use the environment
credentials described above to publish the catalog's ordered paths:

```sh
bun run university:check -- --apply-paths --json
```

The command validates every local course before making an API request. For
each path it writes a draft, upserts the core course positions, and reads the
path back. It publishes only when metadata and ordered steps match, and every
core course exists and is published remotely. Elective names appear in the
path description; they are not added to its ordered steps.

An existing step absent from the catalog leaves that path as a draft. The
result names the unexpected course IDs so an author can remove those steps
deliberately before retrying. No path steps, courses, or lessons are silently
deleted. Other paths are processed independently.

If a publication response is lost or fails verification, the command attempts
to restore that path to a draft. A `failed` result reports when it cannot
confirm recovery; inspect that remote path before retrying. Requests use the
University client's PAT handling and HTTPS requirement, with a two-minute
command timeout and a separate ten-second recovery attempt.

## Add screenshots and files

The [screenshot tool](/dev/documentation-screenshots/)'s JSON result contains an absolute `path` for every
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

When changing an ID, remove its obsolete remote entity in the admin UI before
rerunning the plan. Application references have no persisted position in the
current API, so do not rely on array order to select a default action.

An end-of-course test is a lesson with `finalAssessment: true` and at least
one challenge. It must be the last lesson by module and lesson position and
must be required (`isOptional: false`). A course can contain at most one final
assessment, and publication requires one.

See [`examples/course.plan.json`](https://github.com/Rheosoph/flow-like/blob/dev/apps/desktop/lib/university/examples/course.plan.json) for a complete
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
inspect, and `--timeout-ms` to set the whole-command timeout (120,000 ms by
default, at most 300,000 ms).

## Machine-readable behavior

`--json` returns a versioned University result on stdout. Signed storage URLs
and authentication values are never included. Exit code `0` means success,
`1` means an API operation or remote verification failed, and `2` means CLI
usage, environment configuration, or local plan validation failed.
