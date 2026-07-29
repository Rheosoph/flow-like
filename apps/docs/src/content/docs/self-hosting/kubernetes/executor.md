---
title: Executor
description: Run the Kubernetes executor as a warm HTTP worker pool
sidebar:
  order: 70
---

The checked-in Kubernetes executor has two entrypoint modes, but only its
long-lived server mode currently executes workflows.

Source:

- `apps/backend/kubernetes/executor/src/main.rs`
- `packages/executor/`

## Current mode status

| Mode | Selection | Status |
| --- | --- | --- |
| HTTP server | `EXECUTOR_SERVER_MODE=true` | Implemented and used by the Helm executor pool |
| One-job process | Default when the variable is absent or false | Placeholder; logs an error and exits with status `1` |

:::caution
The API contains a `kubernetes_job` dispatcher that can create a Job, but the
executor image's one-job entrypoint does not consume that Job's environment or
run the board yet. Do not configure `EXECUTION_BACKEND=kubernetes_job` with the
checked-in executor image and expect successful executions.
:::

## Server mode

The Helm chart's `executorPool` Deployment sets
`EXECUTOR_SERVER_MODE=true` and exposes the executor through a ClusterIP
Service. The API normally uses:

```dotenv
EXECUTION_BACKEND=http
EXECUTOR_URL=http://flow-like-executor-pool:8080
```

The actual Service name includes the Helm release/fullname prefix.

Server mode provides:

| Endpoint | Purpose |
| --- | --- |
| `POST /execute` | Execute and return a final JSON response |
| `POST /execute/stream` | Stream newline-delimited JSON events |
| `POST /execute/sse` | Stream Server-Sent Events |
| `GET /health` | Executor health |
| `GET /metrics` | Prometheus metrics |

The application port defaults to `8080`. A second metrics listener defaults to
`9090` and exposes `/metrics` as well.

```dotenv
EXECUTOR_SERVER_MODE=true
PORT=8080
METRICS_PORT=9090
```

Executor behavior is configured by:

| Variable | Default | Purpose |
| --- | --- | --- |
| `EXECUTOR_BATCH_INTERVAL_MS` | `1000` | Callback event batching interval |
| `EXECUTOR_MAX_BATCH_SIZE` | `100` | Events per callback batch |
| `EXECUTOR_CALLBACK_TIMEOUT_MS` | `5000` | Callback request timeout |
| `EXECUTOR_CALLBACK_RETRIES` | `3` | Callback retry count |
| `EXECUTOR_TIMEOUT_SECS` | `3600` | Workflow execution timeout |

## Request contract

The shared executor contract is `ExecutionRequest` in
`packages/executor/src/types.rs`. It includes the application and board,
payload, scoped credentials, execution JWT, callback information, board
version, and optional WASM package references.

The API builds this request. Operators should not hand-construct it from a few
environment variables: the credentials and JWT are part of the trust boundary.

During a run, the executor:

1. verifies the execution JWT;
2. constructs request-scoped Flow-Like state from the supplied credentials;
3. loads and prepares the requested board;
4. overlays verified WASM package artifacts when present;
5. executes the board;
6. returns or streams events and reports callbacks as required.

The warm pool caches prepared board data, but request-specific state and logic
are stripped before cache insertion and reattached per run.

## One-job mode

`run_job_once` in `apps/backend/kubernetes/executor/src/main.rs` currently
contains only an implementation outline. It does not read `RUN_ID`, `APP_ID`,
`BOARD_ID`, `FLOW_LIKE_CREDENTIALS`, `FLOW_LIKE_JWT`,
`FLOW_LIKE_CALLBACK_URL`, or `PAYLOAD`, even though the API's Kubernetes
dispatcher places those values in created Jobs.

The process deliberately exits with status `1` and directs operators to use the
executor pool.

## Local debugging

Run the implemented server mode:

```bash
cd apps/backend/kubernetes/executor
EXECUTOR_SERVER_MODE=true cargo run
```

Then check:

```bash
curl http://localhost:8080/health
curl http://localhost:9090/metrics
```

An `/execute` test additionally requires a valid `ExecutionRequest`, signed JWT,
and reachable backing services. The normal API dispatch path is the safest way
to exercise that contract.

## Related

- [Execution Backends](/self-hosting/execution-backends/)
- [Kubernetes Installation](/self-hosting/kubernetes/installation/)
- [Kubernetes Monitoring](/self-hosting/kubernetes/monitoring/)
