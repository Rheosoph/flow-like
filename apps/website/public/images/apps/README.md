# App-store screenshots (landing `#apps` section)

Drop a real app screenshot in here, then set that app's optional screenshot URL in
the `carouselApps` entry in
[`v5-apps.astro`](../../../src/sections/v5-apps.astro). It crossfades into the
matching card in the "Company App Store" carousel.

Keep the URL set to `null` until the file exists. The card then uses its built-in CSS
mockup without issuing a failed image request.

## Filenames (slug → app)

| File                          | App card             |
| ----------------------------- | -------------------- |
| `operations-hq.webp`          | Operations HQ        |
| `finance-approvals.webp`      | Finance Approvals    |
| `customer-360.webp`           | Customer 360         |
| `knowledge-graph.webp`        | Knowledge Graph      |
| `ai-control-center.webp`      | AI Control Center    |
| `inventory-planner.webp`      | Inventory Planner    |
| `flowpilot-workspace.webp`    | FlowPilot Workspace  |
| `field-service.webp`          | Field Service        |

## Image spec

- **Format:** `.webp` (matches the rest of `public/images/product/`).
- **Aspect:** the card body is roughly **2.1 : 1** (wide). Screenshots are shown
  `object-fit: cover` anchored to the **top**, so keep the app header / hero near
  the top edge and don't rely on the very bottom of the image.
- **Size:** export at ~**1320 × 620** (or wider) for crisp 2× rendering, then
  compress. Aim to stay well under ~300 KB per file.
- Capture the real Flow-Like app UI — a clean, populated screen with no personal data.
