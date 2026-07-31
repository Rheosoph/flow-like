---
title: Translating the Website
description: How to contribute translations to the Flow-Like website
sidebar:
  order: 45
---

Flow-Like's website supports 11 languages. This guide explains how to add or update translations.

## Supported Languages

| Code | Language   |
|------|------------|
| en   | English    |
| de   | Deutsch    |
| es   | Español    |
| fr   | Français   |
| zh   | 中文       |
| ja   | 日本語     |
| ko   | 한국어     |
| pt   | Português  |
| it   | Italiano   |
| nl   | Nederlands |
| sv   | Svenska    |

## File Structure

Translations are split into shared dictionaries, the current landing-page dictionary, and page-specific dictionaries:

| Path | Purpose |
|------|---------|
| `apps/website/src/i18n/index.ts` | Registers supported languages, merges dictionaries, and provides lookup helpers |
| `apps/website/src/i18n/locales/index.ts` | Exports the shared locale dictionaries |
| `apps/website/src/i18n/locales/<code>.ts` | Shared dictionary for one language; English is the fallback |
| `apps/website/src/i18n/locales/v5/<code>.ts` | Current landing-page dictionary for one language |
| `apps/website/src/i18n/locales/v5/index.ts` | Collects the current landing-page dictionaries |
| `apps/website/src/i18n/locales/pages/<page>-<code>.ts` | Page-specific dictionary for one language |
| `apps/website/src/i18n/locales/pages/<page>.ts` | Registers a page's dictionaries and English fallback |

The `v4/` directory contains legacy English content. Use the shared, `v5`, and page-specific locations above for current website work.

## Updating Existing Translations

1. Open the locale file for your language in `apps/website/src/i18n/locales/`
2. Find the key you want to update
3. Edit the translation value
4. Submit a pull request

### Example

To update the German hero headline:

```typescript
// apps/website/src/i18n/locales/de.ts
export const de = {
  // ...
  "hero.headline": "Wenn du es nicht sehen kannst, kannst du ihm nicht vertrauen.",
  // ...
} as const;
```

## Adding a New Language

1. Create a new locale file in `apps/website/src/i18n/locales/`
2. Export your translations following the English structure
3. Add the export to `locales/index.ts`
4. Register the language in `index.ts`

For complete coverage, also add the language to `locales/v5/index.ts` and provide any page-specific dictionaries used by the routes you intend to publish. Missing keys fall back to English, which is useful during development but should not be treated as a completed translation.

### Step 1: Create the Locale File

Create `apps/website/src/i18n/locales/[code].ts`:

```typescript
export const [code] = {
  // Meta
  "meta.title": "Flow-Like — [Your translation]",
  "meta.description": "[Your translation]",

  // Hero
  "hero.tagline": "[Your translation]",
  "hero.headline": "[Your translation]",
  // ... copy all keys from en.ts and translate values
} as const;
```

### Step 2: Export from Barrel File

Add your language to `apps/website/src/i18n/locales/index.ts`:

```typescript
export { en } from "./en";
export { de } from "./de";
// ...existing exports
export { [code] } from "./[code]";
```

### Step 3: Register the Language

Update `apps/website/src/i18n/index.ts`:

```typescript
import { en, de, ..., [code] } from "./locales";

export const languages = {
  en: "English",
  de: "Deutsch",
  // ...existing languages
  [code]: "[Native Name]",
};

export const translations = {
  en,
  de,
  // ...existing translations
  [code],
} as const;
```

## Translation Keys

English (`en.ts`) is the base language. All translation keys should match the keys in the English file. The main categories are:

| Category | Description |
|----------|-------------|
| `meta.*` | Page metadata (title, description) |
| `hero.*` | Hero section content |
| `problem.*` | Problem section content |
| `solution.*` | Solution section content |
| `stack.*` | Enterprise stack section |
| `services.*` | Services section |
| `audience.*` | Target audience section |
| `cta.*` | Call to action buttons |
| `faq.*` | Frequently asked questions |
| `nav.*` | Navigation items |
| `footer.*` | Footer content |

## Best Practices

1. **Preserve formatting**: Keep line breaks, emphasis markers, and spacing as in the original
2. **Don't translate placeholders**: Keep technical terms, brand names, and variable placeholders unchanged
3. **Match the tone**: Maintain the professional yet approachable tone of the original
4. **Test locally**: Run the website locally to verify your translations render correctly
5. **Use native expressions**: Prefer natural phrasing over literal translations

## Testing Locally

```bash
mise run dev:website
```

Then visit `http://localhost:4321/[lang]/` to preview your translations.

## Need Help?

Open an issue on GitHub or reach out on our Discord if you have questions about translating content.
