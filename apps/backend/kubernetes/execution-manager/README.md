# Single-use Kubernetes execution slots

The execution manager prepares clean gVisor runner Pods before requests arrive.
Each runner has a separate gateway Pod that enforces its network policy outside
the gVisor sandbox. A slot accepts one signed execution, then both Pods are
destroyed. An environment that has run tenant code never returns to the pool.

The manager, gateway and slot adapter are Rust binaries from the shared
[execution-manager crate](../../execution-manager/README.md). They use async I/O
and the same lightweight dispatch type as the API and workflow runtime. The
runner image contains the Rust slot adapter and executor; Python remains in
deployment and object-store bootstrap tools only.

The default `per_run` Helm deployment routes interactive requests to the manager
and asynchronous work through the Redis queue bridge. `trusted_shared` explicitly
enables the shared server and uses HTTP for both dispatch paths. The shared server
has a configurable admission limit and drains HTTP and streaming executions under
one shutdown deadline.

## Prerequisites

Install gVisor on the execution nodes and configure the named RuntimeClass to use
it. The chart requires Cilium policy enforcement for `per_run`, including host,
remote-node, Kubernetes API and metadata egress denial. Standard Kubernetes
NetworkPolicy rules alone do not provide the node isolation this mode needs.
See [Kubernetes NetworkPolicy behavior](https://kubernetes.io/docs/concepts/services-networking/network-policies/)
and [Cilium deny policies](https://docs.cilium.io/en/stable/security/policy/deny/).

Set the kubelet `podPidsLimit` on execution nodes to bound process creation.
This deployment uses the node setting for its portable PID limit. See
[Kubernetes PID limits](https://kubernetes.io/docs/concepts/policy/pid-limiting/).
The manager sets CPU and memory limits, a read-only filesystem, a bounded private
temporary volume, non-root users, dropped capabilities and RuntimeDefault
seccomp. Runner and gateway Pods have no service-account token or host mounts.

Pin `executionManager.image.digest` and `executionManager.sandbox.image` to
published images. The manager image also supplies the external gateway. The
queue bridge uses the Compose runtime image; it consumes queue messages and
checks terminal API state without executing workflow code.

Before accepting a dispatch, each initialized slot must connect to its own
gateway and fail to connect to the Kubernetes API, its node's kubelet endpoint
and the cloud metadata endpoint. These checks catch missing or broad network
policy. Qualify peer isolation, node routes, IPv6 and metadata denial on the
actual cluster before admitting untrusted tenants.

## Size active capacity and the warm reserve

| Helm value | Meaning |
| --- | --- |
| `executionManager.replicaCount` | Independent managers; each owns and replenishes its slots. |
| `executionManager.workerThreads` | Tokio worker threads per manager, default 2; separate from active execution capacity. |
| `executionManager.maxConcurrentExecutions` | Active execution limit per manager. |
| `executionManager.warmPoolSize` | Additional clean, initialized slots kept ready per manager. |
| `executionManager.warmPoolCreationConcurrency` | Maximum parallel slot initializations per manager. |
| `executionManager.warmPoolMaxAgeSeconds` | Idle lifetime before a clean slot is replaced. |
| `executionManager.sandbox.memoryMb`, `cpus`, `tmpMb` | Per-run resource limits. |
| `executionManager.sandbox.nodeSelector`, `tolerations` | Placement of runner and gateway Pods on execution nodes. |
| `executionManager.queueBridge.replicaCount`, `concurrency` | Queue consumer count and in-flight deliveries per consumer. |
| `executor.timeout` | Workflow execution budget, 3,600 seconds by default. |

Reserve resources for active slots **plus** warm slots. Each slot has a runner
and a gateway Pod. A gateway requests 32 MiB and is limited to 128 MiB; a runner
requests its configured memory limit. The manager never accepts another request
into an assigned slot. If no initialized slot is available, it returns HTTP 429
with `X-Execution-Admitted: false` before sending streaming headers. The queue
bridge retains these unstarted deliveries and retries them within the queue age
limit.

Startup and terminal callbacks have separate grace budgets of 30 and 60 seconds.
The workflow still receives its full configured execution budget. On assignment,
the manager lowers each Pod's `activeDeadlineSeconds` using its actual Kubernetes
start timestamp. The gateway independently expires the execution capability.
Deployment shutdown stops admission, removes idle slots and drains admitted work;
the Helm termination grace period covers the workflow and supervisor budgets.

## Follow an execution

1. The manager creates slot-specific NetworkPolicy resources before creating the
   Pods. The runner can send TCP traffic only to its own gateway on port 3128.
   Manager Pods alone can reach the runner's assignment port and the gateway's
   policy configuration port.
2. The runner initializes the catalog, prepared registry and public JWT
   verification key, then waits for its first input. It has no tenant payload or
   object-store credentials while waiting.
3. Admission removes one ready slot from the local pool and claims the run in
   Redis with `SET NX EX`. Competing manager replicas cannot assign the same run
   while this claim exists. The manager binds the run identifier and deadline to
   both Pods, checks cancellation markers, and configures the external gateway
   exactly once.
4. The manager sends one `{mode, payload}` envelope to the waiting runner. The
   Rust runtime verifies the signed dispatch before fetching executable content.
   The gateway permits the selected execution callbacks, authorized storage
   origins and approved HTTPS integrations. Prefix-scoped STS credentials enforce
   object permissions inside the configured buckets.
5. The manager relays bounded NDJSON or SSE output, including idle heartbeats.
   The queue bridge settles delivery only after independently reading terminal
   API state. The manager waits for kubelet-reported termination and removes the
   Pods before deleting their restrictive NetworkPolicies.

Cancellation writes a shared marker before looking up assigned Pods. Markers
survive manager replacement and prevent a concurrent assignment from starting
after cancellation. The manager shortens Pod deadlines and waits for termination;
it does not report success based on forced removal of an API object. A node or
control-plane partition can prevent confirmation. In that case admission closes
and cancellation reports that termination could not be confirmed.

Assignment claims use `exec:claims:v1:<namespace>:<release>:<sha256-run-id>` and
expire after 24 hours plus execution, supervisor and cleanup allowances. Claims
contain a slot identifier, never dispatch credentials. The manager verifies Redis
availability before becoming ready and reuses a multiplexed connection with
bounded concurrency and two-second connection and command timeouts. An uncertain claim is not retried.
TLS connections verify both the certificate chain and hostname; URL query
overrides that could change these settings are rejected.

For a dedicated Redis ACL user, allow `PING`, `SET`, and the matching
`~exec:claims:v1:<namespace>:<release>:*` key pattern, plus `SELECT` when using a
nonzero database. The client uses RESP2 and disables library-identification
commands. Run credentials never enter this client. The chart supplies the trusted
control-plane Redis secret to the manager; deployments can provide separate
credentials with the same restricted commands.

Replay prevention depends on retaining Redis data. The bundled Redis configuration
flushes its append-only log every second, so a crash can lose recent claims.
Restoring an older Redis snapshot can also remove claims for executions that
already ran. Reconcile accepted work before resuming dispatch after such a
rollback. This setup does not promise exactly-once external side effects. Size
Redis for retained claims as well as queue and run state: 1,000 executions per
second produces at least 86.4 million claim keys over 24 hours.

## Observe capacity and qualify latency

The manager exposes `/health`, `/ready` and `/metrics` on port 9000. Readiness
means that supervision is available; inspect `executor_warm_slots` to see how
many executions can start from prepared environments. Metrics also expose active
jobs, admission capacity, warm failures, slots being initialized or retired, and
assignment duration counters. A failed
supervision or cleanup operation closes admission and restarts the manager after
draining. Kubernetes owner references clean up its dependent resources.

Prewarming removes Pod scheduling, image loading and catalog initialization from
the request path. Assignment still performs Kubernetes API operations, gateway
configuration and a runner HTTP request. Workflow artifacts and tenant data can
still require storage access before the first node. Completion also waits for
termination confirmation. No few-millisecond latency or sustained throughput
claim has been measured for this implementation.

Benchmark queue-to-first-node p50/p95/p99, completed executions per second, warm
slot replacement rate, Kubernetes API load and resource usage with representative
short and hour-long workflows. Include cancellation and worker replacement under
load. Increase the warm reserve and creation concurrency only while the API
server, CNI and execution nodes have capacity. A future shared egress broker and
an execution controller with fewer Kubernetes operations per assignment can
reduce lifecycle cost further.

Run the local controller and HTTP transport checks with:

```sh
cargo test --locked -p flow-like-execution-manager
```

These tests exercise one-time policy assignment, concurrent slot claims,
cancellation ordering, Redis replay claims, verified TLS configuration, cleanup
and a complete HTTP dispatch to a harmless child process. They do not replace live
gVisor, Cilium, RustFS or load qualification.
