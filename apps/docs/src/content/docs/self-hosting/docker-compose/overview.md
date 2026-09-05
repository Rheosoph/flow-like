---
title: Docker Compose
description: Run Flow-Like with isolated executions and bundled storage on one Docker host
sidebar:
  order: 20
---

The Compose deployment runs Flow-Like on one Linux Docker host. It bundles
PostgreSQL, authenticated Redis and RustFS object storage. Its default execution
mode gives each run a fresh gVisor sandbox, supervised by a native Rust manager.

Use Compose when one host meets your capacity and recovery requirements. Adding
replicas increases process capacity on that host; it does not provide host
failover.

## Request and execution paths

Browser and desktop clients reach the web app, API and signaling through the
edge proxy. API replicas share the database, Redis and object storage.

Interactive executions go through the internal execution gateway to a manager.
Background executions enter Redis; queue bridges consume them and dispatch to
the same managers. A queue bridge runs no workflow code in the default mode.

Each manager prepares an inventory of unused runner and gateway containers.
Preparation initializes trusted runtime code before a request arrives. After
reserving a slot, the manager supplies one signed dispatch, streams its events,
then destroys the containers. The next execution receives a different sandbox.

The runner has no container network. It reaches its permitted callback routes,
object buckets and configured HTTPS integrations through its own Unix socket
proxy. See [Execution Backends](/self-hosting/execution-backends/) for the
isolation boundary and integration limits.

## Services

| Component | Responsibility |
| --- | --- |
| Edge proxy, web, API and signaling | Client access, authentication and collaboration |
| Execution managers and queue bridges | Admission, background dispatch, supervision and cancellation |
| Disposable runners and gateways | One execution and its permitted outbound traffic |
| Compiler | Bounded HTTP compilation of custom WASM nodes |
| PostgreSQL and `db-init` | Application metadata and gated schema initialization |
| Redis | Queues, execution state, caches and signaling coordination |
| RustFS and storage bootstrap | Private object buckets and scoped temporary credentials |
| Sink services | Schedules and configured event adapters |

The optional `monitoring` profile adds Prometheus, Grafana, Tempo and datastore
exporters.

## Start here

Follow [Prerequisites](/self-hosting/docker-compose/prerequisites/) and
[Installation](/self-hosting/docker-compose/installation/). Setup generates a
private environment file, builds and pins runner images, then checks the host
before starting services.

For a separate installation containing only trusted internal workflows,
`setup-env.py --mode trusted` enables shared workers. That mode does not isolate
hostile tenants per execution.

## Operate the deployment

- [Configuration](/self-hosting/docker-compose/configuration/) explains environment variables and public URLs.
- [Storage](/self-hosting/docker-compose/storage/) covers RustFS, signing endpoints and external storage.
- [Scaling](/self-hosting/docker-compose/scaling/) separates active capacity from warm reserves.
- [Monitoring](/self-hosting/docker-compose/monitoring/) describes collection and alert delivery.
- [Troubleshooting](/self-hosting/docker-compose/troubleshooting/) covers admission, storage and recovery failures.
