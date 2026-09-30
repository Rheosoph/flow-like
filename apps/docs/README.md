# Flow-Like documentation

This Astro/Starlight site publishes the product and developer documentation at
[docs.flow-like.com](https://docs.flow-like.com/).

## Work locally

Install the toolchain using [Building from Source](src/content/docs/dev/build.md).
Run these commands from the repository root:

```sh
bun install
mise run dev:docs
mise run build:docs
```

The development server uses `http://localhost:4321`. The production build writes
to `apps/docs/dist` and runs the Markdown/`llms.txt` generator after Astro.
Running `astro build` directly skips that post-build step.

## Maintain content

- [Maintaining site content](src/content/docs/dev/documentation-assets.md)
  covers page placement, sidebar entries, builds, and Markdown responses.
- [Documentation screenshots](src/content/docs/dev/documentation-screenshots.md)
  covers fixtures, capture plans, and reviewing application images.
- Keep authored guides in `src/content/docs` and images in `src/assets`.
  Change generated node references through their source and generator.
- Preserve existing URLs and heading links when reorganizing a guide. Check
  both the rendered page and its generated Markdown after changing examples.

Task notes and validation results belong in the conversation or pull request.
Keep lasting contributor instructions in the linked documentation pages.
