# Per-execution isolation

The execution manager accepts authenticated dispatches and assigns each workflow to a pristine, prewarmed gVisor container. Each container executes once and is destroyed. A separate gateway container holds that run's HTTP egress policy. The runner has no external network interface, Redis connection, database key, manager token, or Docker socket.

The manager and gateway are native Rust binaries from the shared [execution-manager crate](../../execution-manager/README.md). Tokio handles concurrent network I/O; a dedicated SQLite thread keeps registry writes away from async workers. The gateway drops privileges before starting its worker threads. Python is used only by deployment and storage-bootstrap tools.

This implementation needs qualification on a Linux execution host before serving untrusted tenants. The local tests cover the launch contract, deadlines, cleanup, HTTP policy, and a real Unix-socket proxy. They do not establish gVisor compatibility with every catalog node or storage SDK operation.

## Host prerequisites

Use a dedicated Linux Docker daemon for execution. The manager is a trusted control-plane service: access to its Docker socket permits control of that daemon. Keep its HTTP port on a private network. Workflows receive only a separate Unix socket with run-scoped proxy access.

Install a maintained gVisor release and configure its Docker runtime:

```json
{
  "runtimes": {
    "runsc": {
      "path": "/usr/local/bin/runsc",
      "runtimeArgs": ["--network=none", "--host-uds=open"]
    }
  }
}
```

Restart the daemon after changing its runtime configuration. The manager checks those flags and the installed runner and gateway images before becoming ready. Missing gVisor, unavailable images, and unsupported runtime configuration close admission. There is no automatic switch to ordinary containers.

The `--host-uds=open` flag permits connecting to the one Unix socket in the mounted run volume. No host directories or engine sockets are mounted into the runner. gVisor documents its [network isolation](https://gvisor.dev/docs/user_guide/networking/), [Unix-socket flags](https://github.com/google/gvisor/blob/master/runsc/config/config.go), and [shared filesystem mount behavior](https://gvisor.dev/docs/user_guide/filesystem/). Keep the mounted socket directory in shared mode, which is gVisor's default for bind mounts.

The runner image must contain `/app/runtime` and `/usr/bin/timeout`. The gateway image is built from this directory's Dockerfile. Pin both images using either a repository digest (`registry/image@sha256:...`) or the immutable local image ID (`sha256:...`). The manager never pulls an image during a request and accepts no image, command, mount, network, or resource options from a dispatch.

## Configuration contract

| Variable | Meaning |
| --- | --- |
| `EXECUTION_MANAGER_TOKEN_FILE` | Preferred manager authentication secret file. Overrides `EXECUTION_MANAGER_TOKEN`. Minimum 32 characters. |
| `SANDBOX_IMAGE` | Pinned runner image. |
| `SANDBOX_GATEWAY_IMAGE` | Pinned manager/gateway image. |
| `SANDBOX_RUNTIME` | Must be `runsc`. |
| `SANDBOX_GATEWAY_NETWORK` | Existing, private Docker network that can reach the callback and bucket gateways. |
| `EXECUTION_INSTALLATION_ID` | Stable installation label, used to reconcile only this installation's expired containers and volumes. |
| `EXECUTION_CALLBACK_URL` | Fixed private HTTP callback origin. Must match the API's signed dispatch. |
| `EXECUTION_OBJECT_STORE_URL` | S3 origin appearing in runtime credentials and signed artifact URLs. |
| `EXECUTION_OBJECT_STORE_TLS_GATEWAY` | Set `true` only when an HTTPS object origin terminates at a bucket-only proxy that rejects STS and administration. Required for HTTPS storage. Never use this flag with a raw object-store endpoint. |
| `META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET` | Bucket names accepted by the HTTP storage proxy. |
| `EXECUTION_ALLOWED_HTTPS_HOSTS` | Optional comma-separated exact integration hostnames. Only port 443 is allowed; private, loopback, link-local, multicast, and reserved DNS results are denied. No wildcard matching. |
| `MAX_CONCURRENT_EXECUTIONS` | Concurrent runs per manager, 1–1024; default 4. Excess requests get HTTP 429. |
| `EXECUTION_TIMEOUT_SECONDS` | Execution budget, 1–86400 seconds; default 3600. |
| `SANDBOX_WARM_POOL_SIZE` | Ready reserve per manager, additional to active capacity; default 2, minimum 1. |
| `EXECUTION_MANAGER_WORKER_THREADS` | Async worker threads per manager; default 2. Execution concurrency has its own limit. |
| `SANDBOX_CREATE_CONCURRENCY` | Maximum parallel pool replenishment; default 2. |
| `SANDBOX_IDLE_TIMEOUT_SECONDS` | Maximum time an unused ready sandbox remains in the pool; default 300. |
| `SANDBOX_STARTUP_TIMEOUT_SECONDS` | Startup/assignment grace, forwarded as `EXECUTION_STARTUP_GRACE_SECONDS`; default 120. |
| `EXECUTION_TERMINAL_GRACE_SECONDS`, `EXECUTION_CLEANUP_TIMEOUT_SECONDS` | Finalization budgets, default 60 and 30 seconds. |
| `EXECUTION_MANAGER_STATE_PATH` | Shared local SQLite ownership database. Compose mounts `/state/executions.sqlite3` on a persistent local volume. |
| `BACKEND_PUB` | Public verification key prepared before the runner advertises readiness. |
| `SANDBOX_MEMORY_MB`, `SANDBOX_CPUS`, `SANDBOX_PIDS`, `SANDBOX_TMP_MB` | Fixed per-run resource class; defaults 2048 MiB, 1 CPU, 256 PIDs, and 512 MiB scratch. |

The runner's writable `/tmp` is a bounded tmpfs with `noexec`, `nosuid`, and `nodev`. Nodes that need executable temporary files require a separately reviewed class; the manager does not weaken these settings for individual requests.

The Rust manager requires a local Unix Docker socket. `DOCKER_HOST` defaults to `unix:///var/run/docker.sock`; remote TCP and SSH endpoints are unsupported. Run a manager beside each dedicated execution daemon.

## Dispatch, queue delivery, and streaming

The API or queue bridge sends the full `DispatchPayload` to `POST /execute`, `/execute/stream`, or `/execute/sse` with `X-Execution-Manager-Token`. The manager rejects general user tokens and launcher options. The runner verifies the API JWT and its complete dispatch binding before fetching or loading executable artifacts.

`/execute` waits for a terminal result. An authenticated queue bridge adds `X-Execution-Queued: true`, which selects the runner's required terminal-status acknowledgement. A successful response requires both a terminal result and exit status zero. Unexpected exit, missing terminal output, deadline expiry, and unconfirmed cleanup return an execution error; queue delivery must remain unsettled or require reconciliation.

The queue bridge safely returns explicitly non-admitted work to the Redis v3 queue with backoff and its original publication time. Queue age is bounded; expired or ambiguous deliveries remain visible for reconciliation. It never automatically retries an uncertain execution response. After a manager error, only an independently confirmed hard cancellation can settle delivery.

The queue bridge uses the runtime image with `EXECUTION_ISOLATION_MODE=per_run`, `QUEUE_WORKER_ENABLED=true`, `EXECUTION_MANAGER_URL`, `API_URL`, and the manager token. Before settling delivery, it independently checks the trusted API's durable terminal status for the matching run. Runner stdout alone cannot settle a queue job. This mode exposes health and metrics routes only. It forwards the complete signed payload and never initializes the catalog or executes a workflow locally. `trusted_shared` is an explicit alternative for trusted workflows.

The runner accepts exactly one JSON document through stdin:

```text
/app/runtime --once callback
/app/runtime --once callback-queued
/app/runtime --once stream
```

The input is limited to 8 MiB. stdout carries NDJSON; tracing goes to stderr. The manager discards runner stderr, bounds each event to 1 MiB and total output to 64 MiB, and retains only a small event queue. Streaming requests receive heartbeats while work is quiet. A client disconnect closes delivery to that client while accepted execution continues under the same deadline and admission slot.

## Network boundary

Docker and gVisor both disable the runner's external networking. A loopback HTTP proxy relay inside the runner connects to `/gateway/proxy.sock` on its read-only, per-run volume. Ignoring proxy environment variables cannot create direct external connectivity.

The external gateway enforces the signed run's callback capability, app widget identity, and channel identity. Its callback routes permit progress, events, execution JWKS, widgets for the selected app, and the selected run's channel operations. General API management, model APIs, arbitrary hub calls, sibling run channels, and other app widget routes are unavailable unless a separate reviewed capability is implemented.

For HTTP S3, the gateway checks the bucket path and denies STS and administration. Prefix authorization remains the object store's responsibility and must be verified with the bundled store's STS acceptance suite. For HTTPS S3, the configured TLS gateway must enforce those bucket and administration restrictions because CONNECT hides HTTP paths from the run gateway.

Integration grants use exact HTTPS hostnames. Each new connection checks all resolved IPv4 and IPv6 addresses and connects to a validated address without resolving it again. Redirects produce a new proxy request and must pass the same grant. Raw TCP, UDP, private integration destinations, GPU access, and privileged nodes have no fallback in this class.

## Shutdown and recovery

Warm preparation finishes before assignment. The runtime waits for one newline-terminated `{mode,payload}` envelope after announcing readiness. A persistent Docker Engine client handles lifecycle operations; the request path reserves a prepared slot, binds its run, configures its proxy and writes stdin. No tenant code or credentials enter an unused pool slot.

The runner has an independent PID 1 timeout covering its maximum warm and execution lifetime. A supervisor deadline starts at assignment and includes separate startup, execution and finalization budgets. Completion and failure remove the runner, gateway and socket volume. Normal shutdown withdraws admission and drains assigned work; Compose gives managers and queue bridges 65 minutes. Cleanup failure closes admission and leaves retained ownership for reconciliation.

A local SQLite registry shared by manager replicas records assignments, deadlines and cancellation. Assignment is atomic and occurs before tenant bytes enter an already-running sandbox. Cancellation records its marker before enumerating assigned resources, then confirms removal. There is no later container-start operation that can race past cancellation. Assignment and cancellation records remain for at least 24 hours to fence token replay. Indexed expiry runs in bounded batches; dedicated heartbeats continue independently of Docker cleanup.

Keep every manager for one Docker daemon on the same local state volume. Do not use NFS or discard this volume during upgrades. Drain/reconcile old queues and stop old managers before moving to these images and the default `exec:jobs:v3`. Mixing the previous Docker-marker protocol with the new registry is unsupported. Do not automatically replay uncertain work after manager or host failure.

`GET /health` reports process liveness; `/ready` reports admission readiness. An empty warm pool returns explicit non-admission, and replenishment continues. `/metrics` exposes active/ready/creating slots, capacity, creation failures, assignment counters and execution outcome counts. The assignment timing is local registry reservation time, not deployment-to-first-node latency. Keep HTTP private because health and metrics are unauthenticated.

## Verification

Run the Rust protocol and lifecycle tests from the repository root:

```sh
cargo test --locked -p flow-like-execution-manager
```

Before enabling untrusted tenants, run actual gVisor fixtures that prove the mounted Unix socket works, the runner cannot reach host or metadata addresses, approved storage and callbacks succeed, prefix probes fail, and descendants terminate on resource exhaustion and deadline. Test native nodes, LanceDB, model loading, multipart uploads, manager restart, daemon failure, cancellation, and client disconnect. These fixtures require the Linux execution daemon and built images; a passing fake-engine suite is not that qualification.
