---
title: Customizing & White-Label
description: Current code locations for themes, branding, configuration, and UI customization
sidebar:
  order: 30
---

This page covers source-level customization. For licensing and supported
customer deployments, see [Enterprise White-Labeling](/enterprise/whitelabeling/).

## Choose the layer you need

| Layer | Current source of truth |
| --- | --- |
| Default design tokens | `packages/ui/global.css` |
| Profile theme schema and loader | `packages/ui/lib/theme.tsx` |
| Built-in profile themes | `apps/desktop/app/settings/profiles/themes/` |
| Desktop logo assets | `apps/desktop/public/` |
| Tauri product metadata and installer icons | `apps/desktop/src-tauri/tauri.conf.json` and `apps/desktop/src-tauri/icons/` |
| Hub name, domains, authentication, features, and legal links | `flow-like.config.json` |
| Shared React components | `packages/ui/components/` |

## Themes

Flow-Like uses semantic CSS variables with light and dark values. The current
default palette is expressed in OKLCH in `packages/ui/global.css`; it is not an
HSL-only theme.

Profile themes are JSON objects with `light` and `dark` maps:

```json
{
  "id": "My Theme",
  "light": {
    "background": "oklch(0.98 0.01 250)",
    "foreground": "oklch(0.20 0.02 250)",
    "primary": "oklch(0.62 0.18 260)",
    "primaryForeground": "oklch(1 0 0)"
  },
  "dark": {
    "background": "oklch(0.17 0.02 250)",
    "foreground": "oklch(0.94 0.01 250)",
    "primary": "oklch(0.72 0.15 260)",
    "primaryForeground": "oklch(0.17 0.02 250)"
  }
}
```

`loadTheme()` converts the camel-case keys to CSS variables and merges missing
values with the default theme. Built-in examples such as Cosmic Night,
Bubblegum, and Neo Brutalism live beside the other profile themes.

For a CSS-first workflow, edit
`apps/desktop/scripts/theme-input.css`, then run:

```bash
bun run --cwd apps/desktop ./scripts/generate-theme.ts
```

The generator writes `apps/desktop/scripts/generated-theme.json`. Review both
light and dark modes before promoting that output into the built-in theme
directory.

## Branding

### Runtime and in-app assets

The default in-app image is `apps/desktop/public/app-logo.webp`.
`app-logo-light.webp` and `app-logo.png` are also available for surfaces that
need those variants. Search for `/app-logo.webp` before replacing or renaming
it: many components use that path as a fallback.

The browser favicon is `apps/desktop/app/favicon.ico`.

### Native application identity

Update these fields in `apps/desktop/src-tauri/tauri.conf.json`:

```json
{
  "productName": "Your Product",
  "identifier": "com.example.your-product"
}
```

Replace the native files under `apps/desktop/src-tauri/icons/` and keep the
`bundle.icon` list synchronized. Platform-specific files under
`apps/desktop/src-tauri/configs/` can override the base identifier, so inspect
the configuration for every platform you ship.

`apps/desktop/package.json` controls the JavaScript workspace package name; it
does not contain a Tauri `productName` field.

## Hub configuration

`flow-like.config.json` configures the connected hub rather than the native
application bundle. Relevant fields include:

```json
{
  "name": "Your Flow-Like Hub",
  "domain": "api.example.com",
  "secure": true,
  "app": "app.example.com",
  "web": "www.example.com"
}
```

The same file also holds authentication, feature, contact, legal, sink, and
plan configuration. Start from the checked-in schema and configuration used by
your deployment; do not add an `appName` field and assume clients consume it.

## Components and editor surfaces

Shared shadcn-based components live in `packages/ui/components/ui/`. The Flow
editor is under `packages/ui/components/flow/`, while the Page and Widget
builders live under `packages/ui/components/builder/`.

Prefer semantic tokens such as `bg-background`, `text-foreground`, and
`border-border`. Hard-coded colors tend to break profile themes and dark mode.
Flow-Like uses Lucide and Tabler icons in different surfaces, so follow the
imports already used by the component you are editing.

## Validate a customization

```bash
mise run dev:desktop
mise run check
```

At minimum, review:

- onboarding, home, Flow editor, Page Builder, Widget Builder, and settings;
- both light and dark modes;
- common desktop window sizes and any mobile target you ship;
- Tauri bundle metadata, icons, deep links, and update configuration;
- hub login, logout, callbacks, legal links, and public URLs.

## Related

- [Enterprise White-Labeling](/enterprise/whitelabeling/)
- [Building from Source](/dev/build/)
- [Architecture](/dev/architecture/)
