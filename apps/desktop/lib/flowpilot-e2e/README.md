# FlowPilot app-creation E2E

The primary entry point is the repository-root CLI. It starts the real development Tauri runtime,
loads the existing webview runner, receives its result through a nonce-protected loopback callback,
writes a complete JSON artifact, and exits nonzero when generation or validation fails:

```sh
bun run flowpilot:e2e -- --case simple-agent
bun run flowpilot:e2e -- --suite smoke --min-chars 1200 --json
bun run flowpilot:e2e -- --case forum --case ops-dashboard --repeat 3 --fail-fast
```

Useful inspection modes do not start Tauri or spend model budget:

```sh
bun run flowpilot:e2e -- --list
bun run flowpilot:e2e -- --case simple-agent --dry-run
```

By default Tauri starts the normal desktop Next dev server at `http://localhost:3000`. For a faster
edit/run loop, keep that Next server running and pass `--frontend-url http://localhost:3000` so the
CLI reuses it. Close any running Flow Like desktop app first: the native single-instance guard
rejects a second desktop process rather than risking concurrent writes to shared local app data,
and a CLI lock rejects parallel benchmark commands. `--keep-desktop` is available for debugging,
but that retained app must be closed before the next CLI run.

Child-process logs go to stderr, so `--json` reserves stdout for exactly one machine-readable result,
including on infrastructure errors. Use `--output /tmp/flowpilot-e2e.json` for a stable artifact
path. Exit code `0` means every requested run passed, `1` means at least one completed benchmark
failed, and `2` means CLI/startup/transport failure. The controller independently verifies the
selection and artifact order and recomputes the final summary instead of trusting the webview's
pass bit.

This is intentionally a thin controller, not a second FlowPilot implementation. Codex, GitHub
Copilot, Claude Code, and Bits still execute through the shared desktop global-chat and
`GlobalToolBridge` path; the benchmark policy pins Codex Terra/high.

The live benchmark is available in a **development** desktop build at
`/developer/flowpilot-e2e`. It drives the real global-chat and frontend-tool bridge, pins Codex to
`gpt-5.6-terra` with `high` reasoning for the parent turn and every nested specialist, and keeps
every generated app for inspection. Production builds fail the runner preflight before spending
model budget because detailed compiler evidence is intentionally disabled there.

Quick entry points:

- One case: `/developer/flowpilot-e2e?case=simple-agent&run=1`
- Default three-case smoke suite: `/developer/flowpilot-e2e?suite=smoke&run=1`
- All six cases: `/developer/flowpilot-e2e?suite=full&run=1`
- Override the per-case non-whitespace character floor: append `&minChars=1200`

The same runner is callable from the desktop webview console:

```js
await window.flowPilotE2E.run({
  caseId: "forum",
  minFlowScriptNonWhitespaceChars: 900,
});
```

Each returned/downloadable artifact includes the resolved prompt; byte-for-byte authored
FlowScript candidates; exact `check_flowscript` and commit receipts per app/board; canonical board
readbacks; parser and authoritative reconciliation results; app/UI/data/event inventory; persisted
node-capability and generated-ID checks; lower/upper compactness bounds; partial collector failures;
a stable failure fingerprint for grouping regressions; and the assistant debug trace. Live runs
spend model budget and create apps; the focused Vitest suite only tests case construction and
artifact evaluation.
