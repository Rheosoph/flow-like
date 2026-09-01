# GCP maintenance job

This image is the Cloud Run Jobs port of the AWS maintenance Lambda. It is a
stateless Cloud Scheduler target: it holds no database credential and no GCP
credential at all, runs once per execution, POSTs the allowlisted maintenance
jobs to the Flow-Like API with a bearer token, and exits. A non-zero exit is the
failure signal — Cloud Run retries the task, the execution history records the
outcome, and the log-match alert policies pick it up.

The logic is `apps/backend/aws/maintenance` with the Lambda runtime removed:
the same URL validation, the same minimum token length, the same transient /
configuration classification of HTTP failures, and the same job / response
mismatch check.

## Environment contract

| Variable | Required | Notes |
|---|---|---|
| `API_BASE_URL` | yes | absolute `https://` URL of the API, no credentials, no query, no fragment; a trailing slash is stripped. `API_URL` is accepted as an alias; `API_BASE_URL` wins when both are set |
| `MAINTENANCE_TOKEN` | yes | at least 32 bytes after trimming; the same value the API holds. Injected from Secret Manager (`secret_env`), never as plain env |
| `MAINTENANCE_JOB` | no (`all`) | `telemetry_alerts`, `cache_cleanup`, `run_sweep`, `state_cleanup` or `all`; case-insensitive. `all` runs all four in that order and exits non-zero if any failed |
| `ALLOW_INSECURE_API_BASE_URL` | no | `1` or `true` permits an `http://` base URL. Only for trusted development or private networking; the token is a bearer credential |
| `CLOUD_RUN_EXECUTION` | set by Cloud Run | the execution name; used as the idempotency-key suffix (see below). Not something to set by hand |

Rejected at startup — startup fails rather than ignoring them — is everything
`flow_like_gcp_data::metadata::FORBIDDEN_CREDENTIAL_SETTINGS` lists: key-file
and ambient-token variables (`GOOGLE_APPLICATION_CREDENTIALS`,
`GOOGLE_OAUTH_ACCESS_TOKEN`, `CLOUDSDK_AUTH_ACCESS_TOKEN`, …), metadata-server
overrides (`GCE_METADATA_HOST`, …), and every proxy variable (`HTTPS_PROXY`,
`ALL_PROXY`, lower-case variants). The job never mints a GCP token, but a proxy
would route `MAINTENANCE_TOKEN` through whoever set it, and the guard is what
keeps this image from ever becoming a credential holder. The HTTP client is
additionally built with `no_proxy()` so its behaviour does not depend on the
guard.

## What it sends

For every selected job:

```http
POST /api/v1/maintenance/run
Authorization: Bearer <MAINTENANCE_TOKEN>
Idempotency-Key: <job>:<suffix>
Content-Type: application/json

{"job":"telemetry_alerts"}
```

The `Idempotency-Key` suffix is the Cloud Run execution name when
`CLOUD_RUN_EXECUTION` is present, i.e. always on Cloud Run Jobs. It is stable
across task retries within one execution and different for every fresh
execution, so a retried attempt after a transient failure repeats the key while
a Cloud Scheduler retry or a manual `gcloud run jobs execute` gets a new one.
Outside Cloud Run (local runs, other schedulers) the suffix is `now` floored to
the minute in RFC 3339 UTC — `telemetry_alerts:2026-08-15T03:00:00Z` — which
dedups an in-minute retry and makes the next day's run a new key. A
`CLOUD_RUN_EXECUTION` value that is not a plausible execution name (anything
outside `[A-Za-z0-9._-]`) is logged and ignored, and the minute key is used.

The API uses `Idempotency-Key` for log correlation, not as a durable dedup
ledger; the jobs are idempotent on their own. Telemetry alert transitions are
row-locked and transactional, cache cleanup deletes what has already expired,
and run reconciliation conditionally updates only stale non-terminal rows. A
repeat is safe. Future jobs must either be inherently idempotent or add
durable idempotency storage before being enabled here.

The `run_sweep` request reads `RUN_SWEEPER_GRACE_SECS` and
`RUN_SWEEPER_BATCH_SIZE` from the API deployment. The batch defaults to 500 and
is capped at 900. A daily `all` schedule provides only daily reconciliation.
Use a separate, more frequent `run_sweep` schedule when stuck runs must become
terminal sooner. Set the grace period above the longest legitimate queue delay
plus `EXECUTOR_TIMEOUT_SECS`; otherwise the sweep can classify a live run as
stale. The job reconciles the canonical SQL run row only. It does not mutate a
separately configured execution state backend.

## Failure semantics

- Startup validation failure (missing or short token, invalid URL, unknown job,
  forbidden environment): exits non-zero before any request is made.
- Transport failure, non-`2xx` response, unparseable success body, or a response
  for a different job than requested: that job is failed and logged. `408`, `429`
  and `5xx` are logged as transient; other `4xx` as deployment / configuration
  errors. Both classes fail the task — Cloud Run's `max_retries` governs retries
  of either, and the retry repeats the same idempotency key.
- With `MAINTENANCE_JOB=all`, a failed job does not stop the next one: every
  selected job runs, then the process exits non-zero if any failed. The retry
  re-issues all three keys; a job that already succeeded is a safe repeat.
- Each request has a 5-second connect timeout and a 300-second overall timeout,
  matching the API service's Cloud Run `timeout_seconds`. Three sequential jobs
  therefore finish inside roughly 915 seconds, well under the job's
  `task_timeout` of 1800 seconds. Raise the client timeout together with the
  API's request timeout, never independently.

There is no in-process retry loop: Cloud Run's task retry (`max_retries`) is
the retry, and the execution-scoped idempotency key is what makes it safe.

## Deployment

The GCP root (`deployments/gcp/dev/main.tf`) builds this image as the
`maintenance` target of `module.images`, runs it as the `maintenance` Cloud Run
job with `MAINTENANCE_TOKEN` from Secret Manager and `API_BASE_URL` / `API_URL`
set to the public API hostname, and drives it daily at 03:00 UTC through Cloud
Scheduler (`module.scheduler`, `retry_count = 1`). The API deployment it drives
sets `FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1`, so this job is the only telemetry
alert evaluator; the API's row locks remain the correctness boundary if two
evaluations ever overlap.

Cloud Scheduler has no dead-letter target (see `docs/gcp/architecture.md`, D7):
delivery evidence for a Scheduler → job invocation exists only in the logs.

## Local run

```sh
docker build -f apps/backend/gcp/maintenance/Dockerfile -t flow-like-gcp-maintenance .
docker run --rm \
  -e API_BASE_URL=https://api.example.com \
  -e MAINTENANCE_TOKEN="$(openssl rand -base64 48)" \
  -e MAINTENANCE_JOB=telemetry_alerts \
  flow-like-gcp-maintenance
```

Build the image from the `flow-like` workspace root: the Dockerfile copies the
whole workspace so `cargo build --locked` resolves against the committed
`Cargo.lock`. Without `CLOUD_RUN_EXECUTION` the idempotency key falls back to
the scheduled minute.
