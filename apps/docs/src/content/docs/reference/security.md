---
title: Security Architecture
description: How Flow-Like protects your data, executes untrusted code safely, and handles authentication
sidebar:
  order: 0
  badge:
    text: Important
    variant: caution
---

Flow-Like is designed with a defense-in-depth approach. Every layer — from the WASM execution sandbox to the API authentication stack — is built to minimize attack surface and protect user data.

## Design Principles

| Principle | Implementation |
|-----------|---------------|
| **Local-first** | Data stays on your hardware. No cloud dependency unless you choose to deploy remotely. |
| **Least privilege** | WASM nodes get zero capabilities by default. Every permission must be explicitly declared and approved. |
| **Memory safety** | The core engine is written in Rust — no buffer overflows, use-after-free, or data races. |
| **Defense in depth** | Multiple independent layers: sandbox isolation, capability enforcement, authentication, RBAC, TLS. |
| **Auditability** | Complete data lineage, typed execution traces, and structured logging. |

---

## WASM Sandbox

All third-party nodes run inside isolated [Wasmtime](https://wasmtime.dev/) sandboxes. This is the primary security boundary between untrusted code and your system.

### Isolation Guarantees

Each WASM node instance receives:

- **Separate linear memory** — the node cannot read or write host process memory
- **No filesystem access** — WASI is disabled by default (`allow_wasi: false`)
- **No network access** — WASI networking is disabled by default (`allow_wasi_network: false`)
- **No OS peripheral access** — no clipboard, camera, microphone, or other hardware
- **No inter-node visibility** — a node can only see its own inputs and memory

### Resource Limits

The runtime enforces hard limits on every WASM execution:

| Resource | Default | Restrictive | Permissive |
|----------|---------|------------|------------|
| **Memory** | 64 MB | 16 MB | 4 GB |
| **CPU fuel** | 10B instructions (~10s) | 1B (~1s) | 100B (~100s) |
| **Timeout** | 30s | 10s | 300s |
| **Stack depth** | Configurable | Reduced | Increased |
| **Tables/memories** | Bounded | Minimal | Extended |

Fuel metering and epoch-based interruption ensure that runaway loops or deliberate resource abuse cannot impact the host system. When a limit is exceeded, the execution is terminated immediately.

### Capability System

Rather than granting blanket access, Flow-Like uses a bitflag-based capability system. A node's declared permissions map to specific capability bits that are checked at every host function call:

| Capability | What it controls |
|-----------|-----------------|
| `STORAGE_READ` | Read from storage backends |
| `STORAGE_WRITE` | Write to storage backends |
| `STORAGE_DELETE` | Delete from storage |
| `HTTP_GET` | Make outbound HTTP GET requests |
| `HTTP_WRITE` | Make outbound HTTP POST/PUT/DELETE requests |
| `WEBSOCKET` | Open persistent WebSocket connections |
| `TCP` | Open TCP socket connections |
| `UDP` | Open UDP socket connections |
| `DNS` | Perform DNS lookups |
| `VARIABLES_READ` | Read workflow variables |
| `VARIABLES_WRITE` | Write workflow variables |
| `CACHE_READ` / `CACHE_WRITE` | Use the execution cache |
| `STREAMING` | Stream incremental output |
| `A2UI` | Generate dynamic UI |
| `MODELS` | Invoke AI/ML model inference |
| `OAUTH` / `TOKEN` | Use authentication flows |
| `FUNCTIONS` | Call registered host functions |

**Default capability set (`STANDARD`):** `STORAGE_READ | HTTP_GET | VARIABLES_READ | CACHE_ALL`

**Compound capabilities:**
- `NETWORK_ALL` = `HTTP_GET | HTTP_WRITE | WEBSOCKET | TCP | UDP | DNS`
- `STORAGE_ALL` = `STORAGE_READ | STORAGE_WRITE | STORAGE_DELETE`
- `AUTH_ALL` = `OAUTH | TOKEN`

Anything not in the default set must be explicitly requested by the node and approved by the user.

### Security Configuration

The `WasmSecurityConfig` struct brings all sandbox controls together:

- **`limits`** — resource constraints (memory, CPU, timeout)
- **`capabilities`** — bitflag permission set
- **`allow_wasi`** — filesystem/environment access (default: off)
- **`allow_wasi_network`** — WASI networking (default: off)
- **`allowed_hosts`** — optional allowlist for HTTP targets

For details on how permissions appear to end users, see [Sandboxing & Permissions](/dev/wasm-nodes/sandboxing/).

---

## Authentication

Flow-Like supports multiple authentication methods depending on the deployment context.

### User Authentication (OIDC / OAuth2)

For interactive users, Flow-Like delegates authentication to an external OpenID Connect provider. The API proxies the standard OIDC endpoints:

- `/auth/discovery` — OpenID configuration
- `/auth/jwks` — JSON Web Key Set
- `/auth/authorize` — Authorization endpoint
- `/auth/token` — Token exchange
- `/auth/userinfo` — User info lookup
- `/auth/revoke` — Token revocation

The JWT middleware validates every request, resolves the user against the database, and loads their role memberships.

### API Keys

For service-to-service or automation use cases, API keys are supported via the `X-API-Key` header. These are associated with technical users that have explicit role assignments.

### Personal Access Tokens (PAT)

For programmatic access by human users, PATs are supported via the `Authorization: PAT <token>` header. These carry the same permissions as the issuing user.

### Executor JWT (Internal)

The executor (which runs workflows) authenticates to the API using ES256 (ECDSA P-256) JWTs. These tokens are:

- **Short-lived** — include `exp`, `nbf`, and `iat` claims
- **Scoped** — contain `run_id`, `app_id`, `board_id`, and `event_id`
- **Audience-restricted** — validated against `flow-like-executor`
- **Issuer-verified** — must come from `flow-like`

Public keys are distributed via environment variables or fetched from the API's JWKS endpoint.

---

## Authorization (RBAC)

Flow-Like implements role-based access control with 25+ granular permissions organized into logical groups:

| Group | Permissions |
|-------|------------|
| **Admin** | `Owner`, `Admin` (implicitly grant all others) |
| **Team** | `ReadTeam`, `ReadRoles` |
| **Files** | `ReadFiles`, `WriteFiles` |
| **API** | `InvokeApi` |
| **Boards** | `ReadBoards`, `ExecuteBoards`, `WriteBoards` |
| **Events** | `ListEvents`, `ReadEvents`, `ExecuteEvents`, `WriteEvents` |
| **Observability** | `ReadLogs`, `ReadAnalytics` |
| **Configuration** | `ReadConfig`, `WriteConfig` |
| **Content** | `ReadTemplates`, `WriteTemplates`, `ReadCourses`, `WriteCourses`, `ReadWidgets`, `WriteWidgets`, `WriteRoutes` |
| **Meta** | `WriteMeta` |

Permissions are stored as bitflags for efficient evaluation. A user's effective permissions are the union of all their role assignments.

---

## Transport Security

- **TLS everywhere** — all HTTP traffic uses [rustls](https://github.com/rustls/rustls), a memory-safe TLS implementation written in Rust. There is no OpenSSL dependency.
- **HTTP/2 support** — the API server supports HTTP/2 via Axum
- **No plaintext fallback** — production deployments must use TLS-terminated ingress

---

## Data Protection

### Local-First Architecture

By default, all data remains on the user's machine:

- Workflow definitions, execution state, and outputs are stored locally
- AI models can run entirely on-device (llama.cpp, ONNX Runtime, Candle)
- No data is sent to external services without explicit user action

### Storage Scoping

WASM nodes access storage through controlled host functions. Each storage operation is bound to a specific scope:

| Scope | Access |
|-------|--------|
| **Board storage** | Shared across nodes in the same board |
| **Node-scoped storage** | Private to a specific node instance |
| **User-scoped storage** | Tied to the authenticated user |
| **Upload directory** | Controlled ingress path for user-provided files |
| **Cache directory** | Optionally node-scoped and/or user-scoped |

A node cannot access storage outside its declared scope, regardless of what it attempts.

### Credential Handling

- Secrets and API keys are stored in backend configuration, not in workflow definitions
- In Kubernetes deployments, credentials should use workload identity (IRSA / GKE Workload Identity / AKS federated identity) over static keys
- Static keys should be stored in Kubernetes Secrets with RBAC restrictions
- OAuth tokens for third-party services are stored in memory and scoped to sessions

---

## Supply Chain Security

Flow-Like takes several measures to secure its dependency chain:

- **Dependency auditing** — Rust dependencies are auditable via `cargo-audit`
- **License inventory** — complete third-party license information is maintained in the [`thirdparty/`](https://github.com/Rheosoph/flow-like/tree/dev/thirdparty) directory
- **Minimal containers** — production container images use minimal base images
- **Image pinning** — container images can be pinned by digest
- **Image signing** — compatible with [cosign](https://docs.sigstore.dev/cosign/overview/) for image verification
- **TLS stack** — uses rustls across the board, eliminating OpenSSL as an attack vector
- **Observability** — Prometheus metrics, OpenTelemetry tracing, and Sentry error tracking for anomaly detection

---

## Deployment Hardening

### Kubernetes

For production Kubernetes deployments:

- **Kata Containers** — run workflows in lightweight VM boundaries via `runtimeClass`
- **Network policies** — restrict executor egress to storage endpoints only
- **Workload identity** — avoid static S3 credentials; use cloud-native identity
- **RBAC** — limit access to Kubernetes secrets
- **Admission policies** — enforce image signatures

See [Kubernetes Security Notes](/self-hosting/kubernetes/security/) for detailed configuration.

### Docker Compose

For Docker Compose deployments:

- Keep services on an internal network
- Use environment variables or Docker secrets for credentials
- Pin image versions
- Monitor with the built-in Prometheus/Grafana stack

---

## Reporting Vulnerabilities

**Do not open a public GitHub issue for security vulnerabilities.**

Report privately to [security@great-co.de](mailto:security@great-co.de).

| Step | Timeline |
|------|----------|
| Acknowledgment | Within 48 hours |
| Initial assessment | Within 5 business days |
| Regular updates | Until resolved |
| Credit in release notes | Unless you prefer anonymity |

### In Scope

- WASM sandbox escapes or privilege escalation
- Authentication or authorization bypasses
- Injection vulnerabilities (SQL, command, path traversal)
- Sensitive data exposure (credentials, tokens, PII)
- Denial-of-service in the executor or API
- Supply chain issues in dependencies

### Out of Scope

- Vulnerabilities in third-party WASM nodes (report to the node author)
- Social engineering attacks
- Physical access attacks
- Issues in deprecated or unsupported versions
