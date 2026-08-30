# Incident Triage book fixture

This is the deterministic six-node Flow used in Chapter 4 of FlowBook. It has no account,
model, credential, network call, or external dependency.

The source is kept canonical by `packages/ast/tests/book_examples.rs`. Its catalog mappings
were checked against the generated declarations in `packages/ast/flow.d`:

| Source construct | Visual node |
| --- | --- |
| `eventsGeneric triageIncident(payload: Struct, report: string)` | Generic Event |
| `report.trim()` | Trim String |
| `normalized.contains(...)` | Contains |
| `if … else` | Branch |
| `error(...)` imported by `use log::*` | Error Log |
| `info(...)` imported by `use log::*` | Info Log |

Use this invocation payload to take the escalation arm:

```json
{
  "report": "  Production is on hold  "
}
```

The `payload: Struct` parameter is the Generic Event's built-in payload output. It remains
unused in this exercise; `report` is the named typed output we add.

The parser round-trip test proves syntax and text-level canonical rendering. Current lowering
tests also establish the built-in payload parameter and the derived `use log::*` import, while
direct reconciliation and runtime tests cover each constituent construct. A combined
end-to-end UI fixture and release-matched screenshots are still required before Chapter 4 is
marked final.
