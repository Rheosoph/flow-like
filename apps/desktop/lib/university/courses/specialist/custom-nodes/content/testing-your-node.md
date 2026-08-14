`mise run test` is green. Ship it? Not yet — and knowing exactly *why* not is what separates a package author from someone who uploads WASM files.

> **Predict first:** your unit suite passes on a mock host. Name one failure that suite is structurally incapable of catching.

## 1 · Test the contract first

The template ships Vitest and `MockHostBridge`. Your first tests shouldn't check the math — they should check the *promise*, because a perfectly computed result is worthless if the editor can't wire the node. This helper builds an execution context, then the tests hit both phases:

The template's `tests/node.test.ts` already imports `Context`, `ExecutionInput`, `LogLevel`, `MockHostBridge`, and `setHost` from the SDK, plus Vitest's `describe`/`expect`/`it` and your `getDefinition`/`run` — keep those imports and replace the rest:

```typescript
function makeContext(inputs: Record<string, unknown> = {}) {
  const host = new MockHostBridge();
  setHost(host);
  const input = ExecutionInput.fromDict({
    inputs,
    node_id: "test-node",
    run_id: "test-run",
    app_id: "test-app",
    board_id: "test-board",
    user_id: "test-user",
    stream_state: true,
    log_level: LogLevel.DEBUG,
    node_name: "normalize_text",
  });
  return { ctx: new Context(input, host), host };
}

describe("contract", () => {
  it("keeps the identifiers saved boards depend on", () => {
    const def = getDefinition();
    expect(def.name).toBe("normalize_text");
    const pinNames = def.pins.map((p) => p.name);
    expect(pinNames).toEqual(
      expect.arrayContaining(["exec", "text", "trim", "lowercase", "exec_out", "result", "changed"]),
    );
    expect(def.toDict().name).toBe("normalize_text");   // what get_nodes will export
  });
});
```

That `toDict` assertion is your rename tripwire: it pins down the serialized form Flow-Like extracts from the binary. After you publish 0.1.0, freeze these names in a fixture and make any removal or rename a deliberate, reviewed decision instead of an accident.

## 2 · Test the behavior

```typescript
describe("behavior", () => {
  it("normalizes messy support text", () => {
    const { ctx } = makeContext({ text: "   URGENT!!   My PRINTER Is On FIRE   " });
    const result = run(ctx);
    expect(result.outputs.result).toBe("urgent!!   my printer is on fire");
    expect(result.outputs.changed).toBe(true);
    expect(result.activateExec).toContain("exec_out");
  });

  it("reports changed = false for already-clean text", () => {
    const { ctx } = makeContext({ text: "printer is on fire" });
    const result = run(ctx);
    expect(result.outputs.changed).toBe(false);
  });
});
```

Note what these assert: actual output values *and* the activated execution pin — never just "it didn't throw". Round out the suite with the boundaries where normalizers actually break: empty input, whitespace-only input, each option toggled independently, and missing inputs falling back to defaults.

## 3 · What the mock host can't prove

@WasmSandbox

The infographic is the reality your tests approximate. Inside the sandbox box sits `node.wasm` beside its declared capabilities (in the example: NetworkHttp, StorageRead, Models — reviewed at install time). Declared calls pass green gates — an HTTP request through NetworkHttp, app storage through StorageRead — while the undeclared raw-socket attempt hits a red X and fails safely. Around it, the host guarantees memory and fuel limits, timeouts on every call, and typed pins in and out. As the footer puts it: a crash or an undeclared call ends the node — never the app, never your machine.

Here's the catch: `MockHostBridge` plays the host's role but never reads your permission list and never reproduces that Wasmtime enforcement. It answers whatever you script it to answer. So a green unit suite cannot prove that componentization succeeds (a dependency may refuse to bundle even though `npm test` passes — keep `mise run build` as its own gate), and it cannot prove real capability enforcement. For that, install the built package privately and run it in an actual board: confirm the catalog shows your definition, wire it between Incoming Support Request and the reply draft, run messy and clean inputs, check Runs and Logs, then save, close, and reopen the board to prove the identifiers survive persistence. Before any *update*, load an existing saved board against the new version — fresh boards can't reveal compatibility regressions, because they never stored the old names.

One more reviewer's fact: consent is keyed by package ID, not by version. Updating a package under the same ID may not prompt users again, so permission changes deserve human review even when the UI remembers trust.

## Recap

- Contract tests catch renames before saved boards do; behavior tests assert values *and* activated pins.
- The mock host proves logic — only the component build proves packaging, and only a real board proves discovery, enforcement, and persistence.
- Compatibility lives in *old* saved boards, not freshly created ones.
