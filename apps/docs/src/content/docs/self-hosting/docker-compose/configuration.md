---
title: Configuration
description: Configure Flow-Like services, execution, storage, and observability in Docker Compose
sidebar:
  order: 23
---

Docker Compose reads deployment values from
`apps/backend/docker-compose/.env`. Start from `.env.example`; it is the
versioned reference for the current stack.

## Public endpoints

| Variable | Template default | Used for |
| --- | --- | --- |
| `WEB_PORT` | `3001` | Published web-app port |
| `API_PORT` | `8080` | Published Nginx API-gateway port |
| `NEXT_PUBLIC_API_URL` | `http://localhost:8080` | API URL used by the browser |
| `NEXT_PUBLIC_REDIRECT_URL` | `http://localhost:3001/callback` | Login callback |
| `NEXT_PUBLIC_REDIRECT_LOGOUT_URL` | `http://localhost:3001/` | Post-logout redirect |
| `SIGNALING_PORT` | `4444` | Published realtime signaling port |
| `COMPILER_PORT` | `8081` | Published WASM compiler port |

The web URLs are build arguments. Rebuild `web` when they change. The
signaling URL and authentication provider are also represented in the hub
configuration file, so keep both sources consistent.

:::caution[Published ports]
The template publishes PostgreSQL, Redis, signaling, and compiler ports as
well as the web and API ports. Use host firewall rules or a Compose override to
bind or remove ports that must not be reachable from an untrusted network.
:::

## Database and Redis

| Variable | Template default | Notes |
| --- | --- | --- |
| `POSTGRES_USER` | `flowlike` | PostgreSQL role |
| `POSTGRES_PASSWORD` | `flowlike_dev_change_me` | Change for every real deployment |
| `POSTGRES_DB` | `flowlike` | Database name |
| `POSTGRES_PORT` | `5432` | Host port; containers use `postgres:5432` |
| `REDIS_PORT` | `6379` | Host port; containers use `redis:6379` |
| `EXECUTION_STATE_BACKEND` | `redis` | `redis` or `postgres` |
| `EXECUTION_STATE_TTL_SECONDS` | `86400` | Retention for run-state entries |

The supplied Compose file constructs its internal database and Redis URLs. A
managed external database requires a deliberate Compose override, not only a
new `DATABASE_URL` line in `.env`.

## Replicas and limits

| Variable | Template default | Notes |
| --- | --- | --- |
| `API_REPLICAS` | `2` | Internal API replicas behind `api-gateway` |
| `RUNTIME_REPLICAS` | `3` | Shared runtime replicas |
| `MAX_CONCURRENT_EXECUTIONS` | `10` | Concurrent runs per runtime process |
| `EXECUTION_TIMEOUT_SECONDS` | `3600` | Maximum run duration |
| `QUEUE_WORKER_CONCURRENCY` | `10` | Redis queue consumers per runtime |
| `QUEUE_POLL_TIMEOUT_SECS` | `30` | Blocking queue-poll timeout |

`MAX_CONCURRENT_EXECUTIONS` and `QUEUE_WORKER_CONCURRENCY` describe different
entry paths. Size both against CPU, memory, external-service limits, and the
work done by a typical Flow.

## Execution dispatch

The API has separate configuration lanes for interactive and background runs:

| Variable | Template default | Purpose |
| --- | --- | --- |
| `EXECUTION_BACKEND` | `http` | Interactive/streaming dispatch |
| `ASYNC_EXECUTION_BACKEND` | `redis` | Background dispatch |
| `EXECUTOR_URL` | `http://runtime:9000` | HTTP runtime target |
| `REDIS_EXECUTION_QUEUE` | `exec:jobs` | Redis list consumed by runtime workers |
| `QUEUE_WORKER_ENABLED` | `true` | Enables the runtime's Redis consumer |

The Compose topology is wired for HTTP and Redis. Other supported dispatch
backends need their own infrastructure and environment variables; see
[Execution Backends](/self-hosting/execution-backends/).

## Backend trust

| Variable | Required | Purpose |
| --- | --- | --- |
| `BACKEND_KEY` | Yes for API dispatch | Base64 PKCS#8 P-256 private key; API only |
| `BACKEND_PUB` | Yes | Base64 P-256 public key; API, runtime, and compiler |
| `BACKEND_KID` | Recommended | JWKS key identifier |

Generate a matching set from the repository root:

```bash
./tools/gen-execution-keys.sh --export
```

Do not use the older `EXECUTION_KEY`, `EXECUTION_PUB`, or `EXECUTION_KID`
names with this stack.

## Object storage

| Variable | Template default | Purpose |
| --- | --- | --- |
| `STORAGE_PROVIDER` | `aws` | Backing-store provider; the stock API build supports Azure/GCP and the documented R2 bridge, but omits AWS runtime credentials |
| `RUNTIME_CREDENTIALS_PROVIDER` | Copied `.env.example` leaves it empty | Must be deleted from `.env` or set to a non-empty supported provider; explicit empty is invalid |
| `META_BUCKET` | `flow-like-meta` | App and Flow metadata |
| `CONTENT_BUCKET` | `flow-like-content` | User and App content |
| `LOG_BUCKET` | `flow-like-logs` | Execution logs |
| `CDN_BUCKET_NAME` | Copied `.env.example` leaves it empty | Set an actual bucket/container; the Compose template injects the empty value and prevents fallback |

Provider-specific credentials, endpoints, bucket overrides, scoped runtime
credentials, and S3-compatible services are covered in
[Storage Providers](/self-hosting/docker-compose/storage/).

The same explicit-empty behavior applies to the provider-specific bucket names
that Compose injects. Set the selected provider's meta, content, and log names
even when they match the generic names.

## WASM compiler

| Variable | Template default | Purpose |
| --- | --- | --- |
| `COMPILATION_BACKEND` | `http` | `http` or `redis` — the API never compiles WASM in-process, so a compiler service is required |
| `COMPILER_URL` | `http://compiler:8081` | Internal compiler endpoint |
| `COMPILER_TIMEOUT_SECS` | `600` | Compiler timeout |
| `COMPILER_CALLBACK_TIMEOUT_MS` | `10000` | Callback request timeout |
| `COMPILER_CALLBACK_RETRIES` | `3` | Callback retry count |
| `COMPILER_MAX_PARALLEL_TARGETS` | all CPU cores | Target build concurrency |

## Server-side Event services

| Variable | Required when enabled | Purpose |
| --- | --- | --- |
| `SINK_SECRET` | Yes | Shared secret used to sign sink trigger JWTs |
| `SINK_TRIGGER_JWT` | Yes | Scoped token used by `sink-services` |
| `SINK_TOKEN_ENCRYPTION_KEY` | Yes in production | Encrypts stored sink tokens |
| `FLOW_LIKE_RUNTIME_CONFIG_FILE` | Yes | Hub configuration mounted into the service |

Supported sink types are enabled in the hub configuration, not with one
environment switch per adapter.

## Model-provider proxy

The template can pass credentials and endpoints for OpenRouter, OpenAI,
Anthropic, Azure OpenAI, AWS Bedrock, and Google Vertex AI. Populate only the
providers exposed by your hub configuration and model records. Keep provider
keys in `.env` or a deployment secret store, never in the JSON hub
configuration.

The complete variable names and default endpoints are grouped under
**LLM / Model Provider Configuration** in `.env.example`.

## Observability

| Variable | Template default | Purpose |
| --- | --- | --- |
| `PROMETHEUS_PORT` | `9091` | Published Prometheus port |
| `GRAFANA_PORT` | `3002` | Published Grafana port |
| `GRAFANA_ADMIN_USER` | `admin` | Initial Grafana user |
| `GRAFANA_ADMIN_PASSWORD` | `admin` | Change before exposure |
| `TEMPO_HTTP_PORT` | `3200` | Published Tempo query port |
| `TEMPO_OTLP_GRPC_PORT` | `4317` | OTLP gRPC |
| `TEMPO_OTLP_HTTP_PORT` | `4318` | OTLP HTTP |
| `OTEL_TRACES_SAMPLER_ARG` | `0.1` | Trace sampling ratio |

These containers start only with the `monitoring` profile. See
[Monitoring](/self-hosting/docker-compose/monitoring/).

## Validate changes

```bash
docker compose config --quiet
docker compose up -d --build
docker compose ps --all
```

Use `docker compose config` without `--quiet` only in a trusted terminal:
interpolated output can contain credentials.
