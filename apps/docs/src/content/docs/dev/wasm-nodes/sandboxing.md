---
title: Sandboxing & Permissions
description: How WASM node sandboxing works, what permissions mean, and what to know before running third-party code
sidebar:
  order: 26
  badge:
    text: Important
    variant: caution
---

When you add a WASM node to a workflow, you're running code that was written by someone outside the Flow-Like core team. This guide explains how Flow-Like keeps that code contained, what the permission system does, and how to make informed trust decisions.

## The short version

- Every WASM node runs inside an **isolated sandbox** — it cannot access your files, network, or system unless explicitly allowed.
- Nodes **declare which permissions** they need (e.g. network access, streaming). You see these before anything runs.
- You can **trust a package once** and skip the prompt for future workflows.
- If a node declares no permissions, it can only do pure computation — read inputs, return outputs.

---

## What is sandboxing?

Think of a WASM sandbox like a sealed room with no windows or doors. The code inside can think and calculate, but it can't see or touch anything outside. Flow-Like only opens specific, controlled hatches when a node requests them.

### Technical details

Flow-Like uses [Wasmtime](https://wasmtime.dev/), a production-grade WebAssembly runtime, to run every external node. Each node gets:

| Isolation layer | What it means |
|-----------------|---------------|
| **Separate memory** | The node has its own memory space. It cannot read or write the host process memory. |
| **No filesystem access** | Unless specifically granted node/user storage, the node cannot touch any files. |
| **No network by default** | HTTP, WebSocket, and other network calls are blocked unless the node declares the permission. |
| **CPU time limits** | Nodes have execution budgets. A runaway loop will be terminated, not your machine. |
| **Memory caps** | Each node has a memory ceiling. It cannot allocate unbounded RAM. |
| **Deterministic execution** | WASM execution is reproducible — same inputs produce same outputs. |

This is fundamentally different from running a native plugin or a shell script, which typically has full access to your system.

---

## How permissions work

### The old way vs. the new way

Previously, permissions were declared in the package manifest (`flow-like.toml`). This was a static, package-wide declaration. Now, permissions are declared **per-node** directly in code. When a node's `get_node` or `get_nodes` function returns its definition, it includes a list of permissions. This means:

- Different nodes in the same package can request different permissions.
- The permission list is always up to date with the actual code.
- If a node requests no permissions, it needs nothing beyond basic computation.

### Available permissions

| Permission | What it allows | Risk level |
|-----------|----------------|------------|
| `streaming` | Send output incrementally as it's produced (e.g. token-by-token LLM responses) | Low — data only flows outward through the normal output channel |
| `network:http` | Make HTTP requests to external services | **Medium** — the node can talk to the internet |
| `network:websocket` | Open persistent WebSocket connections | **Medium** — similar to HTTP but long-lived |
| `storage:read` | Read from the storage backend | Low — read-only access to stored data |
| `storage:write` | Write to the storage backend | Medium — can persist data |
| `storage:node` | Access a private, per-node storage area | Low — scoped to this node only |
| `storage:user` | Access user-level storage | Medium — shared across nodes |
| `variables` | Read and write flow variables | Low — scoped to the current workflow |
| `cache` | Use the execution cache to skip redundant work | Low — performance optimization only |
| `models` | Access AI/ML models configured in Flow-Like | Medium — can invoke model inference |
| `a2ui` | Generate dynamic UI elements at runtime | Low — visual only, no system access |
| `oauth` | Use OAuth authentication flows | **Medium** — involves user credentials |
| `functions` | Call registered host functions | Medium — depends on which functions are exposed |

### What "no permissions" means

A node with an empty permissions list can only:

- Read its input pins
- Write to its output pins
- Do computation in memory (math, string manipulation, data transformation)

It cannot call out to the network, read files, stream output, or invoke models. This is the safest category.

---

## The confirmation dialog

When you run a workflow that contains WASM nodes you haven't previously approved, Flow-Like shows a confirmation dialog. It lists:

1. **Which packages** are about to run
2. **What permissions** each package needs (aggregated across all its nodes in the workflow)
3. **Trust options** — how long your approval should last

### Trust levels

| Option | Scope | When to use |
|--------|-------|-------------|
| **One-time** | This execution only | You want to test something once |
| **This event** | All executions triggered by this event | You trust the workflow for this specific trigger |
| **This board** | All executions of this board | You trust the workflow regardless of how it's triggered |
| **Trust these packages everywhere** | All workflows using these packages | You fully trust the package author |

Package-level trust is stored locally on your machine. It's never sent to a server. You can clear it at any time from your browser's local storage (keys prefixed with `wasm-consent-package-`).

---

## Performance implications

WASM adds a small overhead compared to native Rust nodes. Here's what to expect:

| Aspect | Impact | Details |
|--------|--------|---------|
| **Startup** | ~1-5 ms per node | The WASM module is compiled on first load and cached afterward |
| **Execution** | ~1.1-1.3x native speed | Wasmtime's optimizing compiler produces near-native code |
| **Memory** | Slightly higher | Each node has its own memory space (default cap applies) |
| **Host calls** | ~0.1 ms per call | Crossing the sandbox boundary (e.g. reading storage) has a small cost |
| **Caching** | Transparent | Compiled modules are cached — subsequent loads are nearly instant |

### When performance matters

For most workflows, the overhead is negligible. It becomes noticeable when:

- A node is called **thousands of times** in a tight loop (consider batching)
- The node makes **many small host calls** (consider batching reads/writes)
- The node processes **very large data** in memory (watch the memory cap)

Native Rust nodes remain the best choice for performance-critical hot paths. WASM nodes are ideal for extensibility, integrations, and logic that changes frequently.

---

## Writing secure nodes

If you're a node author, follow these guidelines:

### Request only what you need

```rust
// Good — only requests what this node actually uses
node! {
    name: "fetch_data",
    friendly_name: "Fetch Data",
    description: "Downloads data from an API",
    category: "Integrations/HTTP",

    inputs: { exec: Exec, url: String },
    outputs: { exec_out: Exec, body: String },

    permissions: ["network:http"],
}
```

```rust
// Bad — requests everything "just in case"
node! {
    name: "fetch_data",
    // ...
    permissions: ["network:http", "network:websocket", "storage:write",
                  "models", "oauth", "functions"],
}
```

Users will see the full permission list and may decline to run a node that asks for too much.

### No permissions is the default

If your node only does computation, don't declare any permissions:

```rust
node! {
    name: "parse_csv",
    friendly_name: "Parse CSV",
    description: "Parses CSV text into structured data",
    category: "Data/Transform",

    inputs: { exec: Exec, csv_text: String },
    outputs: { exec_out: Exec, rows: Json },
}
```

This node will show "No additional permissions requested" in the UI, which builds user trust.

### Permissions are enforced at runtime

Even if a node doesn't declare a permission, the sandbox enforces restrictions. A node that tries to make an HTTP call without `network:http` will get an error, not silent access. Permissions are a contract between the node and the runtime.

---

## FAQ

**Can a WASM node access my clipboard, camera, or microphone?**
No. The sandbox has no OS peripheral access.

**Can a WASM node read other nodes' data?**
No. Each node only sees its own input pins and memory.

**Can a malicious node mine cryptocurrency?**
It could try to use CPU, but the execution time limit will terminate it quickly. And it has no network access to submit results unless `network:http` is granted.

**What happens if I trust a package and it updates?**
Your trust is per package ID — if the same ID ships a new version, your trust persists. Review the changelog of packages you update.

**Can I revoke trust?**
Yes. Clear localStorage entries starting with `wasm-consent-package-` in your browser devtools, or clear app data in the desktop app.

**Are permissions the same as capabilities in the manifest?**
The manifest (`flow-like.toml`) previously held a static `[permissions]` section. This has been superseded by per-node declarations in code. The manifest capabilities (memory limits, timeouts) are still respected and layered on top.
