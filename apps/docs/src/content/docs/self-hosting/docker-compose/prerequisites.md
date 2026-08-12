---
title: Prerequisites
description: Prepare the host, identity provider, object storage, and network
sidebar:
  order: 21
---

The Compose deployment is a complete single-host stack, not only an API and one
runtime. It builds and runs the web app, API gateway and replicas, runtime
workers, compiler, Event sink service, signaling service, PostgreSQL, Redis,
and an initialization job.

## Host tools

Install:

- Git;
- Docker Engine or Docker Desktop;
- the Docker Compose v2 plugin;
- OpenSSL for the backend-key generator;
- Bun when you need to generate server-side Event tokens.

Verify the commands used by the guide:

```bash
docker version
docker compose version
openssl version
bun --version
```

The Compose implementation must support service health conditions, profiles,
configs, and service scaling.

## Identity provider

The template hub configuration uses OpenID Connect placeholders. Prepare an
OIDC application with:

- an authority/discovery URL and JWKS endpoint;
- a client ID;
- the Flow-Like callback and post-logout URLs;
- the claims required by your chosen user lookup settings.

Do not expose a deployment that still points to
`https://your-auth-provider.com`.

## Object storage

Prepare three buckets or containers:

- metadata;
- content;
- execution logs.

The stock Compose API includes Azure, GCP, and R2 runtime credentials. Its
backing-store adapter also understands AWS/S3-compatible endpoints, but the
checked-in target omits the AWS runtime feature and cannot start with that
provider until rebuilt. Credentials must be able to read, write, list, and
delete objects in the configured locations. Runtime credential scoping needs
additional provider-specific setup.

See [Storage Providers](/self-hosting/docker-compose/storage/) before starting
the stack.

## Capacity planning

There is no useful universal CPU or memory minimum: a Flow that calls an API is
very different from a Flow that processes a large model or dataset.

The template starts two API replicas and three runtime replicas. Each runtime
declares a 4-CPU/8-GB limit and a 1-CPU/2-GB reservation. Docker Compose does
not enforce every `deploy` resource field consistently outside Swarm, so
observe the actual host as well as the file.

For a development laptop, begin with:

```dotenv
API_REPLICAS=1
RUNTIME_REPLICAS=1
MAX_CONCURRENT_EXECUTIONS=2
QUEUE_WORKER_CONCURRENCY=2
```

Leave headroom for Docker image builds, PostgreSQL, Redis, the compiler, the
web app, and optional monitoring. Size production from representative Flow
runs and measured concurrency rather than the example defaults.

## Network access

The host needs outbound access to:

- the container and package registries used during builds;
- the configured object-storage endpoint;
- the OIDC provider;
- any model providers, APIs, or data systems used by Flows.

The template publishes these core ports:

| Port | Service |
| --- | --- |
| `3001` | Web app |
| `8080` | API gateway |
| `4444` | Realtime signaling |
| `8081` | WASM compiler |
| `9092` | Compiler metrics |
| `5432` | PostgreSQL |
| `6379` | Redis |

The monitoring profile publishes additional ports. In production, normally
expose only the web/API endpoints and the signaling endpoint required by
clients. Restrict database, Redis, compiler, metrics, and observability ports
with firewall rules or a Compose override.

## TLS and DNS

The Compose stack does not terminate public TLS. Prepare DNS and a reverse
proxy or load balancer for HTTPS/WSS, then update:

- `NEXT_PUBLIC_API_URL`;
- login and logout redirect URLs;
- the hub configuration's `domain`, `app`, `web`, `secure`, and `signaling`
  fields.

Rebuild the web and API images after changing build-time public
configuration.
