Here is the entire `normalize_text` node — every line of it. Before you read the annotations, make a prediction.

> **Predict first:** you could delete one of the two functions below and your node would *still appear in the catalog*, pins and all. Which one?

## 1 · The definition

The TypeScript template puts your node in `src/node.ts`. The file starts with an import of `NodeDefinition`, `PinDefinition`, `PinType`, `Context`, and `ExecutionResult` from the Flow-Like TypeScript WASM SDK — the template ships that line, so keep it as-is. First half — the contract:

```typescript
export function getDefinition(): NodeDefinition {
  const node = new NodeDefinition(
    "normalize_text",                       // internal name — boards store this forever
    "Normalize Text",                       // friendly label — polish it anytime
    "Trims and lowercases incoming text",
    "Custom/Text",                          // category shown in the catalog search
  );

  node.addPin(PinDefinition.inputExec("exec"));
  node.addPin(PinDefinition.inputPin("text", PinType.STRING, { defaultValue: "" }));
  node.addPin(PinDefinition.inputPin("trim", PinType.BOOL, { defaultValue: true }));
  node.addPin(PinDefinition.inputPin("lowercase", PinType.BOOL, { defaultValue: true }));

  node.addPin(PinDefinition.outputExec("exec_out"));
  node.addPin(PinDefinition.outputPin("result", PinType.STRING));
  node.addPin(PinDefinition.outputPin("changed", PinType.BOOL));

  return node;
}
```

Read it as a promise to the editor: "I'm called `normalize_text`, I take text plus two boolean options with sensible defaults, and I hand back a result and a changed flag." `PinType` also offers `I64`, `F64`, `GENERIC` (JSON), and `BYTES` — use the most accurate type you can, because `GENERIC` everywhere throws away editor validation. Notice what's *absent*: no `addPermission` call. Trimming a string needs no protected host service. A node that makes HTTP calls would add `node.addPermission("network:http")`; storage writers add `"storage:write"`; other labels include `variables`, `cache`, `streaming`, `models`, `a2ui`, `oauth`, and `functions`.

## 2 · The run function

Second half — the behavior:

```typescript
export function run(ctx: Context): ExecutionResult {
  const text = ctx.getString("text", "") ?? "";
  const trim = ctx.getBool("trim", true) ?? true;
  const lowercase = ctx.getBool("lowercase", true) ?? true;

  let result = text;
  if (trim) result = result.trim();
  if (lowercase) result = result.toLowerCase();

  ctx.setOutput("result", result);
  ctx.setOutput("changed", result !== text);
  ctx.activateExec("exec_out");   // skip this and downstream nodes never run
  return ctx.success();
}
```

Four beats, always in this rhythm: read typed inputs with defaults, compute, write outputs, activate the execution path and return. The `activateExec` line deserves respect — a run that computes a perfect `result` but never activates `exec_out` leaves the support board silently stuck, because execution flow follows activated pins, not produced values.

And your prediction? You could delete the *body of `run`* and the catalog wouldn't notice. Discovery reads the definitions the compiled binary exports through `get_nodes`; execution is a separate phase. The definition is the node's public face.

## 3 · Where each layer lives

The template `templates/wasm-node-typescript` is small: `flow-like.toml` (package identity and limits), `src/node.ts` (what you just read), `src/app.ts` (the generated WIT bridge — don't edit), `tests/node.test.ts` (mock-host tests), `build.mjs` (bundle + componentize), and `build/node.wasm` once you've built.

Permissions and limits split cleanly across two layers. Protected capabilities go **on the node definition**, in code, as you saw above. Package-wide resource tiers go **in the manifest**:

```toml
id = "com.acme.text-tools"
name = "Acme Text Tools"
version = "0.1.0"
wasm_path = "build/node.wasm"

[permissions]
memory = "minimal"   # 16 MB — plenty for trimming strings
timeout = "quick"    # 5 seconds
```

Pick the smallest tier that works; a higher tier raises the ceiling, it doesn't reserve resources.

> **Legacy note (read once, then forget):** older example manifests carry `[[nodes]]` tables and network fields like `allowed_hosts`. The current typed manifest ignores unknown `[[nodes]]` tables — they register nothing — and package-level host lists are not merged into per-node runtime enforcement. If you copy an old manifest, delete those sections. Nodes are declared in code, capabilities are declared in code; the manifest identifies and bounds the package.

## Recap

- `getDefinition` is the contract the editor renders; `run` is the behavior — and only the compiled binary's exported definitions populate the catalog.
- Capabilities live on the node definition; memory and timeout tiers live in `flow-like.toml`.
- Compute-then-activate: outputs without `activateExec` stop the board.
