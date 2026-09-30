Use **Auto mode off** for this exercise. Ask:

```text
Change only Print Info's Message to "Inspection complete".
Keep its execution connection and every other node unchanged.
```

Inspect the proposed change and the FlowScript diagnostics. The Message value should change; the event and execution wire should remain. Apply the review, run the event, and confirm `Inspection complete` in its log.

**Approval behavior depends on mode.** With Auto off, generated editor changes stay in a review step and side-effecting tools ask before acting. With Auto on, tools and completed reviews can apply without a per-step prompt. Deleting existing board items still requires explicit confirmation. Read the current mode before relying on a review pause.

If you edit the board while a review is pending, it can become stale. Refresh the request against the current board instead of applying an obsolete change.

Compilation checks whether the generated edit is well formed. It does not prove that the edit meets your requirement. The run result completes this check.
