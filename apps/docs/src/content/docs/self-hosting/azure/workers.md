---
title: Azure execution and queues
description: Configure synchronous execution and Queue Storage consumers, including lease recovery and poison queues.
sidebar:
  order: 20
---

Use the HTTP executor for synchronous API requests and Queue Storage workers
for asynchronous execution, compilation, file tracking, and media processing.
The same queue-worker image serves all four workloads; deploy each workload as
a separate Container App with its own user-assigned identity and queue.

## HTTP executor

Set `EXECUTION_BACKEND=http` and `EXECUTOR_URL` on the API. The executor accepts
signed execution requests whose bodies contain the presigned URLs and scoped
runtime credentials needed by the run. The process opens no database or queue
client and requests no managed-identity token for normal execution.

| Variable | Required or default | Purpose |
| --- | --- | --- |
| `EXECUTOR_SERVER_MODE` | Required: `true` or `1`, baked into the image | This image only supports server mode |
| `BACKEND_PUB` | Required | Standard base64 of the API's P-256 public PEM; validated at startup without trimming |
| `PORT` | `8080` | Execution routes and health |
| `METRICS_PORT` | `9090` | Metrics listener; must differ from `PORT` |
| `EXECUTOR_TIMEOUT_SECS` | `3600` | Run timeout |
| `EXECUTOR_BATCH_INTERVAL_MS` | `1000` | Callback batching interval |
| `EXECUTOR_MAX_BATCH_SIZE` | `100` | Events per callback batch |
| `EXECUTOR_CALLBACK_TIMEOUT_MS` | `5000` | Callback request timeout |
| `EXECUTOR_CALLBACK_RETRIES` | `3` | Callback retry count |
| `API_BASE_URL` | Optional | Model-call fallback when no signed run context exists |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Optional | OTLP/gRPC trace export; invalid exporter initialization fails startup |
| `RUST_LOG` | `info` | Log filtering |

Routes on `PORT` are `POST /execute`, `POST /execute/stream` for NDJSON,
`POST /execute/sse`, and `GET /health`. Probe `/health`; collect `/metrics`
on `METRICS_PORT`. Normal hosted-model and remote-embedding calls use the
signed callback URL as their API proxy base. Provider credentials stay on the
API. The server catalog excludes ONNX Runtime, local model weights, and desktop
automation nodes.

Execution JWT verification needs `BACKEND_PUB`; this deployment does not rely
on an `API_URL` JWKS fallback. Shared non-secret Azure resource names can be
present in the executor environment, but process-wide storage keys, SAS tokens,
client secrets, client-certificate credentials, connection strings, emulator
flags, and custom storage endpoints are rejected. Flow nodes run in-process
and can read the environment. See the exact
[executor guard](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/azure/executor/src/config.rs).

Metrics include matched-route request counts and duration, execution counts,
duration, and active jobs. Streaming handlers return at the first event, so
their handler-duration metrics measure time to the first event; use API run
records for completed run duration. SIGTERM closes the listeners and drains
active runs up to the Container App's termination grace period.

Build from the repository root:

```sh
docker buildx build -f apps/backend/azure/executor/Dockerfile .
```

## Queue identity and configuration

For each queue-worker identity, scope these roles to its own queue:

- `Storage Queue Data Message Processor` for receive and delete.
- `Storage Queue Data Reader` for KEDA queue-length reads.
- A custom role granting
  `Microsoft.Storage/storageAccounts/queueServices/queues/messages/write` for
  visibility renewal. Message Processor alone does not grant it.
- `Storage Queue Data Message Sender` on the queue's `-poison` sibling.

Set `AZURE_QUEUE_WORKLOAD` to `execution`, `compilation`, `file-tracking`, or
`media-transformation`; set `AZURE_QUEUE_STORAGE_ACCOUNT_NAME`,
`AZURE_QUEUE_NAME`, `AZURE_QUEUE_POISON_NAME`, and `AZURE_CLIENT_ID`.
The poison name must equal `<AZURE_QUEUE_NAME>-poison`. Inject the workload's
executor/compiler runtime values from the existing secret configuration.

Only Entra managed-identity authentication is supported. Static storage
credentials fail startup. Traffic uses
`https://<account>.queue.core.windows.net`; configure private DNS so this resolves
to the account's queue private endpoint.

| Setting | Default | Constraint |
| --- | --- | --- |
| `AZURE_QUEUE_VISIBILITY_TIMEOUT_SECS` | `300` | `60..604800` |
| `AZURE_QUEUE_RENEWAL_INTERVAL_SECS` | `60` | `10..3600`, below half the visibility timeout |
| `AZURE_QUEUE_PROCESS_TIMEOUT_SECS` | Executor timeout plus `600` | Must exceed the executor run timeout |
| `AZURE_QUEUE_MAX_DEQUEUE_COUNT` | `3` | `1..100` |
| `AZURE_QUEUE_BATCH_SIZE` | `1` | Pinned until waiting-message renewal is implemented |
| `AZURE_QUEUE_POLL_MIN_INTERVAL_SECS` | `1` | `1..60` |
| `AZURE_QUEUE_POLL_MAX_INTERVAL_SECS` | `30` | `1..300` |

Queue Storage has no long poll. The worker uses bounded polling backoff while
the queue is empty; KEDA can scale the Container App to zero.

### File and media workloads

Deliver released-content Event Grid events into the corresponding queues.
The decoder accepts CloudEvents 1.0 or Event Grid schema as raw or
base64-encoded JSON.

File tracking requires `AZURE_CONTENT_CONTAINER`, `COSMOS_ENDPOINT`,
`COSMOS_DATABASE`, `COSMOS_AUTH_MODE`, and optionally `COSMOS_FILES_CONTAINER`
(default `files`). It also needs the
[managed-identity PostgreSQL settings](/self-hosting/azure/deployment/#postgresql-identity-and-lifecycle)
for its own SQL identity, with permission to update `App.totalSize` and
`User.totalSize`. It stores per-blob size state in Cosmos, partitioned by app ID,
and applies deltas to SQL. The worker drains and exits before its database
token expires; keep it restartable.

Media transformation requires `AZURE_STORAGE_ACCOUNT_NAME` and
`AZURE_CONTENT_CONTAINER`, with `Storage Blob Data Contributor` on the content
container. It converts image uploads under `media/` to WebP beside the original,
then deletes the original. WebP inputs are ignored, videos are retained, and
unsupported extensions are deleted. Scope event delivery accordingly.

## Lease loss and poison recovery

A message remains invisible while its lease is renewed. Every renewal rotates
the pop receipt used to delete or move it. A failed renewal aborts the work
without settling and logs `queue_lease_lost`; the message becomes visible again
when its lease expires.

Queue Storage has no broker dead-letter policy. Before starting work, the
worker checks `DequeueCount`. Exhausted or permanently invalid messages move to
`<queue>-poison`: the worker writes the poison copy, with a reason in its body,
before deleting the original. Monitor `queue_message_poisoned` at ERROR and
inspect the poison queue before replaying a corrected message.

Decoded inline bodies are capped at 48 KiB. Larger payloads must arrive as a
reference to data staged in Blob Storage. Dispatch envelopes carry `v`,
`job_id`, and `payload`; the signed job ID must agree with the resolved payload.
Delivery is at least once, so a settle failure after completed work can cause a
repeat. Callback and run persistence provide the final idempotency boundary.
The [worker implementation](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/azure/queue-worker/src/main.rs)
defines decoding, renewal, and poison handling.
