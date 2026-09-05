---
title: Executor
description: Configure single-use gVisor Pods, warm capacity and the Rust execution manager
sidebar:
  order: 70
---

The Rust execution manager assigns each run to a preinitialized, single-use
gVisor runner Pod. A separate gateway Pod enforces that run's network permissions.
The runner is destroyed after use and the manager prepares a replacement in the
background.

The default configuration uses `execution.isolationMode=per_run`. Both
interactive HTTP dispatch and the Redis queue bridge use this manager. The chart
rejects the legacy Kubernetes Job backend for isolated execution.

## Follow one execution

1. The manager creates a runner NetworkPolicy and its paired gateway policy
   before creating either Pod.
2. The gateway starts with no execution grant. The runner checks its gateway
   connection and prohibited cluster/node/metadata endpoints, initializes the
   trusted Rust catalog and JWT verification key, then waits for input.
3. Admission removes a ready slot and atomically claims the run in Redis with
   `SET NX EX`. An existing claim prevents another manager replica from
   assigning the same run.
4. The manager binds both Pods to the run and deadline, checks cancellation
   markers, configures the gateway once and sends a signed dispatch.
5. The runtime verifies the dispatch before fetching executable artifacts.
   It executes once and streams bounded events and terminal acknowledgement.
6. The manager confirms runner termination, removes the Pod pair and only then
   removes the restrictive policies.

The slot adapter shares the runner's sandbox. Its HTTP server is a transport
adapter; cancellation, deadlines and egress are enforced externally. No sandbox
is reused across tenants or executions.

## Capacity and warm reserve

| Helm value | Default | Effect |
| --- | ---: | --- |
| `executionManager.replicaCount` | 1 | Independent manager partitions |
| `executionManager.maxConcurrentExecutions` | 10 | Active runs per manager |
| `executionManager.warmPoolSize` | 2 | Additional clean slots per manager |
| `executionManager.warmPoolCreationConcurrency` | 2 | Concurrent preparation per manager |
| `executionManager.warmPoolMaxAgeSeconds` | 600 | Unassigned slot lifetime |
| `executionManager.workerThreads` | 2 | Async supervisor workers |
| `executionManager.queueBridge.replicaCount` | 1 | Background dispatch consumers |
| `executionManager.queueBridge.concurrency` | 10 | Outstanding jobs per consumer |

Set runner CPU, memory and temporary storage under `executionManager.sandbox`.
The default runner requests 1 GiB of memory, has a one-CPU limit and a 256 MiB
memory-backed temporary volume. Include the separate gateway and gVisor overhead
in node sizing.

Configured concurrency is approximately manager replicas multiplied by the active
limit. Sustainable throughput also depends on run duration and preparation rate:
100 executions per second with a 60-second average duration require about 6,000
concurrent executions. Adding replicas cannot compensate for exhausted nodes.

When no clean slot is available, immediate requests are refused without starting
a run. Queue delivery can wait within its age budget. Monitor available warm slots
and preparation failures as well as active execution counts.

## Latency and long runs

Warm initialization removes process creation from the admission path. The Rust
manager reuses asynchronous network connections and binds the two Pods in
parallel. Kubernetes API requests, Redis claims, credential issuance and artifact
preparation still contribute to start time. No few-millisecond start guarantee
has been measured.

`executor.timeout` defaults to one hour. Separate startup, terminal and cleanup
allowances cover supervision, and Pod deadlines remain in force if a manager
disappears. Graceful manager shutdown stops new admission and allows accepted
executions to drain.

The default RustFS session request is two hours. Actual credential expiry must
cover queue wait, execution and the supervisor allowances. See
[Configuration](/self-hosting/kubernetes/configuration/#time-budgets) before
changing either the workflow or credential limits.

## Cancellation and replay protection

Cancellation writes a shared ConfigMap marker before looking up assigned Pods.
The manager shortens Pod deadlines and waits for kubelet-reported termination.
It does not equate forced deletion of a Pod API object with a stopped sandbox.
If a node or API partition prevents confirmation, cancellation reports failure
and admission closes.

Redis claims survive manager replacement and expire after 24 hours plus the
execution and supervisor allowances. A lost claim reply is not retried. Retain
Redis data across upgrades; restoring a snapshot can remove claims for work that
already ran. External side effects still require application-level idempotency.

The `exec:jobs:v3` queue retains accepted delivery until trusted terminal
confirmation. Ambiguous delivery and expired queue items are quarantined for
reconciliation. Do not replay them solely because the client lost its connection.

## Inspect execution

```bash
kubectl get pods -n flow-like -l app.kubernetes.io/component=execution-sandbox -o wide
kubectl get pods -n flow-like -l app.kubernetes.io/component=execution-egress -o wide
kubectl logs deployment/flow-like-execution-manager -n flow-like --tail=100
kubectl logs deployment/flow-like-queue-bridge -n flow-like --tail=100
kubectl port-forward service/flow-like-execution-manager 9000:9000 -n flow-like
```

Manager `/ready` reports supervisor availability. Inspect `/metrics` to establish
whether warm capacity is actually available. Runtime dispatch and cancellation
endpoints require the manager token and should remain private.

## Trusted local workflows

`execution.isolationMode=trusted_shared` with
`execution.asyncBackend=http` enables the existing reusable executor pool.
Configure its replicas and bounded concurrency under `executorPool`. It shares
a process between executions and does not meet the multi-tenant isolation
requirement.

The source entry points are
`apps/backend/execution-manager/src/kubernetes/`,
`apps/backend/kubernetes/executor/src/main.rs` and
`packages/executor/`. The Kubernetes runner uses `/app/execution-slot` to
deliver one `--once warm` dispatch to the Rust executor.
