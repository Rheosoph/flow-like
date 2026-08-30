# FlowBook standalone hero image QA

## Comparison target

- Route: `/`
- State: dark landing-page hero, default viewport state
- Source visual truth: `apps/website/src/images/parallax/workflow-core.png`
- Source dimensions: 1920 × 1080 RGBA PNG at intrinsic density
- Source alpha bounds: x 173–1776, y 10–1070; the angled application silhouette and transparent margins are part of the intended composition
- Implementation: `apps/book/src/components/BookHero.astro`
- Intended implementation viewports: 2012 × 1248 reference viewport, 900 × 1200 tablet, and 390 × 844 mobile
- Implementation screenshot: unavailable
- CSS viewport and device scale factor: unavailable
- Density normalization: not performed because no browser-rendered implementation capture was available

## Full-view comparison evidence

Blocked. The source PNG was opened at original resolution and its transparency was inspected, but the implementation could not be captured in a browser. The in-app browser was unavailable and browser discovery returned no connected browser instances.

## Focused-region comparison evidence

The source asset itself was verified to contain the desired perspective-skewed board and FlowScript panels with real transparency. The outer rounded card, header/footer labels, clipping background, perspective transform, and float animation were confirmed to be DOM/CSS presentation chrome rather than part of the asset. Those layers were removed, and the image now renders directly with an alpha-following `drop-shadow`.

A focused rendered comparison is still blocked because no implementation screenshot can be captured in the available browser surface.

## Findings

- [P1] Browser-rendered visual verification is unavailable.
  - Location: landing-page hero at `/`.
  - Evidence: source asset is available and inspected; implementation screenshot, viewport measurements, responsive captures, and browser console output are unavailable.
  - Impact: the standalone alpha treatment is implemented, but its final scale and crop cannot be honestly approved at desktop, tablet, or mobile widths in this session.
  - Fix: capture the rendered hero at the three intended viewports, compare it with the source asset, and adjust only image scale/offset if the silhouette is clipped or feels undersized.

## Required fidelity surfaces

- Fonts and typography: unchanged by this focused edit; browser comparison blocked.
- Spacing and layout rhythm: wrapper chrome was removed and transparent-canvas compensation added; browser comparison blocked.
- Colors and visual tokens: existing hero background retained; image receives only alpha-following shadows; browser comparison blocked.
- Image quality and asset fidelity: original RGBA source asset is used through Astro Image with transparency preserved; production build succeeded.
- Copy and content: unchanged by this focused edit.

## Comparison history

- No visual iteration could be recorded because the first browser-rendered capture was unavailable.
- Static implementation removed the previously identified frame, labels, clipping, perspective rotation, and float animation.

## Automated evidence

- `bun run check`: passed with 0 errors, warnings, or hints.
- `bun test src`: passed, 8 tests.
- `bun run build:site`: passed; 19 pages generated and indexed by Pagefind.

## Implementation checklist

- [x] Render the source image directly inside the hero figure.
- [x] Remove the rounded frame, frame bars, labels, caption, and solid image background.
- [x] Remove the extra perspective transform and floating animation.
- [x] Preserve alpha and use `drop-shadow` rather than `box-shadow`.
- [ ] Capture and compare desktop, tablet, and mobile browser renders.
- [ ] Check primary hero actions and browser console errors.

final result: blocked
