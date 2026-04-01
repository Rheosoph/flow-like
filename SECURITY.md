# Security Policy

Flow-Like is a visual workflow automation platform built in Rust. Security is a core design principle — from the sandboxed WASM execution engine to the capability-based permission system.

For a comprehensive overview of the security architecture, see the [Security documentation](https://docs.flow-like.com/reference/security/).

## Supported Versions

Flow-Like is currently in early access. Security patches are applied to the latest release on the `dev` branch.

| Version | Supported          |
| ------- | ------------------ |
| Latest  | :white_check_mark: |
| Older   | :x:                |

We recommend always running the latest version. There is no backporting of security fixes to older releases at this time.

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Please report vulnerabilities privately to [security@great-co.de](mailto:security@great-co.de).

### What to include

- A description of the vulnerability and its potential impact
- Steps to reproduce or a proof-of-concept
- The affected component (e.g. WASM sandbox, API auth, executor, storage)
- Your suggested severity (Critical / High / Medium / Low)

### What to expect

- **Acknowledgment** within 48 hours
- **Initial assessment** within 5 business days
- **Regular updates** until the issue is resolved
- Credit in the release notes (unless you prefer anonymity)

We follow coordinated disclosure — we ask that you give us reasonable time to address the issue before any public disclosure.

## Security Architecture

Flow-Like employs multiple layers of defense:

### WASM Sandboxing

All third-party code runs inside isolated [Wasmtime](https://wasmtime.dev/) sandboxes with:

- **Memory isolation** — each node has its own memory space, separate from the host
- **CPU time limits** — fuel-based metering terminates runaway loops
- **No filesystem access** by default — WASI is disabled unless explicitly enabled
- **No network access** by default — HTTP/WebSocket calls require declared permissions
- **Capability-based permissions** — nodes declare what they need; the runtime enforces it

### Authentication & Authorization

- **OIDC/OAuth2** for user authentication (delegated to external identity providers)
- **API keys** and **Personal Access Tokens (PATs)** for programmatic access
- **ES256 JWTs** for inter-service communication (executor ↔ API)
- **Role-based access control (RBAC)** with 25+ granular permissions

### Data Protection

- **TLS everywhere** — all HTTP traffic uses rustls (no OpenSSL dependency)
- **Local-first** — data stays on your hardware by default
- **Scoped storage** — nodes can only access their own storage directories
- **No telemetry** without explicit opt-in

### Supply Chain

- Rust dependencies are audited via `cargo-audit`
- Container images are minimal and pinnable
- Third-party license inventory is maintained in [`thirdparty/`](./thirdparty/)

## Scope

The following are in scope for security reports:

- WASM sandbox escapes or privilege escalation
- Authentication or authorization bypasses
- Injection vulnerabilities (SQL, command, path traversal)
- Sensitive data exposure (credentials, tokens, PII)
- Denial-of-service vulnerabilities in the executor or API
- Supply chain issues in dependencies

The following are out of scope:

- Vulnerabilities in third-party WASM nodes (report to the node author)
- Social engineering attacks
- Physical access attacks
- Issues in deprecated or unsupported versions

## Security-Related Configuration

When self-hosting Flow-Like, review these hardening guides:

- [Kubernetes Security Notes](https://docs.flow-like.com/self-hosting/kubernetes/security/)
- [WASM Sandboxing & Permissions](https://docs.flow-like.com/dev/wasm-nodes/sandboxing/)

## Contact

- Security reports: [security@great-co.de](mailto:security@great-co.de)
- General support: [https://flow-like.com](https://flow-like.com)
