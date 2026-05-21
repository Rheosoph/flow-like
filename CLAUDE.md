# CLAUDE.md — flow-like

Flow-Like is a visual flow/automation platform with a Rust backend (Tauri desktop + multi-deployment API) and a Next.js/React frontend. It lets users build node-based flow programs, execute them locally or in the cloud, and extend them via WASM nodes.

## Repository Layout

```
packages/core          — Core Rust runtime (flow execution engine)
packages/catalog/      — Built-in nodes grouped by domain (std, data, web, llm, …)
packages/api           — Shared API types and middleware
packages/types         — Shared Rust types (flow-like-types)
packages/executor      — Execution engine
packages/compiler      — Flow compiler
packages/wasm          — WASM sandbox runtime
apps/desktop/          — Tauri + Next.js desktop app
apps/backend/          — Multi-target backends (local, AWS, Kubernetes, Docker)
libs/                  — Additional libraries
templates/             — WASM node templates (Rust, Python, TS, Go, …)
```

---

## Build & Dev Commands

### Rust
```bash
# Fast incremental check for a single package (prefer this over full build)
cargo check -p <package-name>

# Catalog nodes require the execute feature for heavy-dep nodes
cargo check -p flow-like-catalog-data --features execute

# Full workspace check
cargo check

# Run tests
cargo test -p <package-name>
```

### Frontend (Bun + Biome)
```bash
# Lint / format
bunx biome check .
bunx biome format --write .

# Desktop dev
cd apps/desktop && bun run dev          # Next.js only
cd apps/desktop && bun run dev:all      # Tauri + Next.js

# Type-check
cd apps/desktop && bunx tsc --noEmit
```

### Mise tasks (preferred for full workflows)
```bash
mise run dev:desktop          # Auto-detects platform
```

---

## Critical Rules

### Git
- `git diff` is fine. **NEVER run git stash, reset, or any other destructive/state-mutating git command** without explicit user confirmation. The user is actively working in the same repo.

### Code Style
- Edit files directly using file tools — don't generate scripts to perform edits.
- No diff-mode snippets — always return complete drop-in replacements.
- No self-explanatory comments. Keep comments minimal and precise.
- DRY: extract reusable functions/components; avoid large logic blocks.
- Prefer parallel subagent work for multi-step tasks; track progress in `todo/*.md`.

---

## Rust Guidelines

- After editing, run `cargo check -p <package>` (not a full build) to verify.
- After finishing a task, run a final `cargo check` to catch any cross-package issues.
- No unnecessary comments. Rust code should be self-documenting.

### Feature Gating (Heavy Dependencies)
Nodes with heavy deps (ML libs, PDF parsers, HTTP clients, etc.) **must** gate execution behind `--features execute`:

```toml
# Cargo.toml
[features]
execute = ["dep:heavy-crate"]

[dependencies]
heavy-crate = { workspace = true, optional = true }
```

```rust
#[cfg(feature = "execute")]
async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> { … }

#[cfg(not(feature = "execute"))]
async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
    Err(flow_like_types::anyhow!("This node requires the 'execute' feature"))
}
```

`get_node()` and `on_update()` are **never** gated — they provide metadata.

---

## Node Creation (packages/catalog/)

### Pure vs Impure
- **Pure**: no side effects, deterministic → no execution pins.
- **Impure**: side effects or non-deterministic → requires execution pins (`in`/`out`).

### Run Function Pattern
1. Deactivate outgoing execution pins first (stops graph on failure).
2. Execute logic.
3. Activate outgoing execution pins and write pin values.

### Pin Naming Rules
Input and output pins that carry the same value **must have different `name` values**:
```rust
// WRONG — names collide in context.get/set_pin_by_name()
node.add_input_pin("log", "Log", …);
node.add_output_pin("log", "Log", …);

// CORRECT
node.add_input_pin("input_log", "Log", …);
node.add_output_pin("output_log", "Log", …);
```

### Node Best Practices
- Set default values on pins where sensible.
- Use `Options` for enum dropdowns; set JSON Schema for struct pins.
- Add `NodePermission` declarations for WASM nodes (native catalog nodes are trusted by default).
- Use `NodeImage` for images, `FlowPath` for files.
- Add `Success`/`Error` pins only at true system boundaries (external APIs, DBs). For most nodes, just `return Err(…)`.
- Add node/pin descriptions and quality scores (`privacy`, `security`, `performance`, `governance`, `reliability`, `cost` — 0–10).
- When updating a node's pins, **bump the node version**.
- Multiple pins with the same name within one direction let the user add more pins of that type.

### Available NodePermissions
`NetworkHttp`, `NetworkWebsocket`, `NetworkTcp`, `NetworkUdp`, `NetworkDns`, `StorageRead`, `StorageWrite`, `Variables`, `Cache`, `Streaming`, `Models`, `A2ui`, `OAuth`, `Functions`

---

## API Guidelines (packages/api/)

- **Always** verify user access to resources with `ensure_permission!(user, &app_id, &state, RolePermissions::<Permission>)` and enforce in SQL queries. Check comparable endpoints for the pattern.
- Annotate every endpoint with `utoipa` for OpenAPI docs and SDK generation. Include a short user-facing description.
- Optimize queries: use caching, avoid N+1 calls, think about high-frequency endpoints.

---

## TypeScript / React Guidelines

- TypeScript everywhere; functional components with hooks only.
- `useMemo`/`useCallback` for perf; proper `useEffect` dependency arrays.
- **UI**: shadcn components (already installed — import, don't recreate). Tailwind CSS for styling. Lucide icons.
- Avoid hardcoded color tokens; use design system tokens.
- Prefer `const`/`readonly`; use `?.` and `??`.
- Split large components into focused subcomponents (can colocate in the same file).

---

## Code Quality (Codacy)

After **every file edit**, run Codacy CLI analysis on the modified file:
- Tool: `codacy_cli_analyze` with `rootPath` = workspace path, `file` = edited file path.
- Fix any reported issues before moving on.

After **any package manager operation** (npm/bun/pnpm install, adding deps):
- Run `codacy_cli_analyze` with `tool: "trivy"` to check for vulnerabilities.
- Resolve security issues before continuing.

Codacy identifiers: `provider: gh`, `organization: Rheosoph`, `repository: flow-like`.

---

## CI Parity

PRs target `dev`. Run these locally before pushing — they match the CI pipeline:

```bash
# Clippy (same strict flags as CI)
cargo clippy -- -D clippy::correctness -D clippy::suspicious

# Format check
cargo fmt --check

# Security audit
cargo audit

# Tests (some require API keys via env vars)
cargo test -p <package-name>

# Frontend
cd apps/desktop && bunx biome check . && bunx tsc --noEmit
```

---

## Commit Conventions

Use conventional commit prefixes: `feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`.
Keep the subject under 72 characters. The release changelog groups by these labels.

---

## Good Practices

- **Read before writing**: always read existing code/patterns before modifying. Check neighboring files for conventions.
- **Incremental verification**: run `cargo check -p <pkg>` after each file edit, not just at the end. Catch errors early.
- **Match existing patterns**: when adding a new node, endpoint, or component, find the closest existing example and follow its structure.
- **No speculative abstractions**: solve the problem at hand. Don't add config, feature flags, or extension points for hypothetical future needs.
- **Error messages matter**: include enough context in error strings to diagnose without a debugger. Name the operation that failed and the value that was unexpected.
- **Test at boundaries**: validate external input (user input, API responses, file contents). Trust internal code paths and framework guarantees.
