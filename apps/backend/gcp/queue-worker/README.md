# GCP Pub/Sub push queue worker

This image consumes one of four Flow-Like workloads from Pub/Sub: execution jobs
and compilation jobs (dispatched by the API), or released-content object events
for file tracking and media transformation (delivered by
`google_storage_notification`). Deploy one Cloud Run service per workload from
the same digest with separate service accounts.

The worker is a **push target**, not a puller. It serves `POST /` for Pub/Sub
push deliveries, `GET /health/live` for the Cloud Run liveness probe and
`GET /health/ready` (also `GET /health`) for readiness, and it holds no
Pub/Sub client at all: acking is `204`, nacking is `5xx`. That is what lets the
service scale to zero — a pull worker has to stay running to hold a StreamingPull
open, and a Cloud Run instance that stays running is billed for every second of
it.

## Environment contract

### Every workload

| Variable | Required | Notes |
|---|---|---|
| `GCP_QUEUE_WORKLOAD` | yes | `execution`, `compilation`, `file-tracking` or `media-transformation` |
| `GCP_PROJECT_ID` | yes | the app project |
| `PUBSUB_SUBSCRIPTION` | yes | the subscription id (`<name>`, what the Terraform root passes) or the full name `projects/<GCP_PROJECT_ID>/subscriptions/<name>`; a bare id is qualified with `GCP_PROJECT_ID`, a qualified name in any other project is refused; the full name is compared against every delivery's `subscription` field |
| `PUBSUB_PUSH_AUDIENCE` | no | when unset the expected `aud` is derived per request as `https://<Host>` — the URL Pub/Sub connected to, which is the subscription's `oidc_token.audience` **provided the root sets that audience equal to the push endpoint** (it does); set it only for a custom audience, then it must be that value verbatim: an `https://` URL, no query, fragment or trailing slash |
| `PUBSUB_PUSH_SERVICE_ACCOUNT` | yes | the `oidc_token.service_account_email` the subscription pushes as |
| `GCP_QUEUE_ACK_DEADLINE_SECS` | no (`600`) | range `10..600`; the subscription's `ack_deadline_seconds`, informational — a boot warning is logged when the process deadline exceeds it (see *Redelivery idempotency*) |
| `PORT` | no (`8080`) | Cloud Run sets this |
| `FIRESTORE_DATABASE` | no (`(default)`) | read by `flow-like-gcp-data` |
| `FIRESTORE_COLLECTION_PREFIX` | no | read by `flow-like-gcp-data` |
| `FIRESTORE_CLAIMS_COLLECTION` | no (`pubsub-claims`) | the redelivery claim collection |
| `GCP_QUEUE_MAX_DELIVERY_ATTEMPTS` | no (`3`) | range `1..100`; the point at which the worker stops doing work |
| `GCP_QUEUE_REQUEST_TIMEOUT_SECS` | no (`3600`) | must equal the Cloud Run service's `timeout_seconds` |
| `GCP_QUEUE_PROCESS_TIMEOUT_SECS` | no | default `min(EXECUTOR_TIMEOUT_SECS + 600, request timeout − 30)`; on the `execution` workload it must exceed `EXECUTOR_TIMEOUT_SECS` |
| `GCP_QUEUE_CLAIM_HEARTBEAT_SECS` | no (`30`) | range `5..600` |
| `GCP_QUEUE_CLAIM_STALE_AFTER_SECS` | no (`180`) | range `30..3600`; must be ≥ 3 × heartbeat |
| `GCP_QUEUE_CLAIM_RETENTION_SECS` | no (`86400`) | range `3600..604800`; must be > 2 × process timeout |

### `execution`

Adds the executor runtime configuration those packages already read:
`BACKEND_PUB`, `API_BASE_URL`, `EXECUTOR_TIMEOUT_SECS`,
`EXECUTOR_BATCH_INTERVAL_MS`, `EXECUTOR_MAX_BATCH_SIZE`,
`EXECUTOR_CALLBACK_TIMEOUT_MS`, `EXECUTOR_CALLBACK_RETRIES`,
`EXECUTOR_MAX_REMOTE_PAYLOAD_BYTES`, `EXECUTION_STATE_BACKEND`,
`ORT_GLOBAL_THREAD_POOL_DISABLE`, `OMP_NUM_THREADS`. **No `BACKEND_KEY`, no
database, no Secret Manager access** — the executor verifies a signed payload and
never mints one.

**`EXECUTOR_TIMEOUT_SECS` has to come down on this platform.** Container Apps let
the worker's deadline sit at `EXECUTOR_TIMEOUT_SECS + 600` because nothing above
it capped the request; Cloud Run terminates a request at `timeout_seconds`, which
caps at 3600. The worker's own deadline must fire at least 30 seconds before that
so it can release the claim rather than be killed holding it, which leaves the
executor run timeout below roughly 3540. Startup fails with an explicit message
if the two cannot both hold — including at the default `EXECUTOR_TIMEOUT_SECS` of
3600, which is unsatisfiable here by construction.

### `compilation`

Adds `BACKEND_PUB`, `BACKEND_KID`, `API_BASE_URL`, `COMPILER_TIMEOUT_SECS`,
`COMPILER_MAX_PARALLEL_TARGETS`, `COMPILER_CALLBACK_TIMEOUT_MS`,
`COMPILER_CALLBACK_RETRIES`, `COMPILER_STORAGE_TIMEOUT_SECS`,
`COMPILER_MAX_WASM_BYTES`, `COMPILER_MAX_ARTIFACT_BYTES`.
`storage.googleapis.com` is already in the
compiler's storage-host allowlist, so `COMPILER_ALLOWED_STORAGE_HOSTS` can stay
empty.

### `file-tracking`

Adds `GCP_CONTENT_BUCKET` (or the generic `CONTENT_BUCKET`),
`FIRESTORE_FILES_COLLECTION` (default `files`), and the Cloud SQL IAM settings
`flow-like-gcp-data` requires: `GCP_POSTGRES_AUTH_MODE=iam`,
`GCP_POSTGRES_HOST`, `GCP_POSTGRES_DATABASE`, `GCP_POSTGRES_USER`,
`GCP_POSTGRES_SERVER_CA`.

It keeps a size ledger per object in the `files` collection (`app_id` is the
indexed field that Cosmos carried as `pk`) and applies size deltas to
`App.totalSize` / `User.totalSize`. Its SQL role must exist and be allowed to
`UPDATE` both tables. Because the Cloud SQL IAM token cannot be refreshed inside
the pool, the process stops accepting deliveries when the token enters its safety
window, reports `503` on `/health/ready` and `/health`, and exits on its own
before expiry so Cloud Run starts an instance with a fresh token. Liveness
(`/health/live`) stays `200` throughout: the exit is already scheduled, and a
liveness restart would only risk killing a delivery still in flight. Refused
deliveries are nacks, so nothing is lost.

An `OBJECT_DELETE` carries the **same** generation as the `OBJECT_FINALIZE` that
wrote the ledger row (a generation belongs to the object version, not to the
event — unlike an Event Grid `sequencer`), so deletion staleness is *strict*: a
delete is ignored only when the row was written by a newer generation, i.e. the
object was overwritten first. A redelivered finalize with an equal generation is
still a no-op.

### `media-transformation`

Adds `GCP_CONTENT_BUCKET` (or `CONTENT_BUCKET`). It converts `media/` uploads to
WebP next to the original and deletes the original; `.webp` inputs are ignored,
videos are kept, other extensions are deleted (AWS parity).

### Forbidden environment

Startup fails, rather than silently ignoring, if any of these is set:
`GOOGLE_APPLICATION_CREDENTIALS`, `GOOGLE_APPLICATION_CREDENTIALS_JSON`,
`GOOGLE_CREDENTIALS`, `GOOGLE_OAUTH_ACCESS_TOKEN`, `CLOUDSDK_AUTH_ACCESS_TOKEN`,
`GCE_METADATA_HOST`, `GCE_METADATA_IP`, `GCE_METADATA_ROOT`,
`METADATA_SERVER_DETECTION`, `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (and the
lowercase spellings), `PUBSUB_EMULATOR_HOST`, `FIRESTORE_EMULATOR_HOST`,
`DATASTORE_EMULATOR_HOST`, `STORAGE_EMULATOR_HOST`,
`CLOUDSDK_API_ENDPOINT_OVERRIDES_PUBSUB`,
`CLOUDSDK_API_ENDPOINT_OVERRIDES_STORAGE`, `GOOGLE_STORAGE_CUSTOM_ENDPOINT`,
`GOOGLE_SERVICE_ACCOUNT`, `GOOGLE_SERVICE_ACCOUNT_PATH`,
`GOOGLE_SERVICE_ACCOUNT_KEY`, `SERVICE_ACCOUNT`. Ignoring one would leave an
operator believing an override took effect; honouring one would hand the
workload identity to whoever can write the environment. The credential half of
that list is enforced by `flow-like-gcp-data`, which every workload loads.

## IAM

Per workload service account, all of it keyless — no `google_service_account_key`
exists anywhere in this design.

| Workload | Roles |
|---|---|
| every workload | `roles/datastore.user` on the Firestore database (the redelivery claim) |
| `execution` | nothing further; it reaches the API over HTTPS with the JWT inside the signed payload |
| `compilation` | nothing further; staged jobs and artifacts are reached through signed URLs the API mints |
| `file-tracking` | `roles/cloudsql.client` + `roles/cloudsql.instanceUser` on the instance |
| `media-transformation` | `roles/storage.objectAdmin` on the **content** bucket only |

The subscription's own push identity needs `roles/run.invoker` on the worker
service, and Pub/Sub's service agent needs
`roles/iam.serviceAccountTokenCreator` on that identity so it can mint the OIDC
token. Neither belongs to the worker's service account.

The worker publishes to no topic. Dead-lettering is the subscription's
`dead_letter_policy`, which means `roles/pubsub.subscriber` on the subscription
and `roles/pubsub.publisher` on the dead-letter topic belong to the **Pub/Sub
service agent**, not to this workload.

## Cloud Run configuration

- `ingress = "INGRESS_TRAFFIC_INTERNAL_ONLY"`. Pub/Sub push originates inside
  Google's network, so the endpoint never needs to be internet-reachable.
- `max_instance_request_concurrency = 1`. One delivery per instance is what makes
  Pub/Sub's undelivered-message count a usable autoscaling signal, and it keeps a
  long execution from starving a short one on the same instance.
- `timeout_seconds` must equal `GCP_QUEUE_REQUEST_TIMEOUT_SECS`.
- The liveness probe hits `GET /health/live` (the contract shared with the
  gcp-api and gcp-executor images and pinned by `modules/gcp/runtime`); it
  answers `200` for the life of the process. A liveness path the router does not
  serve is not a skipped probe — Cloud Run counts the `404` as a failure and
  restarts the container every `failure_threshold × period_seconds`, and no
  delivery longer than that would ever complete. `/health/ready` and `/health`
  are readiness: `503` while the file tracker drains its database token.
- The push subscription's `oidc_token.audience` must equal its `push_endpoint`
  (the worker's own run.app URL) unless `PUBSUB_PUSH_AUDIENCE` is pinned to the
  custom value; the worker derives the expected `aud` from the request `Host`
  when it is not pinned.

## Security and delivery behaviour

- **Every delivery is authenticated before its body is parsed.** The
  `Authorization: Bearer` token must be RS256-signed by a key in Google's
  published JWKS, carry `iss` `https://accounts.google.com`, carry `aud` exactly
  equal to the expected audience (`PUBSUB_PUSH_AUDIENCE` when pinned, otherwise
  `https://<Host>` of the request — the URL Pub/Sub connected to, which Cloud
  Run's own invoker check has already matched against the service URL before
  the container saw the request), and carry a verified `email` exactly equal to
  `PUBSUB_PUSH_SERVICE_ACCOUNT`. Any one of those alone is insufficient: the
  audience is choosable by any Google identity, and the signature says nothing
  about which service the token was minted for. The key set is fetched at
  startup, cached for an hour, and refreshed on an unknown `kid` at most once a
  minute so an unknown-key flood cannot drive this worker's egress. A key-set
  outage returns `503`, not `401`, so a Google incident is distinguishable from
  an intrusion attempt in the response-code metrics.
- The delivery's `subscription` field is compared against the canonical
  `projects/<GCP_PROJECT_ID>/subscriptions/<name>` derived from
  `PUBSUB_SUBSCRIPTION`. The token proves Pub/Sub sent it; this proves it came
  from the topic this worker was built to drain, in this project.
- Push bodies above 14 MiB are rejected by the router before allocation, and a
  decoded message body above 10 MiB — Pub/Sub's own publish limit — is treated as
  permanently invalid. Anything larger is staged to Cloud Storage by the producer
  and arrives as a claim-check reference, which this worker resolves.
- Pub/Sub mints its own `messageId`, so the signed job identity travels in the
  versioned dispatch envelope (`v`, `job_id`, `payload`, with `v == 1` enforced)
  and is compared against the resolved payload's `job_id` exactly as the broker
  message ID used to be on Service Bus.
- Object-event workloads read `eventType`, `bucketId`, `objectId` and
  `objectGeneration` from the message **attributes** and `size`/`etag` from the
  JSON API v1 object resource in the body, so the notification must be created
  with `payload_format = "JSON_API_V1"`. Only `OBJECT_FINALIZE` and
  `OBJECT_DELETE` are accepted. `generation` replaces Event Grid's `sequencer`
  as the ordering token, and because it is a decimal integer the staleness
  comparison is numeric rather than lexicographic — read as text, `"9"` outranks
  `"10"`. Because a generation belongs to the object version rather than to the
  event, a delete carries the generation the finalize carried: finalize
  staleness is `>=` (a redelivery is a no-op), delete staleness is strictly `>`
  (only a row already overwritten by a newer version ignores the delete).
- The file tracker never deletes a stored object to undo a failed accounting
  write. Deleting one would publish another notification into this very
  subscription and the worker would be feeding itself. Only the ledger row is
  rolled back, under a compare-and-swap on the `updateTime` this delivery
  produced, so a newer event that landed in between keeps its value.
- Failures are classified as `Permanent` or `Retryable` with stable codes.
  `Retryable` nacks with `503`; `Permanent` writes a terminal `Failed` run status
  and then nacks with `500` so the subscription's `dead_letter_policy` carries the
  message — and its attributes — into the dead-letter topic, where the GCS sink
  preserves it. Acking a permanent failure would destroy the only forensic copy.
- `GCP_QUEUE_MAX_DELIVERY_ATTEMPTS` (default `3`, matching the AWS
  `maxReceiveCount` and the Azure dequeue cap) is where the worker stops doing
  work. It is deliberately **not** where the worker stops nacking: Pub/Sub refuses
  a `dead_letter_policy` below five attempts, so acking at three would starve the
  policy and the message would never reach the dead-letter topic. The worker
  therefore keeps nacking until the delivery count passes
  `max(ceiling, 5)`, at which point the message is either already in the
  dead-letter topic or has no policy to take it, and further redelivery buys
  nothing.
- The delivery count is Pub/Sub's `deliveryAttempt` when the subscription has a
  dead-letter policy, and the claim document's own counter when it does not —
  Pub/Sub omits `deliveryAttempt` entirely in that case. The claim counter is the
  direct analogue of Azure's `DequeueCount`, and it is what makes the ceiling
  behave identically with and without a policy.
- Every dead-letter decision logs `pubsub_message_dead_lettered` at ERROR, and
  the terminal `Failed` run status is written exactly once per message no matter
  how many times it is redelivered.
- This is at-least-once processing. Callback and run persistence remain the final
  idempotency boundary when a settle fails after completed work.

## Redelivery idempotency

A push subscription's ack deadline caps at **600 seconds**; a Cloud Run request
runs for up to **3600**. There is no `modifyAckDeadline` on the push path,
because the worker never holds an `ackId`. So any execution longer than ten
minutes *will* be redelivered while the first attempt is still running, and
without a guard one forty-minute flow becomes four concurrent flows and the size
ledger is counted four times.

The guard is a Firestore **claim document**, keyed by
`blake3(subscription ‖ 0x1f ‖ messageId)` in the `pubsub-claims` collection:

1. Every delivery first writes the claim with a **create-if-absent**
   precondition. Firestore resolves concurrent creates of the same document name
   server-side, so exactly one delivery is ever told `Applied` — that is the
   entire mutual exclusion.
2. The winner heartbeats the claim every `GCP_QUEUE_CLAIM_HEARTBEAT_SECS` from
   inside the same `select!` that runs the work, never from a spawned task. The
   document's server-owned `updateTime` is the concurrency token, and it behaves
   exactly like the Azure pop receipt it replaces: it rotates on every successful
   write and the previous value dies with it. Losing that compare-and-swap means
   what a lost pop receipt meant — another replica owns this message now — so the
   worker drops the work and settles nothing.
3. Every other delivery of the same `messageId` reads the claim instead of doing
   the work:
   - **in progress and heartbeating** → **park, never ack.** This is the
     ack-deadline gap itself, and it is also what a redelivery sees after the
     owner was OOM-killed, SIGKILLed or restarted a few seconds ago. Acking here
     would be fatal in the second case: acking any live `ackId` removes the
     message, so an owner that dies afterwards takes the message with it — no
     redelivery, no dead-letter copy, a run left `Running` until its TTL. On
     AWS and Azure the visibility timeout guarantees the message comes back;
     here the claim has to. The redelivery therefore polls the claim (every
     `min(GCP_QUEUE_CLAIM_HEARTBEAT_SECS, 15)` s) for at most
     `min(GCP_QUEUE_CLAIM_STALE_AFTER_SECS, process timeout)`: the moment the
     owner settles or its heartbeat ages past the staleness window it re-enters
     step 1, which recognises the duplicate or takes the message over on *this*
     delivery. An owner still heartbeating when the budget ends is nacked
     (`owner_in_flight`) so the broker asks again later. Nothing acks on the
     strength of somebody else's heartbeat.
   - **completed** → ack. The duplicate is exactly that.
   - **dead-lettered** → nack (or ack once the delivery count passes the
     dead-letter window), without repeating the work or the terminal-status write.
   - **released, or in progress with a stale heartbeat** → take the claim over
     under a compare-and-swap on the `updateTime` that was read, incrementing the
     delivery count. This is what recovers a message whose owner was killed
     mid-flight; the compare-and-swap is what stops two replicas from recovering
     it at once.
4. A retryable failure marks the claim `released` rather than deleting it, so the
   delivery counter survives into the next attempt. A deleted claim would be
   recreated with `attempts = 1` and the ceiling would never be reached. A claim
   guard that goes out of scope *without* being settled — a panic inside a
   catalog node, a cancelled task — releases the claim from `Drop` with the code
   `guard_dropped` (best effort, on a spawned task), so the next redelivery takes
   it over at once instead of waiting for the heartbeat to go stale.
5. If Firestore cannot be reached, the delivery is nacked. The claim is the only
   thing between a redelivery and a duplicate run, so an unreachable claim store
   is a hard stop rather than a warning to work through.

Two configuration invariants keep this sound and are enforced at startup:
`GCP_QUEUE_PROCESS_TIMEOUT_SECS` must leave 30 seconds inside
`GCP_QUEUE_REQUEST_TIMEOUT_SECS`, so the worker's own deadline fires before Cloud
Run kills the request and the claim is always released by the process that took
it; and `GCP_QUEUE_CLAIM_RETENTION_SECS` must exceed twice the process timeout,
so the claim's TTL cannot collect a duplicate's evidence while the original is
still running.

**What parking costs, and what it still does not cover.** Every nack of a parked
redelivery counts toward the subscription's `max_delivery_attempts` (floor 5),
exactly as an ack-deadline expiry does. With the shipped root configuration
(request timeout 600 = ack deadline 600, process deadline 570) a run cannot
outlive its delivery, so this never fires for a healthy owner. If an operator
raises `GCP_QUEUE_REQUEST_TIMEOUT_SECS` above the ack deadline — the worker logs
a warning at boot when the process deadline exceeds `GCP_QUEUE_ACK_DEADLINE_SECS`
— then a legitimately long run accumulates one broker attempt per
`ack deadline + park budget + backoff`, and the broker may copy the message to
the dead-letter topic while the owner is still working (a spurious DLQ copy; the
owner still writes the terminal run status and the claim still de-duplicates the
DLQ-driven redelivery). Either accept that, or raise the subscription's
`max_delivery_attempts` to cover `request_timeout / ack_deadline` extra
attempts and keep `GCP_QUEUE_MAX_DELIVERY_ATTEMPTS` as the work ceiling. Each
parked redelivery also holds one Cloud Run request slot for up to the park
budget, so size `max_instance_request_concurrency` accordingly. Residual: an
owner killed by SIGKILL/OOM between heartbeats is recovered only after
`GCP_QUEUE_CLAIM_STALE_AFTER_SECS` (the `Drop` release cannot run), and a
process forced down by the database-token hard stop before its graceful
`worker_shutdown` release lands is recovered the same way.
