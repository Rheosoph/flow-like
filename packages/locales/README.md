# @flow-like/locales

Every user-facing string in the desktop app, the web app and `packages/ui`
comes from here. One package, one set of JSON files, both apps.

## Layout

```
locales/
  config.json          source language, namespaces, enabled languages
  en/                  source language — bundled into the app
    common.json        shared chrome: dialogs, buttons, feedback
    nav.json           sidebar and route labels
    settings.json      settings, account, theme
    flow.json          flow editor and boards
    store.json         store and library
  de/                  same namespaces, loaded on demand
src/
  config.ts            reads config.json
  languages.ts         Intl-backed names, RTL detection
  resources.ts         static import of the source language
  create-i18n.ts       the i18next instance
  provider.tsx         <I18nProvider> and useLanguage()
  types.ts             makes t() type-check against the English files
```

`config.json` is the single source of truth. The runtime, `i18next.config.ts`
and the translation studio all read it, so adding a language is a JSON edit —
no code change.

## Using it

```tsx
import { useTranslation } from "@flow-like/locales";

function BugReportButton() {
  const { t } = useTranslation(); // defaults to the `common` namespace
  return <button>{t("feedback.trigger")}</button>;
}
```

Other namespaces, either way round:

```tsx
const { t } = useTranslation("settings");
t("theme.light");

const { t } = useTranslation(["common", "settings"]);
t("feedback.trigger");
t("settings:theme.light");
```

Keys are type-checked against `locales/en/*.json`, so a typo is a build error
rather than a string that renders as `settings:theme.ligth`.

## How loading works

The source language is imported statically and ships in the main bundle: the
first paint never shows raw keys, and a missing translation always has
something to fall back to. Every other language is a dynamic import, which the
bundler splits into one chunk per file — a language costs nothing until someone
selects it.

Language detection order is querystring (`?lng=de`), then `localStorage`
(`flow-like:language`), then the browser. `de-AT` resolves to `de`.

## Workflow

```bash
mise run i18n:studio     # edit translations in a UI
mise run i18n:extract    # pull new t() keys out of the source into every locale
mise run i18n:status     # coverage per locale
mise run i18n:sync       # align secondary locales with the source key set
mise run i18n:lint       # find hardcoded strings that should go through t()
```

`extract` scans `apps/desktop`, `apps/web` and `packages/ui`, adds new keys to
English with the literal as its value, and adds them to every other language
**empty** — an English string sitting in a German file reads as done and never
gets looked at again.

It also deletes keys it cannot find a call site for. `nav.*` is exempt via
`preservePatterns` in `i18next.config.ts`: those keys were migrated from the
old `apps/desktop/public/locales` and are already translated, but the sidebar
builds its labels from a module-level const that cannot call `t()` yet. Drop
the exemption once those call sites are converted.

## Adding a language

Through the studio (`mise run i18n:studio` → Coverage → Add language), or by
hand: create `locales/<code>/` with one `{}` file per namespace and add the
code to `config.json`.

## Adding a namespace

Namespaces are referenced from code, so this one is not a pure JSON change:

1. add the name to `config.json`
2. create `locales/<lang>/<name>.json` for every language
3. add the import to `src/resources.ts`
