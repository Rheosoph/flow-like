---
title: Customizing & White-Label
description: Technical guide for customizing Flow-Like's appearance and behavior
sidebar:
  order: 30
---

This guide covers the technical aspects of customizing Flow-Like — from simple theme changes to complete rebranding. For white-label licensing and business options, see [Enterprise White-Labeling](/enterprise/whitelabeling/).

:::tip[Looking for white-label licensing?]
If you want to deploy Flow-Like under your own brand for customers, check out our [Enterprise White-Labeling](/enterprise/whitelabeling/) page for licensing options, pricing, and professional services.
:::

## Quick Overview

| Customization Level | Complexity | What You Can Change |
|---------------------|------------|---------------------|
| **Themes** | Low | Colors, fonts, light/dark mode |
| **Branding** | Low | Logo, app name, metadata |
| **Components** | Medium | UI elements, layouts |
| **Node Editor** | Medium | Node appearance, canvas settings |
| **Engine Embedding** | High | Integrate into your own product |

---

## Theming System

Flow-Like uses Tailwind CSS with CSS custom properties. The theme system supports:

- Light and dark modes
- Custom color palettes
- Multiple built-in themes (Cosmic Night, Bubblegum, Neo Brutalism, etc.)

### Theme Structure

Themes are defined using CSS custom properties in HSL format:

```css
:root {
  --background: 0 0% 100%;
  --foreground: 222.2 84% 4.9%;
  --primary: 222.2 47.4% 11.2%;
  --primary-foreground: 210 40% 98%;
  --accent: 210 40% 96.1%;
  --muted: 210 40% 96.1%;
  --border: 214.3 31.8% 91.4%;
  --destructive: 0 84.2% 60.2%;
  --ring: 222.2 84% 4.9%;
  --radius: 0.5rem;
}

.dark {
  --background: 222.2 84% 4.9%;
  --foreground: 210 40% 98%;
  /* ... dark mode overrides */
}
```

### Creating a Custom Theme

1. Define your color palette as HSL values
2. Create CSS custom properties for each semantic color
3. Add the theme to the theme selector in `packages/ui/components/theme-provider.tsx`

---

## Branding

### Logo Files

Replace these files in `apps/desktop/public/`:

| File | Usage |
|------|-------|
| `app-logo.webp` | Light mode logo |
| `app-logo-light.webp` | Dark mode logo |
| `favicon.ico` | Browser favicon |
| `android-chrome-*.png` | Android icons |
| `apple-touch-icon.png` | iOS icon |

### App Name & Metadata

Update the app name in these locations:

```json
// apps/desktop/src-tauri/tauri.conf.json
{
  "productName": "Your App Name",
  "identifier": "com.yourcompany.yourapp"
}
```

```json
// apps/desktop/package.json
{
  "name": "your-app-name",
  "productName": "Your App Name"
}
```

```json
// flow-like.config.json
{
  "appName": "Your App Name"
}
```

---

## UI Components

The UI is built on [shadcn/ui](https://ui.shadcn.com/) components in `packages/ui/components/ui/`.

### Customization Levels

1. **Global styles** — Modify `packages/ui/styles/globals.css`
2. **Component variants** — Edit component files in `packages/ui/components/ui/`
3. **Instance overrides** — Pass `className` props to components

### Icon System

Flow-Like uses [Lucide](https://lucide.dev/) icons:

```tsx
import { Plus, Settings, Workflow } from "lucide-react";

<Plus className="h-4 w-4" />
<Settings className="h-5 w-5 text-muted-foreground" />
```

---

## Node Editor Customization

### Node Appearance

Customize nodes in the visual editor:

- **Category colors** — Defined per node category
- **Icons** — SVG icons referenced in node definitions
- **Pin styling** — Input/output pin appearance

### Canvas Settings

Configurable canvas options:

| Setting | Location |
|---------|----------|
| Grid size | Editor settings |
| Snap-to-grid | Editor settings |
| Zoom limits | `packages/ui/components/flow/` |
| Background pattern | Canvas component |
| Connection line style | Edge components |

---

## Engine Embedding

For deeper integration, you can embed Flow-Like's workflow engine into your own product.

### Rust Core

Use `flow-like` as a Cargo dependency:

```toml
[dependencies]
flow-like = { git = "https://github.com/Rheosoph/flow-like" }
```

### Visual Editor

The React workflow editor can be embedded as a component. Contact us for integration guidance.

### REST API

Execute workflows via the API:

```bash
POST /api/v1/apps/{app_id}/events/{event_id}/invoke
Content-Type: application/json

{
  "payload": { ... }
}
```

→ For engine embedding and white-label licensing, see [Enterprise White-Labeling](/enterprise/whitelabeling/).

---

## Configuration Files

| File | Purpose |
|------|---------|
| `flow-like.config.json` | Runtime configuration |
| `apps/desktop/src-tauri/tauri.conf.json` | Desktop app settings |
| `packages/ui/tailwind.config.ts` | Tailwind theme |
| `packages/ui/styles/globals.css` | Global CSS |

---

## Need Help?

:::tip[Looking for White-Label or Professional Services?]
We offer complete white-labeling with custom branding, deployment assistance, and priority support.

📧 **[info@great-co.de](mailto:info@great-co.de)**

→ [Enterprise White-Labeling](/enterprise/whitelabeling/)
:::

## Related

- [Enterprise White-Labeling](/enterprise/whitelabeling/) — Licensing and business options
- [Building from Source](/dev/build/) — Development setup
- [Architecture](/dev/architecture/) — Technical overview
- [Contribute](/dev/contribute/) — Submit customizations
