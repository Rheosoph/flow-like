# Azure scheduler tick

This image is the cron dispatcher for Flow-Like on Azure. The API's own
`SINK_SCHEDULER_PROVIDER` is unset on Azure, so the API only *lists* cron sinks;
this job, run every minute as a Container Apps job (`cron_expression =
"* * * * *"`, `replica_timeout_in_seconds = 240`), is the only thing that fires
them. Each execution is one tick and exits; nothing loops in-process.

The tick algorithm, cron parsing, sink-trigger API client and the Cosmos state
store live in the shared `flow-like-scheduler-tick` crate
(`packages/scheduler-tick`, built here with `features = ["azure"]`). This crate
is only the Azure config guard around it.

## What one tick does

1. `GET {API_BASE_URL}/api/v1/sink/schedules` with the cron-scoped
   `SINK_TRIGGER_JWT`; disabled schedules are dropped.
2. For each schedule, read its state item from the `scheduler` Cosmos container
   (`id` = event id, partition key `/app_id`). Missing item → the window opens
   60 s before `now` (a new schedule fires from this minute on, never back-fills).
3. Window `(last_fired_at, now]`, clamped to `SCHEDULER_MAX_CATCHUP_SECS`
   (default 600) so an outage does not replay hours; compute the cron
   occurrences inside it (5/6/7-field expressions are normalised).
4. **Claim first**: replace the item with `last_fired_at = now` under `If-Match`
   on the `_etag` read in step 2 (create-if-absent when there was none). A
   precondition failure or create conflict means another tick owns the schedule
   right now → skip it at debug. This is what makes overlapping or duplicated
   executions safe.
5. `POST /api/v1/sink/trigger/async` once per occurrence with
   `Idempotency-Key: cron:{event_id}:{occurrence_rfc3339}`. The API dedups the
   key for ~15 min per replica; the CAS is the cross-tick guard.
6. A fire failure after the claim is logged and counted, the remaining
   occurrences still fire, and the job exits non-zero. The claim is never rolled
   back — that would replay the whole window and double-fire the successes.

Sixteen schedules are processed concurrently; the whole run is bounded by
`SCHEDULER_TICK_DEADLINE_SECS` (default 200, inside the 240 s job timeout) and
exits non-zero if it is exceeded.

## Environment

Required:

- `API_BASE_URL` (or `API_URL`; `API_BASE_URL` wins): HTTPS origin of the API.
  Plain HTTP is refused unless `ALLOW_INSECURE_API_BASE_URL=1`, a development
  override only. The certificate must chain to a public root; the image ships
  no private CA, so point this at the API's environment FQDN, not a bare
  internal hostname.
- `SINK_TRIGGER_JWT`: pre-minted HS256 token (`sub=sink-trigger`,
  `iss=flow-like`, `sink_types=["cron"]`, `jti`), at least 32 bytes and a
  compact three-segment JWT. Mint it offline with
  `scripts/generate-sink-trigger-jwt.sh` and inject it from Key Vault
  (`SINK-TRIGGER-JWT`); it is never logged.
- `SCHEDULER_STATE_BACKEND=cosmos`: must be exactly this — the binary refuses
  any other value rather than write cron state where no other job reads it.
- `COSMOS_ENDPOINT`: HTTPS account endpoint (private endpoint resolves it).
- `COSMOS_AUTH_MODE=managed_identity`: pinned; `auto`, `workload_identity`
  and `developer_tools` are refused in this image.
- `AZURE_CLIENT_ID`: UUID client ID of the scheduler's user-assigned identity.
- `IDENTITY_ENDPOINT` / `IDENTITY_HEADER`: injected by Container Apps. If
  present, `IDENTITY_ENDPOINT` must be an HTTP loopback URL.

Optional:

- `COSMOS_DATABASE` (client default applies when unset)
- `COSMOS_SCHEDULER_CONTAINER` (default `scheduler`)
- `SCHEDULER_MAX_CATCHUP_SECS` (default `600`, range `60..86400`)
- `SCHEDULER_TICK_DEADLINE_SECS` (default `200`, range `1..3600`)
- `RUST_LOG` (default `info`)

Forbidden — presence, even empty, fails startup:

- the Azure API's list (`apps/backend/azure/api/src/config.rs`): storage account
  keys, SAS tokens, `AZURE_CLIENT_SECRET`, ACS keys and connection strings,
  emulator switches, `AZURE_STORAGE_ENDPOINT` / `AZURE_ENDPOINT` /
  `AZURITE_BLOB_STORAGE_URL`, `AZURE_USE_AZURE_CLI`, signature-skip flags
- managed-identity source overrides and proxies (`MSI_ENDPOINT`, `MSI_SECRET`,
  `IMDS_ENDPOINT`, `IDENTITY_SERVER_THUMBPRINT`, `AZURE_AUTHORITY_HOST`,
  `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` and lowercase)

## Identity and state

The scheduler identity needs `Cosmos DB Built-in Data Contributor` scoped to
the `scheduler` container only (`modules/azure/state` grants exactly that). The
container is provisioned by Terraform with partition key `/app_id` and no TTL;
items look like

```json
{ "id": "<event_id>", "app_id": "<app_id>", "cron_expression": "* * * * *",
  "last_fired_at": "2026-08-15T12:05:30Z", "updated_at": "2026-08-15T12:05:30Z" }
```

`last_fired_at` is the watermark the claim advances; deleting an item resets
that schedule to "fire from now on".

The API is reached only with the bearer JWT. No Cosmos key, storage key or
client secret exists in this process, and only Entra managed identity is
implemented for Cosmos.

## Exit codes

- `0`: every claimed occurrence was accepted by the API (lost claims and
  unparseable or over-cap expressions are steady-state outcomes and do not fail
  the run).
- `1`: configuration or startup error, the schedule listing failed, any store
  or fire error after a claim, or the deadline was exceeded. Container Apps
  records the execution as failed; the job's `replica_retry_limit` re-runs it,
  and the re-run continues from the claims already written — failed occurrences
  are not replayed.

## Build

```sh
docker buildx build -f apps/backend/azure/scheduler/Dockerfile .
```

Context is the `flow-like` workspace root; there is no config secret in this
image.
