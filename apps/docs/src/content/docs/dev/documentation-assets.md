---
title: Maintaining site content
description: Build the website and docs, maintain their Markdown responses, and prepare screenshots for site content.
---

The marketing website lives in `apps/website`; the documentation site lives in
`apps/docs`. Keep contributor instructions in these docs and use the site source
directories for content, assets, and implementation.

## Run and build the sites

Install the repository's toolchain as described in
[Building from Source](/dev/build/), then run commands from the repository root:

```sh
bun install
mise run dev:website
```

Use `mise run dev:docs` for the documentation server. The production build
commands are `mise run build:website` and `mise run build:docs`. The website
writes its static assets to `apps/website/dist/client`; the docs write to
`apps/docs/dist`.

Documentation pages belong in `apps/docs/src/content/docs/`. Add a title and
description in frontmatter, link related pages, and add new topics to the
sidebar in `apps/docs/src/sidebar.mjs`. Use the
[screenshot tools](/dev/documentation-screenshots/) when a procedure needs a
capture of the actual application.

## Check a documentation change

Run these commands from the repository root before opening a pull request:

```sh
bun run --cwd apps/docs check:source
bun apps/docs/scripts/generate-flowpilot-corpus.ts
bun run --cwd apps/docs build
bun run --cwd apps/docs check
bun test apps/docs/src/components/node-docs/NodeReference.test.tsx
bun apps/docs/scripts/check-sdk-examples.ts
uv run --with httpx python apps/docs/scripts/check-sdk-examples.py
```

The source check validates frontmatter, code fences, and sidebar coverage. The
built check follows internal links and headings, verifies screenshot files and
search scopes, and checks HTML and image budgets. The FlowPilot corpus check
requires its generated JSON to match the selected source guides. Commit that
JSON when those guides change.

For browser checks, serve `apps/docs/dist` on port 4339 and run
`bun run --cwd apps/docs check:browser` in another terminal. Set
`DOCS_BROWSER_EXECUTABLE` to your Chrome executable if Puppeteer has no browser
installed. The checks cover search, the catalog directory, mobile layout,
image expansion, chart examples, and the 404 page. They save previews under
`/tmp/flow-like-docs-preview`. The Documentation workflow runs these checks
on relevant pull requests and uploads its previews for review.

Keep node reference changes in their Rust node definitions or
`packages/catalog/src/bin/generate_node_docs.rs`. Run `bun run docs:nodes`
to regenerate the catalog; do not patch individual generated pages. Use
`bun run docs:nodes -- --check` to compare the generated output without
writing files. CI runs that comparison against the metadata-only Rust catalog.

## Screenshot size and readability

Use a screenshot to show the controls a reader needs for that step. Prefer a
focused panel or populated workspace over several copies of the same overview.
Keep the source image large enough to inspect labels when opened separately.
Markdown images get responsive WebP derivatives at 480, 800, 1200, and 1600
pixels, plus their source width. Readers can open large article images at full
size by selecting them.

The built check limits each derivative up to 1600 pixels to 450 KB and each
page's unique images at the 800-pixel article size to 1.5 MB. It also limits
Quick Start HTML to 200 KB and the catalog overview to 2 MB. These budgets use
uncompressed build artifacts; they do not represent a production network
measurement. Check phone and desktop previews before raising a budget or
reducing image quality.

## Markdown responses

Both sites provide Markdown versions of prerendered HTML pages. A request with
`Accept: text/markdown`, or a page URL ending in `.md`, selects that version.
Ordinary browser requests receive HTML. On-demand website routes such as
`/store/**` have no prebuilt Markdown version and continue to serve HTML.

Each site's `build` script runs Astro and then
`scripts/agent-markdown/generate.mjs`. The generator writes an adjacent `.md`
file for each built HTML page and an `llms.txt` index. Running `astro build`
alone does not run this post-build conversion.

The docs serve these files through `apps/docs/functions/_middleware.ts`. The
website's `scripts/prepare-workers-sites-deploy.mjs` copies the negotiation
module into its Worker entry. Markdown responses include
`Content-Type: text/markdown; charset=utf-8`, `Vary: Accept`, a canonical `Link`
header, and an `x-markdown-tokens` estimate.

The `html-to-markdown.mjs` converters in both sites are mirrored. Keep their
behavior in sync when changing conversion. To check the website's negotiated
response locally, run the following from `apps/website`:

```sh
bun run build
node scripts/prepare-workers-sites-deploy.mjs
bunx wrangler dev --port 8798
```

Then, in another terminal:

```sh
curl -i http://127.0.0.1:8798/pricing/ -H 'Accept: text/markdown'
```

For the docs, build from `apps/docs`, start
`bunx wrangler pages dev --port 8799`, and request
`http://127.0.0.1:8799/dev/rust/` with the same header. Check both the response
headers and the page content; a successful HTML response alone does not verify
content negotiation.

## Website App screenshots

The Company App Store carousel can show real screenshots in place of its CSS
mockups. Add screenshots to `apps/website/public/images/apps/`, then set the
optional screenshot URL in the matching `carouselApps` entry in
[`v5-apps.astro`](https://github.com/Rheosoph/flow-like/blob/dev/apps/website/src/sections/v5-apps.astro).
For example, use `/images/apps/operations-hq.webp` after adding that file.
Leave the URL as `null` while an image is unavailable so the card uses its
mockup without requesting a missing asset.

| Filename | App card |
| --- | --- |
| `operations-hq.webp` | Operations HQ |
| `finance-approvals.webp` | Finance Approvals |
| `customer-360.webp` | Customer 360 |
| `knowledge-graph.webp` | Knowledge Graph |
| `ai-control-center.webp` | AI Control Center |
| `inventory-planner.webp` | Inventory Planner |
| `flowpilot-workspace.webp` | FlowPilot Workspace |
| `field-service.webp` | Field Service |

Use WebP screenshots of populated application screens without personal data.
Export around 1320 × 620 pixels or wider, and aim for less than 300 KB per image.
The carousel crops images with `object-fit: cover` and anchors them at the top;
keep the important content near the top edge and check the crop at phone and
desktop widths.

For artwork with transparent margins, preserve the alpha channel. Use a
`drop-shadow` when the shadow should follow the visible silhouette. Inspect the
rendered page at its intended widths before judging scale or clipping; the
source image alone cannot show the final browser layout.
