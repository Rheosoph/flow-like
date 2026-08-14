# Starlight Starter Kit: Basics

[![Built with Starlight](https://astro.badg.es/v2/built-with-starlight/tiny.svg)](https://starlight.astro.build)

```
npm create astro@latest -- --template starlight
```

[![Open in StackBlitz](https://developer.stackblitz.com/img/open_in_stackblitz.svg)](https://stackblitz.com/github/withastro/starlight/tree/main/examples/basics)
[![Open with CodeSandbox](https://assets.codesandbox.io/github/button-edit-lime.svg)](https://codesandbox.io/p/sandbox/github/withastro/starlight/tree/main/examples/basics)
[![Deploy to Netlify](https://www.netlify.com/img/deploy/button.svg)](https://app.netlify.com/start/deploy?repository=https://github.com/withastro/starlight&create_from_path=examples/basics)
[![Deploy with Vercel](https://vercel.com/button)](https://vercel.com/new/clone?repository-url=https%3A%2F%2Fgithub.com%2Fwithastro%2Fstarlight%2Ftree%2Fmain%2Fexamples%2Fbasics&project-name=my-starlight-docs&repository-name=my-starlight-docs)

> 🧑‍🚀 **Seasoned astronaut?** Delete this file. Have fun!

## 🚀 Project Structure

Inside of your Astro + Starlight project, you'll see the following folders and files:

```
.
├── public/
├── src/
│   ├── assets/
│   ├── content/
│   │   ├── docs/
│   └── content.config.ts
├── astro.config.mjs
├── package.json
└── tsconfig.json
```

Starlight looks for `.md` or `.mdx` files in the `src/content/docs/` directory. Each file is exposed as a route based on its file name.

Images can be added to `src/assets/` and embedded in Markdown with a relative link.

Static assets, like favicons, can be placed in the `public/` directory.

## 🧞 Commands

All commands are run from the repository root using [mise](https://mise.jdx.dev):

| Command                   | Action                                           |
| :------------------------ | :----------------------------------------------- |
| `bun install`             | Installs dependencies                            |
| `mise run dev:docs`       | Starts local dev server at `localhost:4321`      |
| `mise run build:docs`     | Build your production site to `./dist/`          |
| `mise run deploy:docs`    | Deploy docs                                      |

## 🤖 Markdown for agents

Every page answers `Accept: text/markdown` with a Markdown representation; HTML
stays the default for browsers. Appending `.md` to any page URL works too
(`/apps/pages.md`), which is handy for humans checking what an agent sees.

| Piece | File |
| :--- | :--- |
| HTML → Markdown conversion | `scripts/agent-markdown/html-to-markdown.mjs` |
| Build step writing the twins | `scripts/agent-markdown/generate.mjs` |
| Accept-header negotiation | `functions/_middleware.ts` |

`astro build` writes `<page>.md` next to every `<page>.html`, and the Cloudflare
Pages Function serves that twin when the request asks for Markdown. Responses
carry `Content-Type: text/markdown; charset=utf-8`, an `x-markdown-tokens`
estimate, `Vary: Accept`, and a canonical `Link` header.

`scripts/agent-markdown/html-to-markdown.mjs` is mirrored in `apps/website` —
keep both copies in sync.

```sh
bun run build
bunx wrangler pages dev --port 8799
curl -sD- -o- http://127.0.0.1:8799/dev/rust/ -H 'Accept: text/markdown' | head
```

## 👀 Want to learn more?

Check out [Starlight’s docs](https://starlight.astro.build/), read [the Astro documentation](https://docs.astro.build), or jump into the [Astro Discord server](https://astro.build/chat).
