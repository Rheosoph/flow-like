Enable **Developer Mode** in Desktop Settings. In **Packages → Mine**, choose **New package** and the TypeScript template, or **Add folder** for a project copied from `templates/wasm-node-typescript` in a Flow-Like checkout.

1. Work in your own project folder. Set a package ID you control, a descriptive name and version `0.1.0` in `flow-like.toml`. Keep `wasm_path = "build/node.wasm"` and the template resource tiers until runtime testing justifies a change.
2. Replace `src/node.ts` with the supplied `node.ts` and `tests/node.test.ts` with the supplied test file. This removes the example streaming behavior and its capability from the definition.
3. In the project directory run:

```text
npm install
npm test
npm run build
```

The template's `mise run setup`, `mise run test` and `mise run build` provide the corresponding workflow if you use mise.

4. Confirm `build/node.wasm` was produced by the current source.
5. In **Packages → Library**, choose **Load local .wasm** and select that artifact. Review the displayed definition and permissions. Use the local package in a scratch app; add/select its package version in the app's Packages view if required by that app.
6. Search the node catalog for **Normalize Text**, place it between a Simple Event and Print Info, and connect result to Message. Run `  Mixed CASE  ` and expect `mixed case`.

Keep the fixture token-free. A WASM binary is distributable code, so credentials embedded in it are not hidden from recipients.
