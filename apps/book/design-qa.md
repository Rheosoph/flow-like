# FlowBook redesign QA

## Review target

- Route: `/`
- Intended states: landing page, long-form documentation, light theme, dark theme
- Source of visual truth:
  - `apps/website/src/layouts/v5-landing.astro`
  - `apps/website/src/sections/v5-hero.astro`
  - `apps/website/src/images/parallax/workflow-core.png` (1920 × 1080)
  - `apps/docs/src/assets/RunsAndLogs.webp` (3248 × 2120)
  - `apps/docs/src/assets/FlowLikeAppAnatomy.svg`

## Static comparison

The implementation carries the V5 website system into an editorial reading product: warm off-white and near-black surfaces, ember red/orange accents, tight sans-serif display type, serif narrative copy, small uppercase mono labels, asymmetric layouts, fine rules, and authentic product UI imagery. The homepage deliberately avoids the generic hero-plus-card-grid pattern. Documentation pages retain Starlight search, theme controls, sidebar navigation, table of contents, pagination, and accessible semantic content.

Image handling was verified during the production build. Astro optimized the hero workflow image and runtime tracing image into local WebP assets, and emitted the application-anatomy SVG. Source dimensions and aspect ratios are preserved through the asset pipeline.

## Browser-rendered comparison

- Implementation screenshot: unavailable
- Captured viewport: unavailable
- Full-page comparison: blocked
- Focused component comparison: blocked
- Interaction and console checks: blocked

The in-app browser runtime exposed no browser instances (`agent.browsers.list()` returned an empty list), so no honest browser-rendered screenshot, responsive measurement, interaction pass, or console inspection could be completed in this session.

## Required follow-up

- [P1] Run visual QA at 1440 × 1000, 900 × 1200, and 390 × 844.
- [P1] Verify the hero crop, heading wraps, section spacing, color contrast, and image clarity at each viewport.
- [P1] Exercise Start reading, contents, chapter rails, search, theme switching, mobile navigation, table of contents, and pagination.
- [P1] Inspect both light and dark documentation pages and confirm no runtime or console errors.

## Automated evidence

- `bun run check`: passed with 0 errors, warnings, or hints.
- `bun run build`: passed; 15 pages generated and indexed by Pagefind.
- Static root-relative link and asset scan: passed across all 15 generated HTML files.
- `git diff --check`: passed.

final result: blocked
