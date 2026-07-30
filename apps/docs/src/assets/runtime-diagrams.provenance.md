# Runtime diagram provenance

These diagrams are deterministic SVG compositions created for the Flow-Like developer documentation. They contain no generated product UI or third-party imagery.

The sink relationships were verified against:

- `apps/desktop/src-tauri/src/event_sink/`
- `apps/backend/docker-compose/sink-services/src/`
- `packages/api/src/routes/sink/`
- `packages/sinks/src/scheduler/`

The WASM relationships were verified against:

- `packages/wasm/src/unified.rs`
- `packages/wasm/src/abi.rs`
- `packages/wasm/src/component/`
- `packages/wasm/src/limits.rs`
- `packages/wasm/wit/flow-like-node.wit`
- `templates/wasm-capability-matrix.md`

The palette and typography follow the existing Flow-Like documentation artwork: a dark neutral surface, coral-orange primary accent, and restrained cyan, green, violet, and amber secondary accents.
