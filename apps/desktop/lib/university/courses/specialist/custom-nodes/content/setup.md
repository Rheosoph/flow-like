Three commands stand between an empty folder and a WASM binary Flow-Like can load. In this lab you'll run all three, then start turning the template's example node into Acme's `normalize_text`. By the end you'll have a green build with your name on it — literally, it's in the manifest.

## 1 · Unlock the developer surfaces

Package authoring lives behind Developer Mode. Open **Settings** in Flow-Like Desktop and scroll to the **Developer** section at the bottom.

@DeveloperModeSettings

The screenshot shows the Settings page — Personalization, AI & Models, and System cards up top, and at the bottom the **Developer Mode** card with its toggle: "Enable developer mode — Unhides flows, events, data tooling, and package registries. Synced across your devices." Flip it on. **Library → Packages** becomes available, which is where you'll later inspect and publish your work. Developer Mode reveals authoring tools; it doesn't bypass registry permissions, package review, or runtime capability checks.

## 2 · Claim your identity

Copy `templates/wasm-node-typescript` to a fresh directory *outside* the template tree, and initialize git before you touch anything — you'll want generated changes distinguishable from your edits. Then open `flow-like.toml` and make the package yours:

1. Set `id = "com.acme.text-tools"` — or any reverse-domain identifier you actually control. This ID is permanent once published, so pick it like a domain name, not a variable name.
2. Set `version = "0.1.0"`, an honest `description`, your `license`, and keep `wasm_path = "build/node.wasm"`.
3. Keep `memory = "minimal"` and `timeout = "quick"` — a pure text node needs nothing more.

## 3 · Three commands

```text
mise run setup
mise run test
mise run build
```

(Not a mise user? `npm install`, `npm test`, `npm run build` do the same jobs.) The template uses standard TypeScript, `esbuild`, and Bytecode Alliance `componentize-js` — no hand-written ABI glue. When the build finishes, look for `build/node.wasm`, newer than your sources.

That file is a complete Flow-Like extension. Ten minutes in, and you've already produced something the runtime can load. Everything from here is making it *yours*.

## 4 · Your turn: complete the node

Last lesson you read the finished `normalize_text`. Now build it from memory. Replace the template's example definition with the one from lesson 2 — internal name `normalize_text`, pins `exec`, `text`, `trim`, `lowercase` in; `exec_out`, `result`, `changed` out — then complete this `run` skeleton without peeking:

```typescript
export function run(ctx: Context): ExecutionResult {
  const text = ctx.getString("text", "") ?? "";
  const trim = ctx.getBool("trim", true) ?? true;
  const lowercase = ctx.getBool("lowercase", true) ?? true;

  let result = text;
  // TODO 1: if trim is enabled, strip surrounding whitespace
  // TODO 2: if lowercase is enabled, lowercase the result

  ctx.setOutput("result", result);
  ctx.setOutput("changed", /* TODO 3: did the text actually change? */);

  // TODO 4: one line is still missing before the return — which?
  return ctx.success();
}
```

The template's example node streams progress and therefore declares the `streaming` permission — `normalize_text` does neither, so remove both the call and the declaration. Update `tests/node.test.ts` to match your new names in the same change (the shipped tests still assert the example node), then re-run `mise run test` and `mise run build`. Green again? That's the loop you'll live in: tests catch logic mistakes, the build catches packaging mistakes — they are separate gates, and one passing says nothing about the other.

> **Watch out:** never commit credentials, tokens, or production sample data into the package. WASM isolation protects the host from the node — it does not protect secrets baked into the binary from anyone who downloads it.

## Recap

- Developer Mode unhides the package surfaces; it grants no extra runtime rights.
- Package ID, node name, and pin names are chosen now, before publishing — while renames are still free.
- `mise run test` and `mise run build` guard different failure classes; run both every loop.
