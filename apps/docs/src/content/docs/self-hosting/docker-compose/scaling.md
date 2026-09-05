---
title: Scaling
description: Size active executions, warm sandboxes and service replicas on one Docker host
sidebar:
  order: 26
---

Scale Compose by changing service counts and resource limits for one host.
Execution managers maintain unused sandboxes separately from active runs, so
both consume resources. Moving beyond one host requires a different scheduling
and persistence arrangement, such as the
[Kubernetes deployment](/self-hosting/kubernetes/overview/).

## Active capacity and warm reserves

For `M` managers, active limit `C` per manager and warm reserve `W` per manager:

- Maximum admitted active runs: `M × C`.
- Additional warm slots: `M × W`.
- Each slot has one runner container and one gateway container.

For example:

```dotenv
EXECUTION_MANAGER_REPLICAS=2
MAX_CONCURRENT_EXECUTIONS=10
SANDBOX_WARM_POOL_SIZE=4
SANDBOX_CREATE_CONCURRENCY=2
QUEUE_BRIDGE_REPLICAS=2
QUEUE_WORKER_CONCURRENCY=10
```

This permits twenty active runs and prepares up to eight additional unused
slots. At the default limits, twenty-eight runners can consume up to 28 GiB,
and their gateways up to 3.5 GiB, before accounting for other services. CPU
limits can also exceed the host's physical capacity; measure contention.

Preparation and retirement count against the warm inventory budget. Assigned
slots are never returned to it. A positive warm reserve is required; setting
`SANDBOX_WARM_POOL_SIZE=0` is rejected.

When no ready slot or active capacity is available, the manager rejects
admission. Explicit rejection allows queued work to retry with backoff.
Ambiguous requests are retained for reconciliation because they may have
already executed.

## Throughput and start latency

Concurrency is the number of occupied slots; throughput is completed executions
per second. At steady state, required active slots are approximately the arrival
rate multiplied by the average execution duration. Ten starts per second with
one-hour executions would need about 36,000 active slots.

Warm preparation removes container creation and trusted runtime initialization
from normal admission. Artifact fetches, credential issuance, storage and
downstream services still contribute to startup and completion time. A
few-millisecond end-to-end start is not a measured guarantee.

Increase `SANDBOX_CREATE_CONCURRENCY` when preparation cannot replace consumed
slots quickly enough. Increase the warm reserve to cover bursts. Measure the
reserve level, preparation errors and startup latency under representative
work before increasing active capacity.

`EXECUTION_MANAGER_WORKER_THREADS` defaults to two Tokio workers. It controls
the manager's async scheduling threads independently of execution capacity.

## Apply changes

Set replica counts and limits in `.env`, then run:

```bash
python3 scripts/preflight.py
python3 scripts/up.py
docker compose ps
docker stats --no-stream
```

API, compiler and execution gateways refresh Docker DNS every ten seconds.
They do not retry execution POSTs after ambiguous acceptance. Drain before
scaling managers down; their default stop grace allows hour-long executions
and final cleanup.

API, web, compiler and signaling have separate replica settings. Keep
`sink-services` at one replica unless every enabled scheduler and adapter has
been tested for concurrent ownership.

## Shared services and limits

All managers on this daemon must share the local `execution_manager_state`
volume. It contains SQLite ownership and replay records. Do not place it on
NFS or reuse this arrangement across multiple Docker hosts.

API database pools scale with `API_REPLICAS`. Preflight reserves one extra API
replica for rollout and ten administrative connections when comparing pools
against `POSTGRES_MAX_CONNECTIONS`.

The bundled PostgreSQL, Redis and RustFS services remain single instances.
Use the supplied external-service overlays for managed datastores, with a
separate backup and migration procedure. Merely adding worker replicas does not
increase datastore capacity or protect against host loss.

For an explicitly trusted shared-worker installation, `RUNTIME_REPLICAS`
controls reused workers instead of execution managers. The legacy Swarm file
also supports trusted shared execution with external storage only. Neither is
a substitute for per-execution isolation.
