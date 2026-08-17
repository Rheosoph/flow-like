Friday morning. The support team's board has a placeholder where `normalize_text` should be, and they'd like it gone today. Your package builds, your tests pass, and this lesson is release day — first the ship procedure, then a release review that doubles as your final assessment.

## Release day

Work from a fresh checkout so nothing local leaks into the artifact: `mise run test`, `mise run build`, and confirm `flow-like.toml` still points at the newly produced `build/node.wasm`.

Then, in Flow-Like Desktop, open **Library → Packages → Publish**. The wizard walks you through it: hand it the WASM binary and the manifest, review the package identity and descriptive metadata, the resource tiers, and finally the node definitions the backend extracts from your binary — the same `normalize_text` contract your tests froze. Behind the scenes the backend validates the WebAssembly artifact, hashes it, and refuses any upload that reuses an already-published package ID and version pair.

Your freshly published 0.1.0 starts out private. You activate it for the support team, they drop `normalize_text` between the incoming request and the reply draft, and the placeholder is gone before lunch. Making a package public is a different road — a publication request that goes through review and comes back approved, rejected, or with change requests. Some packages in the registry also carry an administrator-set `verified` badge.

## Monday: three things on your desk

The release went well. Too well — now everyone wants something, and three artifacts are waiting for your judgment.

**Artifact 1 — the 0.2.0 proposal.** A teammate's diff for the next version:

```diff
- node.addPin(PinDefinition.outputPin("result", PinType.STRING));
+ node.addPin(PinDefinition.outputPin("output", PinType.STRING));   // clearer name
+ node.addPin(PinDefinition.inputPin("strip_punctuation", PinType.BOOL, { defaultValue: false }));
```

Commit message: "Rename result → output for clarity, add punctuation stripping. Bumped version to 0.2.0, all tests updated and green."

**Artifact 2 — the `fetch_greeting` pull request.** A second node for the package. Its `run` calls the HTTP host service to fetch a template greeting from an internal Acme server, but `getDefinition` contains no `addPermission` call at all. The author's note: "MockHostBridge suite passes, including the HTTP path — bridge responds with the fixture greeting."

**Artifact 3 — the evidence table.** Attached to the 0.2.0 release ticket:

| Check | Status |
| --- | --- |
| Unit tests (contract + behavior + boundaries) | green |
| Component build from reviewed source | green |
| Run in a brand-new scratch board | green |

**Artifact 4 — the Friday-afternoon bug.** Ten minutes after 0.1.0 went live, QA finds that `changed` reports `true` for text that only differs by a trailing newline in one edge case. Small fix, one line. A teammate suggests: "Just re-upload the corrected binary as 0.1.0 — nobody's installed it yet except us."

The questions below are your release review. Clear them and ship.
