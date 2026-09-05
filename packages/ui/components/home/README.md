# Editable home pages

Web and desktop use the same home renderer and editor. A layout contains ordered widgets with stable IDs, column and row spans, appearance, and configuration. Widget data loads through the current viewer's backend state; published defaults contain configuration only.

## Profiles and defaults

The renderer selects the first valid layout in this order:

1. The profile's personal `home_layout`.
2. The latest published default referenced by `home_default_id`.
3. The latest main default for the backend.
4. The bundled layout in `catalog.ts`.

An empty layout is intentional. Reset clears the personal override and retains its default association. Publishing a new default therefore updates profiles that inherit it without replacing personal layouts.

Administrators with `WriteLandingPage` or `Admin` permission use `/admin/home` to edit the main default or a profile template's default. Publication checks the revision loaded when editing began. A conflicting publication retains the draft and explains how to review the latest default.

Apply the schema before deploying the API. See [migration and rollout instructions](../../../api/prisma/home-layouts.md).

## Editing

Customize opens the catalog and widget inspector. Users can add, reorder, resize, duplicate, configure, and remove widgets. Save commits the draft; Cancel preserves the saved layout. Reset remains undoable until saved. The editor supports up to 80 widgets and a 128 KiB configuration document.

Drag handles support keyboard pickup, arrow movement, and drop through DnD Kit. Resize handles support arrow keys. Command/Ctrl+Z undoes changes, Shift+Command/Ctrl+Z redoes them, and Command/Ctrl+S saves. The options menu also moves widgets earlier or later.

The canvas adapts to one, six, or twelve columns. The mobile inspector uses a modal sheet with focus trapping and focus return. Desktop, tablet, and phone previews share the same layout order. Unsaved drafts survive route and profile changes in the current app session. Closing the app ends that session; a browser unload warning protects an open dirty editor.

## Widget catalog

The 79 presets cover four FlowPilot forms, native app embeds, app collections, packages, models, categories, quick actions, links, rich content, personal activity, and 32 data presentations. Presets share configurable renderers rather than separate persistence formats.

App embeds use the existing native page and event runtime. They can open an app's landing page, a route, or an event/chat with query parameters. Each embed owns its navigation state. Embedded navigation does not change the home URL or another widget. The editor shows a preview without starting the embedded runtime. Arbitrary external iframes and app-published widget extensions are outside this release.

Data widgets read project or personal tables, local ontology object types, and saved queries through Data Studio. The inspector configures measures, aggregation, grouping, date buckets, typed filters, current-viewer filters, sorting, limits, formatting, targets, and refresh. Aggregation happens at the source before the workbench result limit. Saved queries retain their own SQL limits. Truncation and errors remain visible.

Data presentations include metrics, targets, column and ranked bars, stacked and percentage charts, lines, areas, distributions, heatmaps, treemaps, funnels, waterfalls, Sankey, pivot tables, record views, calendars, timelines, and graphs. Box plot quartiles are approximate. Sankey uses two stages. Comparisons display at most ten records; record calendars display at most 24 returned months.

Activity widgets use the authenticated viewer's recorded usage and notifications. Run charts show coverage of the latest 100 execution records. Log severity is not a workflow success rate. Schedule previews calculate upcoming dates from saved event schedules and timezones; they do not confirm live scheduler state. AI cost cards use reported usage costs and do not infer remaining credits.

## Verification

Pure query, layout, embed, schedule, and activity tests use Bun. Profile cache and desktop sync tests use Vitest. Type-check both application projects after changing shared components.

The isolated browser harness renders the production editor and native app runtime with fixture backend data. Run it from the repository root:

```sh
bunx vite --config tests/home-browser/vite.config.ts
```

In a second terminal:

```sh
node tests/home-browser/verify.mjs
node tests/home-browser/content-verify.mjs
node tests/home-browser/verify-data.mjs
node tests/home-browser/activity-verify.mjs
```

The scripts use Chrome on macOS or Playwright's default Chromium elsewhere. Set `CHROME_EXECUTABLE_PATH` to choose a different executable. They block outbound requests, exercise editor and native embed behavior, and save screenshots and reports in the temporary directory. This fixture verifies shared UI behavior; it does not replace a signed-in backend or native desktop smoke test.
