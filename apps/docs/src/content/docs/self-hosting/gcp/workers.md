---
title: GCP execution and queues
description: Configure the Cloud Run executor and Pub/Sub push workers, including deadlines, claims, and dead-letter recovery.
sidebar:
  order: 20
---

The API uses an HTTP executor for synchronous runs and Pub/Sub push services
for asynchronous workloads. Give each service its own identity and keep the
request timeout aligned with the process timeout. A Cloud Run request that
ends before the worker's own deadline can interrupt final callbacks or leave a
claim waiting for recovery.

## Synchronous HTTP executor

Deploy `apps/backend/gcp/executor` as a Cloud Run service with internal/load
balancer ingress and the API service account as its only invoker. Keep its URL
out of the public load balancer's URL map. Configure
`EXECUTION_BACKEND=http`, `EXECUTOR_URL`, and `EXECUTOR_AUTH=gcp_id_token` on
the API. Every request includes a signed execution payload with presigned URLs
and scoped runtime credentials; progress uses the callback URL in the
execution JWT.

| Variable | Default or requirement |
| --- | --- |
| `BACKEND_PUB` | Required: standard base64 of the API's ES256 public-key PEM; no whitespace trimming |
| `EXECUTOR_SERVER_MODE` | Image default `true`; unset is accepted, but any set value other than case-insensitive `1`/`true` is rejected |
| `PORT` | Cloud Run supplies it; image default `8080`. Do not also declare it in the service environment map |
| `METRICS_PORT` | Forbidden; all routes use `PORT` |
| `EXECUTOR_TIMEOUT_SECS` | Image default `3540`, accepted range `1..3570`; leave at least 30 seconds inside the actual service request timeout |
| `EXECUTOR_BATCH_INTERVAL_MS` | `1000` |
| `EXECUTOR_MAX_BATCH_SIZE` | `100` |
| `EXECUTOR_CALLBACK_TIMEOUT_MS` | `5000` |
| `EXECUTOR_CALLBACK_RETRIES` | `3` |
| `API_BASE_URL` | Optional fallback for model calls without a run context |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Optional OTLP/gRPC exporter |
| `GCP_REQUIRE_OTEL` | `true` makes a missing exporter endpoint fatal |
| `RUST_LOG` | Image default `info` |

All executor tuning values must parse. The process knows the platform ceiling
of 3600 seconds, but cannot discover a lower service `timeout_seconds`; lower
`EXECUTOR_TIMEOUT_SECS` when you lower that service setting. Keep CPU allocated
outside request handling (`cpu_idle = false`) so final callback batches can flush.

| Route | Behavior |
| --- | --- |
| `POST /execute` | Run to completion with callback events |
| `POST /execute/stream` | NDJSON event stream |
| `POST /execute/sse` | Server-sent event stream |
| `GET /health`, `GET /health/live` | Process liveness |
| `GET /health/ready` | Startup/readiness; returns 503 while draining after SIGTERM |
| `GET /metrics` | Prometheus registry on the serving port |

Use `/health/ready` for the startup probe and `/health/live` for liveness.
SIGTERM closes readiness and drains immediately within the platform's
termination grace. There is no job-once mode or database/queue connection in
this binary. Normal remote-model calls use the signed callback URL as their
API proxy base, and provider credentials stay on the API. The server catalog
excludes ONNX Runtime and local model weights.

The executor rejects the API's
[forbidden GCP credential and endpoint sources](/self-hosting/gcp/deployment/#rejected-credential-sources),
even though it normally needs no Google client. Flow nodes and credential
fallbacks can read process environment settings. `BACKEND_PUB` is validated
at startup; this deployment does not rely on a JWKS fallback through `API_URL`.

The request middleware records `http_requests_total{route,method,status}` with
matched routes to bound label cardinality. It does not record a duration for
streaming handlers that return before the run ends; use API run records for
execution duration. Build from the repository root with
`docker buildx build -f apps/backend/gcp/executor/Dockerfile .`.

## Pub/Sub push worker configuration

Deploy one Cloud Run service for each `GCP_QUEUE_WORKLOAD`: `execution`,
`compilation`, `file-tracking`, or `media-transformation`. All use the same
queue-worker digest. The worker serves `POST /` for deliveries, acknowledges
with 204, and negatively acknowledges with 5xx. It can scale to zero because
Pub/Sub initiates each request.

| Variable | Default or requirement |
| --- | --- |
| `GCP_QUEUE_WORKLOAD` | Required workload name |
| `GCP_PROJECT_ID` | Required application project |
| `PUBSUB_SUBSCRIPTION` | Required subscription ID or `projects/<GCP_PROJECT_ID>/subscriptions/<name>`; a different project is rejected |
| `PUBSUB_PUSH_SERVICE_ACCOUNT` | Required email of the subscription's push identity |
| `PUBSUB_PUSH_AUDIENCE` | Optional exact HTTPS audience, without query, fragment, or trailing slash; otherwise derived as `https://<Host>` per request |
| `GCP_QUEUE_ACK_DEADLINE_SECS` | `600`, range `10..600`; set to the subscription's acknowledgement deadline |
| `GCP_QUEUE_REQUEST_TIMEOUT_SECS` | `3600`, range `60..3600`; must equal the service's `timeout_seconds` |
| `GCP_QUEUE_PROCESS_TIMEOUT_SECS` | `min(EXECUTOR_TIMEOUT_SECS + 600, request timeout - 30)`; must exceed the workload's executor/compiler timeout |
| `GCP_QUEUE_MAX_DELIVERY_ATTEMPTS` | `3`, range `1..100`; limits attempts that perform work |
| `GCP_QUEUE_CLAIM_HEARTBEAT_SECS` | `30`, range `5..600` |
| `GCP_QUEUE_CLAIM_STALE_AFTER_SECS` | `180`, range `30..3600`, at least three heartbeat intervals |
| `GCP_QUEUE_CLAIM_RETENTION_SECS` | `86400`, range `3600..604800`, greater than twice the process timeout |
| `FIRESTORE_DATABASE` | `(default)` |
| `FIRESTORE_COLLECTION_PREFIX` | Optional collection prefix |
| `FIRESTORE_CLAIMS_COLLECTION` | `pubsub-claims` |
| `PORT` | Cloud Run supplies it; default `8080` |

The process deadline must leave at least 30 seconds for claim release before
Cloud Run ends the request. The library's 3600-second executor default cannot
fit that contract; set a lower `EXECUTOR_TIMEOUT_SECS`. Compilation must also
leave its inner timeout below the worker deadline. Startup rejects impossible
combinations.

Configure internal-only ingress and align the subscription's
`oidc_token.audience` with its `push_endpoint`, unless a custom
`PUBSUB_PUSH_AUDIENCE` is explicitly configured. Start with
`max_instance_request_concurrency = 1`; account for parked redeliveries before
raising timeouts beyond the acknowledgement deadline.

Use `/health/live` for liveness and `/health/ready` or `/health` for readiness.
File tracking returns 503 on readiness while its SQL token drains, but liveness
stays 200 so a probe does not interrupt the planned drain. Refused deliveries
are negatively acknowledged for later delivery.

### Workload-specific settings

Execution uses `BACKEND_PUB`, `API_BASE_URL`, the `EXECUTOR_*` settings above,
`EXECUTOR_MAX_REMOTE_PAYLOAD_BYTES`, and the configured execution state backend.
It does not need `BACKEND_KEY`, a database credential, or Secret Manager access.

Compilation uses `BACKEND_PUB`, `BACKEND_KID`, `API_BASE_URL`,
`COMPILER_TIMEOUT_SECS`, `COMPILER_MAX_PARALLEL_TARGETS`,
`COMPILER_CALLBACK_TIMEOUT_MS`, `COMPILER_CALLBACK_RETRIES`,
`COMPILER_STORAGE_TIMEOUT_SECS`, `COMPILER_MAX_WASM_BYTES`, and
`COMPILER_MAX_ARTIFACT_BYTES`. `storage.googleapis.com` is already permitted
by the compiler's storage host allowlist.

File tracking requires `GCP_CONTENT_BUCKET` (or `CONTENT_BUCKET`), optionally
`FIRESTORE_FILES_COLLECTION` (default `files`), and its own
[Cloud SQL IAM settings](/self-hosting/gcp/deployment/#database-bootstrap-and-token-rotation).
Its database user needs update rights on `App` and `User`. It stores an
object-size ledger in Firestore and applies deltas to SQL totals. The process
stops accepting work, drains, and exits before its database token expires.

Media transformation requires `GCP_CONTENT_BUCKET` (or `CONTENT_BUCKET`). It
converts image uploads under `media/` to WebP beside the original, then deletes
the original. WebP inputs are ignored, videos are retained, and unsupported
extensions are deleted. Scope object-event delivery accordingly.

### Identity grants

| Identity | Required access |
| --- | --- |
| Every worker | `roles/datastore.user` for Firestore claims |
| Execution and compilation workers | Signed payload/URL access; no additional cloud grant for normal run/artifact delivery |
| File-tracking worker | `roles/cloudsql.client` and `roles/cloudsql.instanceUser`, plus its SQL grants |
| Media-transformation worker | `roles/storage.objectAdmin` on the content bucket |
| Subscription push identity | `roles/run.invoker` on the target worker service |
| Pub/Sub service agent | `roles/iam.serviceAccountTokenCreator` on the push identity |
| Pub/Sub service agent for dead-lettering | `roles/pubsub.subscriber` on the source subscription and `roles/pubsub.publisher` on the dead-letter topic |

The worker publishes to no Pub/Sub topic. Keep push authentication and
dead-letter grants on the Pub/Sub identities that perform those operations.
Static credentials, metadata/proxy overrides, emulator settings, and the
worker's custom Pub/Sub/Storage endpoint settings fail startup, including
empty values. The exact list is in the
[worker entrypoint](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/gcp/queue-worker/src/main.rs).

## Delivery validation and object events

Each delivery is authenticated before its body is parsed. The Google-signed
RS256 token must have issuer `https://accounts.google.com` or
`accounts.google.com`, the expected audience, and a verified email matching
`PUBSUB_PUSH_SERVICE_ACCOUNT`. The
delivery's `subscription` must match the configured canonical subscription
name. The signing-key cache loads at startup, lasts an hour, and refreshes an
unknown key at most once a minute. A key-service outage returns 503.

The router caps push envelopes at 14 MiB and decoded message data at 10 MiB.
Larger jobs must use a reference to staged Cloud Storage data. Versioned
dispatch envelopes require `v = 1`, `job_id`, and `payload`; the job ID must
match the resolved signed payload.

For storage notifications, configure `payload_format = "JSON_API_V1"` and
deliver `OBJECT_FINALIZE` and `OBJECT_DELETE`. The worker reads `eventType`,
`bucketId`, `objectId`, and `objectGeneration` from message attributes and
`size`/`etag` from the object-resource body. Generation comparisons are numeric.
An equal-generation finalize is a duplicate, while an equal-generation delete
must still remove that version's accounting contribution. Only a newer stored
generation makes that delete stale.

An accounting failure rolls back the ledger under a Firestore version
precondition. It does not delete a stored object as compensation, which would
publish another notification. A newer event's ledger update survives rollback.

## Claims, redelivery, and dead-letter handling

Push acknowledgement deadlines are capped at 600 seconds. A longer execution
can be redelivered while its owner is still running. Each delivery therefore
uses a Firestore claim keyed by the subscription and Pub/Sub message ID. The
first conditional create owns the work and heartbeats its claim. Losing the
claim's `updateTime` precondition aborts the work without settling it.

| Claim state seen by a redelivery | Worker response |
| --- | --- |
| Active owner with a fresh heartbeat | Wait for completion or staleness, then recheck; never acknowledge solely because another owner is active |
| Completed | Acknowledge the duplicate |
| Released or stale owner | Attempt takeover with a version precondition and increment the attempt count |
| Dead-lettered | Skip work and terminal-status writes; continue negative acknowledgements through the dead-letter window |
| Firestore unavailable | Negatively acknowledge; do not run without a claim |

An active-owner delivery waits for at most
`min(GCP_QUEUE_CLAIM_STALE_AFTER_SECS, process timeout)`, polling every
`min(GCP_QUEUE_CLAIM_HEARTBEAT_SECS, 15)` seconds. If the owner is still active
when that budget ends, it returns `owner_in_flight` for retry. Retryable errors
release the claim while retaining the attempt counter. A dropped guard attempts
a best-effort release (`guard_dropped`); an OOM or SIGKILL can only recover after
the heartbeat becomes stale.

Retryable failures return 503. Permanent failures return 500 within the
dead-letter window so the subscription can preserve the message and attributes
in its dead-letter topic. For execution jobs with a resolvable signed payload,
the worker also attempts to record terminal `Failed` state. That write is
best-effort: unreadable payloads or failed callbacks can leave the run
nonterminal. Check the run record and callback state during recovery.

The worker stops doing work at `GCP_QUEUE_MAX_DELIVERY_ATTEMPTS`, but continues
negative acknowledgements until attempts exceed `max(work ceiling, 5)`. It then
returns 204 and acknowledges the message without confirming dead-letter
delivery. It uses Pub/Sub's `deliveryAttempt` when present and the claim counter
otherwise. Monitor `pubsub_message_dead_lettered` at ERROR and configure a
retained dead-letter topic/sink before relying on this recovery path. A broker
delivery limit beyond the worker's acknowledgement cutoff can lose the recovery
copy; keep the broker policy compatible with that cutoff and verify routing.

Keep request and process deadlines within the acknowledgement deadline when
possible; for example, a 600-second request with a 570-second process deadline
avoids healthy work outliving its first delivery. With longer requests, parked
deliveries consume request slots and broker attempts. A broker may create a
dead-letter copy while the owner still works. Size the subscription's delivery
limit for the additional acknowledgement windows within the worker's finite
acknowledgement window, and account for parked requests in concurrency planning.

Delivery remains at least once. Callback and run persistence are the final
boundary when settlement fails after completed work; claim ownership alone
does not make external side effects exactly once. See the
[claim implementation](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/gcp/queue-worker/src/claim.rs)
for takeover and release behavior.
