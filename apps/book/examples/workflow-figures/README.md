# Workflow figure fixtures

These FlowScripts support the real Studio screenshots used where a chapter needs a smaller,
purpose-built topology than its complete example. Generate committed images with the root
`workflow:screenshot` command and keep them in `apps/book/src/assets/workflows`.

- `sequence.flow` — an event followed by three named callable steps.
- `parallel.flow` — one source-authored task pin and Parallel Execution's separate Done continuation.
- `structure.flow` — three callable boundaries at the top level and an inspectable function body.
- `typed-collections.flow` — a typed Incident array crossing a sequential For Each boundary.
- `break-struct.flow` — explicit schema-driven Break Struct field pins.
- `cache-invalidation.flow` — durable update, revision, invalidation, and publication order.
