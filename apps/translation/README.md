# Translation Studio

A UI for the JSON files in `packages/locales/locales`. It reads them, shows you
what is missing or broken, and writes them back. There is no API, no database
and no build step — the files on disk are the entire state.

```bash
mise run i18n:studio     # http://localhost:5177
```

## The two views

**Coverage** — every namespace against every target locale, computed live from
the files. Cells are clickable and land you in the workbench with that
namespace and language already selected. "Add language" creates the folder,
seeds one empty file per namespace and updates `config.json`.

**Workbench** — the key-by-key editor. Namespace rail on the left with per
-namespace progress, the source/target list in the middle, and an inspector on
the right. `↑`/`↓` move, `⌘S` saves every file you touched.

## What it flags

Statuses are derived from the JSON alone — nothing is tracked in a side table,
so the studio can never disagree with the files:

| Status | Meaning |
| --- | --- |
| **Missing** | empty or absent in the target |
| **Placeholder lost** | the source has `{{name}}`, `$t(ref)`, or a numeric React Trans tag such as `<0>…</0>` and the translation drops it — runtime values or components can disappear |
| **Same as source** | identical string; correct for product names, otherwise an untranslated string that reads as done |
| **Not in source** | the key was removed from English but is still sitting in this locale |
| **Translated** | present and different |

The inspector also greps the working tree for each key's call sites, so you can
see the component a string renders in before you write it. That search runs on
request against the real files — there is no index to go stale.

## Why the dev server writes the files

The browser cannot write to the repository, and standing up a service for a
local tool would be worse than the problem. The Vite dev server that already
serves the app carries a small middleware
(`server/locales-plugin.ts`) with four routes, and it refuses to touch anything
outside `packages/locales/locales`. `vite preview` mounts the same middleware,
so a production build of the studio behaves identically.

## Relationship to the CLI

The studio and `i18next-cli` do different halves of the job and share the same
files:

- `mise run i18n:extract` discovers keys from `t()` calls and prunes dead ones
- the studio is where a human writes and reviews the values

Run extract after adding new `t()` calls, then open the studio to fill the gaps.
