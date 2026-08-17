# Astro Starter Kit: Basics

```sh
npm create astro@latest -- --template basics
```

[![Open in StackBlitz](https://developer.stackblitz.com/img/open_in_stackblitz.svg)](https://stackblitz.com/github/withastro/astro/tree/latest/examples/basics)
[![Open with CodeSandbox](https://assets.codesandbox.io/github/button-edit-lime.svg)](https://codesandbox.io/p/sandbox/github/withastro/astro/tree/latest/examples/basics)
[![Open in GitHub Codespaces](https://github.com/codespaces/badge.svg)](https://codespaces.new/withastro/astro?devcontainer_path=.devcontainer/basics/devcontainer.json)

> 🧑‍🚀 **Seasoned astronaut?** Delete this file. Have fun!

![just-the-basics](https://github.com/withastro/astro/assets/2244813/a0a5533c-a856-4198-8470-2d67b1d7c554)

## 🚀 Project Structure

Inside of your Astro project, you'll see the following folders and files:

```text
/
├── public/
│   └── favicon.svg
├── src/
│   ├── layouts/
│   │   └── Layout.astro
│   └── pages/
│       └── index.astro
└── package.json
```

To learn more about the folder structure of an Astro project, refer to [our guide on project structure](https://docs.astro.build/en/basics/project-structure/).

## 🧞 Commands

All commands are run from the root of the project, from a terminal:

| Command                   | Action                                           |
| :------------------------ | :----------------------------------------------- |
| `npm install`             | Installs dependencies                            |
| `npm run dev`             | Starts local dev server at `localhost:4321`      |
| `npm run build`           | Build your production site to `./dist/`          |
| `npm run preview`         | Preview your build locally, before deploying     |
| `npm run astro ...`       | Run CLI commands like `astro add`, `astro check` |
| `npm run astro -- --help` | Get help using the Astro CLI                     |

## 🤖 Markdown for agents

Every prerendered page answers `Accept: text/markdown` with a Markdown
representation; HTML stays the default for browsers. Appending `.md` to a page
URL works too (`/pricing.md`), which is handy for humans checking what an agent
sees.

| Piece | File |
| :--- | :--- |
| HTML → Markdown conversion | `scripts/agent-markdown/html-to-markdown.mjs` |
| Build step writing the twins | `scripts/agent-markdown/generate.mjs` |
| Accept-header negotiation | `scripts/agent-markdown/markdown-negotiation.mjs` |

`astro build` writes `<page>.md` next to every `<page>.html` in `dist/client`,
and `scripts/prepare-workers-sites-deploy.mjs` copies the negotiation module
into the Worker entry so it can serve that twin before Astro handles the
request. Responses carry `Content-Type: text/markdown; charset=utf-8`, an
`x-markdown-tokens` estimate, `Vary: Accept`, and a canonical `Link` header.

The on-demand `/store/**` routes have no prebuilt twin and keep serving HTML.

`scripts/agent-markdown/html-to-markdown.mjs` is mirrored in `apps/docs` — keep
both copies in sync.

```sh
bun run build
node ./scripts/prepare-workers-sites-deploy.mjs
bunx wrangler dev --port 8798
curl -sD- -o- http://127.0.0.1:8798/pricing/ -H 'Accept: text/markdown' | head
```

## 👀 Want to learn more?

Feel free to check [our documentation](https://docs.astro.build) or jump into our [Discord server](https://astro.build/chat).
