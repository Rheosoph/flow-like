# GCP scheduler tick

This image is the per-minute Cloud Run **job** that dispatches cron sinks on
GCP. The API's `SINK_SCHEDULER_PROVIDER` is unset on this cloud, so nothing
inside the API fires schedules — Cloud Scheduler starts one execution of this
job every minute, the execution runs exactly one *tick* and exits, and that
tick is the only cron dispatcher in the deployment.

The binary is thin on purpose. The algorithm, the cron parsing, the API client
and the claim store trait live in `packages/scheduler-tick`
(`flow-like-scheduler-tick`); this crate builds it with the `gcp` feature,
guards the GCP environment, pins the state backend to Firestore and maps the
tick report to an exit code. `apps/backend/azure/scheduler` is the same binary
over the Cosmos store.

## What one tick does

1. `GET {API_BASE_URL}/api/v1/sink/schedules` with `Authorization: Bearer
   $SINK_TRIGGER_JWT` — every active cron sink, across all apps.
2. For each enabled schedule, read its state document from the Firestore
   `scheduler` collection (`{ id: <event_id>, app_id, cron_expression,
   last_fired_at, updated_at }`). A missing document means "fire from this
   minute forward": the window opens 60 s before `now`, never further back.
3. Compute the cron occurrences in `(last_fired_at, now]`, capped by
   `SCHEDULER_MAX_CATCHUP_SECS` so an outage replays at most ten minutes of
   backlog, and by a per-schedule occurrence cap of 30 so an every-second
   expression cannot flood the API.
4. **Claim first**: compare-and-swap `last_fired_at = now` conditioned on the
   document's `updateTime` read in step 2 (create-if-absent when there was
   none). Losing the CAS means another execution owns this schedule right now
   — skip it. This is what makes an overlapping tick, a slow tick or a Cloud
   Run retry safe: two executions that both see the same occurrence race on
   one write and exactly one of them fires.
5. `POST {API_BASE_URL}/api/v1/sink/trigger/async` once per occurrence with
   `Idempotency-Key: cron:{event_id}:{occurrence_rfc3339}` and body
   `{ "event_id", "sink_type": "cron", "payload": { "scheduled_for" } }`. The
   API caches the response under that key for ~15 minutes per replica; the
   CAS is the cross-execution guard, the key is belt-and-braces.
6. A trigger failure after the claim is logged and counted, the remaining
   occurrences still fire, and the job exits non-zero. The claim is **not**
   rolled back: that would replay the whole window and double-fire the
   occurrences that succeeded.

Schedules are processed 16 at a time and the whole tick runs under a wall-clock
deadline (`SCHEDULER_TICK_DEADLINE_SECS`, default 200 s) that must sit inside
the job's `task_timeout` (240 s in Terraform). Cloud Run has no way to tell the
process its own timeout, so the deadline is the operator's responsibility: a
tick Cloud Run kills leaves no report and no exit code, only a failed
execution.

## Environment contract

| Variable | Required | Notes |
|---|---|---|
| `API_BASE_URL` | yes | absolute HTTPS origin of the API; `API_URL` is accepted as a fallback, `API_BASE_URL` wins when both are set. No credentials, query or fragment; a trailing slash is stripped |
| `SINK_TRIGGER_JWT` | yes | pre-minted HS256 token with `sub=sink-trigger`, `iss=flow-like`, `sink_types=["cron"]`, a `jti`. Inject as a Secret Manager reference (`secret_env`), never as a plain env value. Startup rejects anything that is not three non-empty base64url segments (a pasted `Bearer ` prefix, a trailing newline) so the mistake surfaces at boot rather than as a 401 every minute |
| `SCHEDULER_STATE_BACKEND` | yes | must be `firestore`. The image sets it; a Terraform value naming another backend fails startup |
| `GCP_PROJECT_ID` | yes | read by `flow-like-gcp-data` for the Firestore document root |
| `FIRESTORE_DATABASE` | no (`(default)`) | the state database that holds the `scheduler` collection (`modules/gcp/state`) |
| `FIRESTORE_COLLECTION_PREFIX` | no | applied by the Firestore client to every collection id it is handed, including the one below |
| `FIRESTORE_SCHEDULER_COLLECTION` | no (`scheduler`) | the **unprefixed** collection id — the client prepends `FIRESTORE_COLLECTION_PREFIX` itself. With a non-empty prefix, pass `scheduler`, not the already-prefixed `module.state.collections["scheduler"]` |
| `FIRESTORE_MAX_RETRIES` | no (`8`) | range `0..20`; read by `flow-like-gcp-data` for retryable Firestore statuses |
| `SCHEDULER_MAX_CATCHUP_SECS` | no (`600`) | range `60..86400`; how far behind `now` a window may open after an outage |
| `SCHEDULER_TICK_DEADLINE_SECS` | no (`200`) | range `1..3600`; keep it below the job's `task_timeout` |
| `ALLOW_INSECURE_API_BASE_URL` | no | `1`/`true` lets a plain-`http://` `API_BASE_URL` through and drops the client's HTTPS-only transport guard. Never set on GCP: the job reaches the API over the public hostname, which is the only name that resolves to a certificate the client accepts |
| `RUST_LOG` | no (`info`) | |

Fan-out (16) and the per-schedule occurrence cap (30) are not operator-tunable;
both are sized against the API's per-replica trigger queue rather than against
any deployment property.

`CLOUD_RUN_EXECUTION` and `CLOUD_RUN_TASK_ATTEMPT`, which Cloud Run sets on
every job execution, are logged on the start line so a retried attempt's fires
can be told apart from the first attempt's.

### Forbidden

Startup fails when any of these is present, **even with an empty value** —
the same posture as `gcp-api` and `gcp-queue-worker`:

```text
GOOGLE_APPLICATION_CREDENTIALS      GOOGLE_APPLICATION_CREDENTIALS_JSON
GOOGLE_CREDENTIALS                  GOOGLE_OAUTH_ACCESS_TOKEN
CLOUDSDK_AUTH_ACCESS_TOKEN          GCE_METADATA_HOST
GCE_METADATA_IP                     GCE_METADATA_ROOT
METADATA_SERVER_DETECTION           HTTP_PROXY / HTTPS_PROXY / ALL_PROXY (+ lowercase)
                                    (flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env)

FIRESTORE_EMULATOR_HOST             DATASTORE_EMULATOR_HOST
CLOUDSDK_API_ENDPOINT_OVERRIDES_*   GOOGLE_*_CUSTOM_ENDPOINT
                                    (the Firestore client refuses its own; the families are matched by shape)

SINK_SECRET                         (this job holds the *token*, never the key that signs it)
```

The proxy variables matter twice here: `reqwest` honours them, so the bearer
JWT would travel through whatever they name on its way to the API, and the
metadata-server token would do the same on its way to Firestore. `SINK_SECRET`
is refused because a job that holds the signing key could mint a token for any
sink type and any app; its presence means the job was handed the API's
environment by mistake.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | every claimed occurrence fired; lost claims and rejected expressions are steady-state outcomes and do not fail the run |
| `1` | the tick ran and needs an operator: a store read/claim failure, a trigger POST that failed after the claim, a schedule listing that failed, or the deadline was exceeded |
| `2` | startup refused: forbidden environment, wrong `SCHEDULER_STATE_BACKEND`, missing or malformed `API_BASE_URL` / `SINK_TRIGGER_JWT`, Firestore client could not be built |

A `1` surfaces in Cloud Run job history and the observability module's
alert policies; the failed occurrences are named in the log at ERROR. Because
the claim advanced, the next minute's tick does not replay them — see step 6
above for why.

The job's `max_retries = 2` is safe: a retried execution runs a fresh tick
against the same state, so anything the failed attempt claimed is skipped and
anything it did not claim is picked up.

## Identity and access

The job runs as the `scheduler` service account and needs:

- `roles/datastore.user` on the state database (or project) — the claim is a
  create/replace with a precondition on the `scheduler` collection.
- `roles/secretmanager.secretAccessor` on the `SINK_TRIGGER_JWT` secret, held by
  the scheduler identity only. The API never reads this secret; it verifies the
  token against `SINK_SECRET`.
- Nothing on the API's Cloud Run service for the HTTP hop itself: the token in
  `Authorization` is the whole authentication, and the request goes through the
  public load balancer like any other client.

There is no key file, no `DATABASE_URL`, no Pub/Sub, no Cloud Storage. The
Firestore token comes from the instance metadata server for the job's own
service account and nothing else.

## Minting `SINK_TRIGGER_JWT`

```sh
export SINK_SECRET="<the API's SINK_SECRET>"
export TF_VAR_sink_trigger_jwt="$(./scripts/generate-sink-trigger-jwt.sh)"
```

`scripts/generate-sink-trigger-jwt.sh` (deployment repo root) mints the token
offline with `openssl`: HS256 over `SINK_SECRET`, claims
`{"sub":"sink-trigger","iss":"flow-like","jti":"sink_cron_<24 hex>","sink_types":["cron"],"iat":<now>}`,
no `exp` — the API validates without expiry and revokes by `jti`. The script
writes the token to stdout only and the `jti` to stderr; record the `jti` so
the token can be revoked later. Rotation can also go through
`POST /admin/sinks/register`, which additionally records the `jti` for
revocation.

## Running it by hand

```sh
gcloud run jobs execute <job name> --region <region> --wait
```

The image cannot run outside GCP: it refuses every credential variable that
would let it authenticate as anything other than the metadata server's identity,
and that is the point.
