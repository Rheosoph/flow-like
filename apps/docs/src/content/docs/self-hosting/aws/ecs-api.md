---
title: ECS API
description: Run the AWS API as a long-running ECS service for internal hosting, next to or instead of the Lambda API.
sidebar:
  order: 25
---

The Lambda API stays the default AWS entry point. `aws-api-ecs`
(`apps/backend/aws/api-ecs`) serves the same router, state and data plane from a
long-running process on ECS (Fargate or EC2). Use it when an account cannot run
Lambda, when the API must stay inside a private VPC behind an internal load
balancer, or when sustained traffic makes a fixed task cheaper than invocations.

Both binaries build their state through the same startup code
(`apps/backend/aws/shared/api_bootstrap.rs`) and read the same environment:
`SECRET_PREFIX`, `CDN_BUCKET_*`, `DSQL_*` or `DATABASE_URL`, and the execution
backend settings described in [service operations](/self-hosting/aws/operations/).
Executors, the file tracker and the maintenance scheduler work unchanged. Point
the maintenance Lambda's `API_BASE_URL` at whichever API clients reach.

## Image

CI publishes `ghcr.io/<owner>/flow-like-aws-api-ecs` as one AMD64 and ARM64
index. To build it locally from the repository root:

```sh
docker buildx build \
  --platform linux/arm64 \
  --load \
  --tag flow-like-aws-api-ecs:local \
  --file apps/backend/aws/api-ecs/Dockerfile \
  .
```

The image is a stripped binary built with ThinLTO and LLD on
`gcr.io/distroless/cc-debian12:nonroot`: no shell, no package manager, UID
65532. The code is compiled for Graviton2 (`neoverse-n1`) on ARM64 and
`x86-64-v2` on AMD64. For older EC2 capacity, pass a lower target such as
`--build-arg TARGET_CPU_ARM64=generic`. If you enable `readonlyRootFilesystem`,
mount a writable volume at `/tmp`.

## Task definition

```json
{
  "requiresCompatibilities": ["FARGATE"],
  "networkMode": "awsvpc",
  "runtimePlatform": { "cpuArchitecture": "ARM64", "operatingSystemFamily": "LINUX" },
  "cpu": "1024",
  "memory": "2048",
  "containerDefinitions": [
    {
      "name": "api",
      "image": "ghcr.io/<owner>/flow-like-aws-api-ecs@sha256:<digest>",
      "essential": true,
      "portMappings": [{ "containerPort": 8080, "protocol": "tcp" }],
      "stopTimeout": 30,
      "healthCheck": {
        "command": ["CMD", "/app/api", "healthcheck"],
        "interval": 15, "timeout": 3, "retries": 3, "startPeriod": 60
      },
      "dependsOn": [{ "containerName": "otel-collector", "condition": "START" }],
      "environment": [
        { "name": "SECRET_PREFIX", "value": "/flow-like/prod/" },
        { "name": "CDN_BUCKET_NAME", "value": "<cdn-bucket>" },
        { "name": "DSQL_CLUSTER_ENDPOINT", "value": "<id>.dsql.<region>.on.aws" },
        { "name": "DSQL_USER", "value": "flow_like_api" },
        { "name": "FLOW_LIKE_OTEL_ENABLED", "value": "true" }
      ],
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/flow-like/api",
          "awslogs-region": "<region>",
          "awslogs-stream-prefix": "api"
        }
      }
    },
    {
      "name": "otel-collector",
      "image": "public.ecr.aws/aws-observability/aws-otel-collector:latest",
      "essential": false,
      "command": ["--config=/etc/ecs/ecs-default-config.yaml"]
    }
  ]
}
```

Give the task role the permissions of the Lambda API's execution role: SSM
parameters under `SECRET_PREFIX`, `dsql:DbConnect`, the storage buckets, the
execution queues or functions, and the state tables. Add
`xray:PutTraceSegments` and `xray:PutTelemetryRecords` for the collector. The
task execution role only pulls the image and writes logs. Pin the collector
image to a version in production.

`dependsOn` makes ECS stop the API before the collector, so the final trace
export still has somewhere to go.

## Load balancer

Register the service with an ALB target group (target type `ip`, HTTP, port
8080) or with Service Connect.

- **Health check:** path `/health/ready`, success code `200`.
- **Idle timeout:** keep the ALB idle timeout below `HTTP_IDLE_TIMEOUT_SECS`
  (60 s and 75 s by default). A target that closes a keep-alive connection the
  load balancer still considers open produces intermittent 502 responses. Raise
  both together for long streams.
- **Deregistration delay:** ECS deregisters a task and waits this long before it
  sends SIGTERM. Set it to your longest ordinary request, such as 30 s; the
  300 s default only slows deployments.
- **CloudFront in front:** the origin response timeout applies between packets,
  so streamed responses need data or heartbeats more often than that timeout.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `PORT` | `8080` | Listen port. The `healthcheck` command reads it too |
| `HTTP_IDLE_TIMEOUT_SECS` | `75` | Idle keep-alive and request-header timeout; must exceed the ALB idle timeout |
| `SHUTDOWN_TIMEOUT_SECS` | `20` | Drain window after SIGTERM; add 5 s and stay within `stopTimeout` |
| `MAX_CONCURRENT_REQUESTS` | unset | In-flight request limit; beyond it the task answers `503` with `Retry-After: 1` instead of queueing |
| `DSQL_MAX_CONNECTIONS` | `32` | DSQL pool size (the Lambda API defaults to 4) |
| `DATABASE_POOL_MAX_CONNECTIONS` | `10` | Pool size for the `DATABASE_URL` path |
| `CORS_ALLOWED_ORIGINS` | unset | Comma-separated HTTP(S) origins or `tauri://localhost`; unset keeps the Lambda API's permissive policy |
| `TOKIO_WORKER_THREADS` | visible CPUs | Worker threads; set it to `1` on fractional-vCPU tasks |
| `RUST_LOG` | `warn` | Log filter, for example `info` |

Size `MAX_CONCURRENT_REQUESTS` above the load at your scaling target, so
shedding covers only the minutes before new tasks start. Shed responses bypass
the API's request metrics; watch the target group's `HTTPCode_Target_5XX_Count`.
A permit is held until response headers are ready, so a long stream does not
occupy one.

## Health and shutdown

`/health/live` answers `200` while the process runs. `/health/ready` answers
`200` until shutdown starts and `503` after. Neither checks the database: if
every task failed its health check during a database outage, ECS would keep
replacing healthy tasks. Use `/api/v1/health/db` to diagnose the database.
Health requests do not create traces or request metrics.

On SIGTERM the task stops accepting connections. HTTP/1 connections close once
their current response completes, and HTTP/2 connections receive GOAWAY.
Requests still running after `SHUTDOWN_TIMEOUT_SECS` are cut off, buffered traces
get up to 4 s to export, and the process exits.

## Telemetry

Logs are JSON lines on stdout. With `FLOW_LIKE_OTEL_ENABLED=true`, request
metrics are written as embedded metric format lines. CloudWatch Logs extracts
them from `awslogs` output, so dashboards built for the Lambda API work without
a separate metrics pipeline.

Traces go over OTLP gRPC to `OTEL_EXPORTER_OTLP_ENDPOINT`, by default the
collector sidecar at `http://127.0.0.1:4317`. Sampling follows the Lambda API's
rules. `FLOW_LIKE_OTEL_SAMPLE_RATE` (default `0.05`) selects trace ids, and a
trace is always kept when any span failed or it took at least one second.
Because a task has no invocation boundary, spans wait in memory until the
request's root span closes, and the whole request is exported or dropped
together. Children of a root still open after 60 seconds, such as a stream, are
decided without waiting for it. The exporter applies the same attribute
allowlist as the Lambda API (`apps/backend/aws/shared/span_export.rs`).
`OTEL_SERVICE_NAME` and `FLOW_LIKE_ENVIRONMENT` label both traces and metrics.

## Differences from the Lambda API

- The Aurora DSQL token rotates on a background timer instead of being checked
  on every request.
- No per-request compute-attempt rows are written: a task bills for its lifetime,
  not per request.
- Responses are bounded by the load balancer's timeouts rather than Lambda's
  15-minute limit.
- Handlers see the load balancer as the connection peer; the client address is
  in `X-Forwarded-For`.
