---
title: Scaling
description: Scale API and shared runtime workers on one Docker host
sidebar:
  order: 26
---

Docker Compose can add API and runtime processes on the same host. This improves
throughput and resilience to one process failure, but it does not create a
multi-host control plane or a fresh container for every run.

## Set replica counts

The template reads:

```dotenv
API_REPLICAS=2
RUNTIME_REPLICAS=3
```

Apply a change with:

```bash
docker compose up -d --scale api=3 --scale runtime=4
docker compose ps
```

The published API port belongs to `api-gateway`, which forwards to the internal
`api` service. API containers share PostgreSQL, Redis, object storage, backend
signing keys, and hub configuration.

Runtime replicas expose only the internal `runtime:9000` service. Interactive
runs use the HTTP execution backend; background runs can use the Redis queue,
which is consumed by every enabled runtime worker.

## Concurrency

Two settings bound work inside each runtime:

```dotenv
MAX_CONCURRENT_EXECUTIONS=10
QUEUE_WORKER_CONCURRENCY=10
```

Replica count multiplied by a concurrency value is a theoretical ceiling, not
a capacity guarantee. A run can be limited by CPU, memory, database
connections, object-storage bandwidth, model-provider quotas, or downstream
APIs first.

Increase concurrency only after measuring representative Flows. For
CPU-intensive, memory-intensive, or non-thread-safe workloads, more smaller
workers can be safer than one highly concurrent process.

## Resource limits

The runtime service declares resource limits and reservations under
`deploy.resources`. Docker Compose support for these fields depends on the
runtime and mode. Confirm the effective limits with Docker rather than assuming
the YAML was enforced.

Maintain deployment-specific values in an override:

```yaml
services:
  runtime:
    deploy:
      resources:
        limits:
          cpus: "8"
          memory: 16G
        reservations:
          cpus: "2"
          memory: 4G
```

Keep enough host capacity for image builds and all non-runtime services.

## Stateful bottlenecks

The Compose topology contains one PostgreSQL service and one Redis service.
Scaling API or runtime containers does not scale either datastore.

For an external PostgreSQL-compatible service, create an override that:

- supplies the external `DATABASE_URL` to API and `db-init`;
- removes or disables the bundled PostgreSQL dependency;
- applies the required TLS settings;
- preserves the normal schema-initialization step;
- backs up and migrates data deliberately.

Do the same level of planning before replacing Redis. A lone environment
variable is not enough because the base file wires service dependencies and
internal URLs.

## Services that should remain singletons

Keep `sink-services` at one replica unless the scheduler and every enabled sink
have been designed and tested for active/active operation. Multiple uncoordinated
schedulers can duplicate triggers.

`db-init` is a one-time job. `api-gateway` is the single published gateway in
the supplied topology.

## Docker Swarm

`docker-stack.yml` is provided for Swarm. Swarm does not build images from the
stack file and does not automatically load `.env`.

Build and push registry-backed images first, validate the interpolated stack,
and then deploy it:

```bash
docker compose build
docker stack config -c docker-stack.yml
docker stack deploy -c docker-stack.yml flowlike
```

Use explicit `*_IMAGE` values that every Swarm node can pull. Keep backend
keys, storage credentials, and sink tokens consistent across replicas and
prefer Swarm secrets over plain environment values for production.

## When to move to Kubernetes

Use the [Kubernetes deployment](/self-hosting/kubernetes/overview/) when you
need multi-host scheduling, cluster-native autoscaling, or network policies.
The chart's warm HTTP executor pool is implemented; its fresh-Job path is not
yet operational end to end. Compose remains a shared-worker, single-host
deployment even when several process replicas are running.
