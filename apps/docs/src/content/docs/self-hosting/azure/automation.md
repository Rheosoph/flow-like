---
title: Azure scheduling and maintenance
description: Operate cron sink dispatch and maintenance as separate Container Apps Jobs.
sidebar:
  order: 30
---

Run cron dispatch every minute as a Container Apps Job. Run maintenance in a
separate job at the cadence required for alert evaluation and cleanup. Each
execution performs one pass and exits, so job history and nonzero exit codes
are the main failure signals.

## Cron dispatcher

Leave `SINK_SCHEDULER_PROVIDER` unset on the API when this job owns scheduling.
Configure the scheduler job with `cron_expression = "* * * * *"` and a
240-second replica timeout. Its own deadline defaults to 200 seconds and must
remain below the replica timeout.

| Variable | Value |
| --- | --- |
| `API_BASE_URL` | HTTPS API origin; `API_URL` is a fallback and `API_BASE_URL` wins |
| `SINK_TRIGGER_JWT` | Cron-scoped compact JWT, injected from Key Vault |
| `SCHEDULER_STATE_BACKEND` | Exactly `cosmos` |
| `COSMOS_ENDPOINT` | HTTPS account endpoint, resolved through private DNS |
| `COSMOS_AUTH_MODE` | Exactly `managed_identity` |
| `AZURE_CLIENT_ID` | Scheduler identity client UUID |
| `IDENTITY_ENDPOINT`, `IDENTITY_HEADER` | Supplied by Container Apps; the endpoint must be HTTP on loopback when present |
| `COSMOS_DATABASE` | Optional; the Cosmos client's default applies when unset |
| `COSMOS_SCHEDULER_CONTAINER` | Default `scheduler` |
| `SCHEDULER_MAX_CATCHUP_SECS` | Default `600`, range `60..86400` |
| `SCHEDULER_TICK_DEADLINE_SECS` | Default `200`, range `1..3600` |
| `RUST_LOG` | Default `info` |

Use the API's environment FQDN with a certificate trusted by public roots.
A bare internal hostname may fail TLS verification. The development-only
`ALLOW_INSECURE_API_BASE_URL=1` permits HTTP; it does not change HTTPS
certificate verification. The scheduler rejects static Azure credentials,
storage endpoint/emulator overrides, alternate identity endpoints, and proxy
variables, even when empty. See its
[configuration guard](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/azure/scheduler/src/config.rs).

Grant the scheduler identity `Cosmos DB Built-in Data Contributor` scoped to
the scheduler container, provisioned with partition key `/app_id` and no TTL.
The bearer token authenticates requests to the API. Register a cron token
through the admin-authenticated `POST /api/v1/admin/sinks/register` endpoint
with this body:

```json
{"sink_type":"cron","name":"Azure cron scheduler"}
```

Store the returned `token` in Key Vault and retain its `jti` for revocation.
The token uses HS256 and contains `sub=sink-trigger`, `iss=flow-like`, and
`sink_types=["cron"]`. Supply only the compact token, without a `Bearer `
prefix or whitespace. It must contain at least 32 bytes. The scheduler does
not need the API's signing key. The
[registration handler](https://github.com/Rheosoph/flow-like/blob/main/packages/api/src/routes/admin/sinks/register_sink.rs)
defines the request and response.

### Catch-up and failed occurrences

One tick lists active schedules at `GET /api/v1/sink/schedules`, reads each
Cosmos state item, and computes occurrences in `(last_fired_at, now]`. A new
item opens its window 60 seconds before now. Catch-up is bounded by
`SCHEDULER_MAX_CATCHUP_SECS`; the shared scheduler processes up to 16 schedules
concurrently and caps each schedule at 30 occurrences.

Before triggering a schedule, the job advances `last_fired_at` under an
`If-Match` check on the previously read `_etag`, or creates the missing item
conditionally. A lost claim skips the schedule. The winner calls
`POST /api/v1/sink/trigger/async` for each occurrence with
`Idempotency-Key: cron:<event_id>:<occurrence_rfc3339>`.

The conditional state update prevents overlapping jobs from claiming the same
window. API idempotency keys have a per-replica cache of approximately 15
minutes. A failed trigger after the claim is logged and makes the job fail,
but the claim stays advanced to avoid replaying successful occurrences. Inspect
the ERROR logs and handle missed occurrences individually; the next tick and
automatic retries do not replay the claimed window. Deleting a scheduler state
item resets that schedule to firing from the current minute.

Exit `0` means every claimed occurrence was accepted. Lost claims and invalid
or over-cap expressions do not fail the job. Exit `1` covers startup,
listing, store, trigger, or deadline errors. Configure bounded replica retries
and monitor failed executions. Build from the repository root with
`docker buildx build -f apps/backend/azure/scheduler/Dockerfile .`.

## Maintenance job

The maintenance image holds no database credentials. It sends authenticated
requests to `POST /api/v1/maintenance/run` for these selections:

| `MAINTENANCE_JOB` | Work |
| --- | --- |
| `telemetry_alerts` | Evaluate alert rules |
| `cache_cleanup` | Sweep expired cache entries |
| `run_sweep` | Reconcile stale non-terminal SQL runs |
| `state_cleanup` | Delete expired execution state and old staged payloads |
| `payments` | Recover pending payments, refunds, webhooks and effects |
| `all` | Default; run all five as independent requests |

Set `API_BASE_URL` (or fallback `API_URL`) to an absolute HTTPS URL without
credentials, query, or fragment. Inject `MAINTENANCE_TOKEN` from Key Vault with
the same value on the API, at least 32 bytes after trimming. Generate a value
with `openssl rand -base64 48` when first provisioning the shared secret.
`ALLOW_INSECURE_API_BASE_URL=1` or `true` permits HTTP for trusted development.
HTTPS always uses certificate verification.

Set `FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1` on the API when this job owns alert
evaluation, and `CACHE_SWEEPER_DISABLED=1` if scheduled cleanup should replace
the API's cache loop. A daily `all` job only reconciles runs daily. Use a more
frequent separate `run_sweep` job when stuck runs must become terminal sooner.
The API's `RUN_SWEEPER_BATCH_SIZE` defaults to 500 and is capped at 900. Set
`RUN_SWEEPER_GRACE_SECS` above the longest legitimate queue delay plus
`EXECUTOR_TIMEOUT_SECS`; reconciliation updates SQL run rows, leaving separately
configured execution state backends unchanged.

When payment servicing is enabled, schedule a separate `MAINTENANCE_JOB=payments`
execution at least once per minute. A daily `all` job does not provide timely
payment recovery when the API has no continuous CPU. See
[Payments rollout and recovery](/dev/platform-administration/#payments-rollout-and-recovery)
for configuration and alert requirements. Provider sandbox validation remains a
rollout prerequisite.

The idempotency key is `<job>:<CONTAINER_APP_JOB_EXECUTION_NAME>`, stable across
replica retries. Outside Container Apps it uses the UTC minute sampled when the
run started. The API uses this key for correlation; transactional alert updates
and conditional cleanup/sweeps make repeated requests safe.

Each request has a five-second connect timeout and a 120-second overall
timeout. A replica timeout of 1800 seconds leaves room for all five requests.
Exit `0` requires a matching successful response from every selected job.
Transport errors, non-2xx responses, invalid response bodies, and mismatched job
responses fail the execution. In `all` mode later jobs still run before exit
`1`. Logs classify 408, 429, and 5xx as transient and other 4xx as configuration
errors; both classes use the Container Apps replica retry policy.

Start a manual execution of the configured job with:

```sh
az containerapp job start --name <maintenance-job> --resource-group <resource-group>
```

For a local invocation, provide the existing API token in the environment and
pass `API_BASE_URL`, `MAINTENANCE_TOKEN`, and optionally `MAINTENANCE_JOB` to
the maintenance container. A newly generated token only works after the API is
configured with that same value.
