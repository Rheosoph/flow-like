---
title: Kubernetes
description: Deploy Flow-Like with the repository's Helm chart
sidebar:
  order: 10
---

The Helm chart deploys Flow-Like with a private RustFS object store and a Rust
execution manager that prepares a clean gVisor sandbox for each execution. Use
this path for untrusted workflows from multiple tenants. The cluster must provide
gVisor and Cilium before installation.

The chart lives at `apps/backend/kubernetes/helm/`. Follow
[Installation](/self-hosting/kubernetes/installation/) for the generated Secrets,
image builds and deployment commands. For trusted local development, the
[k3d workflow](/self-hosting/kubernetes/local-development/) uses an explicitly
selected shared runtime.

## Components and defaults

| Component | Default behavior |
| --- | --- |
| API and web | One Deployment replica each |
| Execution manager | One Rust supervisor, ten active executions and two additional warm slots |
| Execution slot | One single-use gVisor runner Pod paired with a separate gateway Pod |
| Queue bridge | One trusted Redis consumer forwarding background work to the manager |
| RustFS | One persistent storage Pod, bucket initialization Job and two object gateway replicas |
| Redis | Authenticated single instance with persistent queue and replay records |
| Database | Single-node CockroachDB for evaluation; external PostgreSQL or CockroachDB for production |
| Monitoring | Prometheus, Grafana and Tempo enabled |
| Compiler, signaling and sink services | Optional |
| Public ingress | Disabled until domains and TLS are configured |

The default execution settings are `execution.isolationMode=per_run`,
`execution.backend=http` and `execution.asyncBackend=redis`. The API sends
interactive requests directly to the manager; the queue bridge sends background
requests to the same admission path.

## What isolates a run

The runner initializes trusted runtime code before it receives tenant input. On
assignment, it receives one signed dispatch and scoped temporary storage
credentials. Its network policy permits traffic only to its paired gateway. That
gateway enforces the selected callbacks, object store and approved HTTPS
destinations outside the tenant sandbox.

After completion or cancellation, the manager confirms runner termination before
removing its network policy. A used runner never returns to the warm reserve.
Runner and gateway Pods receive no Kubernetes service account token, database
credentials, Redis password, storage root key or API signing key.

Read [Executor](/self-hosting/kubernetes/executor/) for capacity and lifecycle
settings, and [Security](/self-hosting/kubernetes/security/) for the Cilium policy
requirements and the limits of this boundary.

## Scale and availability

API replicas, manager replicas, warm reserve size, queue consumers and compiler
concurrency have separate settings. Adding managers increases configured
execution capacity only when the execution nodes have enough resources. Account
for both active and idle warm Pod pairs.

Bundled RustFS, Redis and the internal database are single-instance data services.
More application replicas do not provide storage failover. Review
[Storage](/self-hosting/kubernetes/storage/) and
[Database](/self-hosting/kubernetes/database/) before selecting an availability
target.

Warm initialization removes process creation from admission. It does not establish
a few-millisecond start guarantee: Kubernetes requests, credential issuance and
signed artifact preparation still contribute to latency. Qualify isolation,
failure recovery and representative load on the actual cluster before exposing
tenants.

## Operator guides

- [Prerequisites](/self-hosting/kubernetes/prerequisites/)
- [Installation](/self-hosting/kubernetes/installation/)
- [Configuration](/self-hosting/kubernetes/configuration/)
- [Helm chart and Secret contracts](/self-hosting/kubernetes/helm/)
- [Monitoring](/self-hosting/kubernetes/monitoring/)
- [API service](/self-hosting/kubernetes/api/)
- [Scripts](/self-hosting/kubernetes/scripts/)
- [kubectl basics](/self-hosting/kubernetes/kubectl-basics/)
