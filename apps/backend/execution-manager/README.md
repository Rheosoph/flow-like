# Rust execution supervision

This crate supplies the trusted services that assign workflows to single-use
gVisor environments in Docker Compose and Kubernetes. Each prepared environment
accepts one execution and is destroyed afterward. The workflow runtime verifies
the signed dispatch before loading tenant code.

| Binary | Deployment role |
| --- | --- |
| `execution-manager docker` | Reserve warm containers, persist ownership in SQLite, supervise execution and confirm cleanup. |
| `execution-manager kubernetes` | Reserve warm Pod pairs, claim runs in Redis, supervise execution and confirm termination before removing network policy. |
| `execution-gateway --unix-warm` | Enforce one execution's HTTP egress policy through its private Compose Unix socket. |
| `execution-gateway --tcp` | Enforce the same policy in a separate Kubernetes gateway Pod. |
| `execution-slot` | Deliver one dispatch to the prepared Kubernetes executor and relay bounded output. |

The manager and gateway images build this crate without the workflow catalog or
executor dependency graph. The Kubernetes runner image contains both the slot
adapter and the existing Rust executor. Deployment setup and object-store
bootstrap scripts still use Python.

## Admission and isolation

The shared HTTP server authenticates dispatches, checks their shape and reserves
capacity before returning streaming success. Once admission begins, a detached
task owns the reservation through execution and cleanup. A browser disconnect
does not cancel accepted work or free its capacity early. An uncertain admission
failure cannot be automatically retried as an unstarted execution.

The gateway lives outside the tenant sandbox. It accepts one immutable policy,
checks callback identity and storage routes, validates integration DNS addresses
before connecting, and revokes connections on cancellation or expiry. Prefix
authorization remains the object store's responsibility through scoped STS
credentials. Runners receive no manager token, Docker socket, database credentials
or Kubernetes service-account token.

Docker uses a dedicated SQLite actor so durable claims do not block async workers.
Kubernetes uses atomic Redis claims and a retained multiplexed connection.
Lifecycle mutations are not retried after an ambiguous transport failure.
Cancellation succeeds only after termination is confirmed; failed cleanup closes
admission and retains state for reconciliation.

## Capacity and latency

Set `EXECUTION_MANAGER_WORKER_THREADS` in Compose or
`executionManager.workerThreads` in Helm to tune the manager's Tokio workers.
Both default to two. Active execution capacity, warm reserve and replacement
concurrency have separate limits. Each ready slot is additional to active
capacity, so reserve resources for both.

Prewarming moves container or Pod creation and catalog initialization ahead of
admission. Assignment still requires durable claims, gateway configuration and
dispatch delivery. Kubernetes also binds run identity and deadlines through API
requests. The warm reserve must contain at least one slot; an empty reserve
returns explicit non-admission while replenishment continues.

Rust removes the Python execution services and provides bounded async I/O. This
port does not establish a few-millisecond start guarantee. Measure
queue-to-first-node p50/p95/p99, replacement rate, completed executions per second
and resource use on the intended Linux hosts, including hour-long executions.

## Build, verify and upgrade

From the repository root:

```sh
cargo build --locked -p flow-like-execution-manager --bins
cargo test --locked -p flow-like-execution-manager --all-targets
```

Tests use local Unix/TCP sockets, fake Docker and Kubernetes APIs, a Redis wire
fixture and harmless child processes. They cover admission races, replay,
streaming, deadlines, policy enforcement and cleanup. Live gVisor, Cilium, RustFS
and load qualification require the actual deployment.

Before replacing the Python images, pause dispatch and drain managers and queue
bridges. Rebuild and pin the manager/gateway image and, for Kubernetes, the runner
image containing `/app/execution-slot`. Preserve the Compose SQLite volume and
Kubernetes Redis claims and cancellation markers. Their record formats and the
HTTP dispatch protocol remain compatible. Reconcile uncertain work before
resuming dispatch; retained claims must not be cleared to bypass replay checks.

Follow the deployment-specific prerequisites and configuration in the
[Compose guide](../docker-compose/execution-manager/README.md) or
[Kubernetes guide](../kubernetes/execution-manager/README.md).
