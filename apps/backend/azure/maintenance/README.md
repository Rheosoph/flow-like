# Azure maintenance job

This image is the Azure counterpart of the AWS maintenance Lambda: a stateless
Container Apps Job that runs once per execution and exits. It holds no database
credentials and can invoke only the maintenance jobs allowlisted by the
Flow-Like API (`POST /api/v1/maintenance/run`, service-authenticated with the
shared `MAINTENANCE_TOKEN`). The Terraform root schedules it daily
(`0 3 * * *`), gives it a 1800 s replica timeout and up to three replica
retries, and injects the token from Key Vault; the job needs no Azure SDK, no
queue and no Cosmos/PostgreSQL access.

## Configuration

- `API_BASE_URL`: base URL of the Flow-Like API. `API_URL` is accepted as a
  fallback because the Azure root sets `API_URL=https://api` on the job;
  `API_BASE_URL` wins when both are present. Blank values count as unset.
- `MAINTENANCE_TOKEN`: the API's maintenance token, at least 32 bytes after
  trimming. Seed it once in Key Vault (`scripts/generate-maintenance-token.sh`)
  and let the job reference the secret; never bake it into the image or the
  Terraform variables file.
- `MAINTENANCE_JOB`: `telemetry_alerts`, `cache_cleanup` or `all` (default and
  the image's baked-in value). `all` runs telemetry alerts first, then cache
  cleanup, as two independent requests. Any other value fails startup.
- `ALLOW_INSECURE_API_BASE_URL`: `1`/`true` permits an `http://` base URL for
  trusted development only. It has the same meaning as on AWS and does not
  touch TLS verification: `https://` endpoints are always verified against the
  bundled Mozilla roots and there is deliberately no override for that. If the
  bare `https://api` hostname does not validate in your environment, point
  `API_BASE_URL` at the app's environment FQDN instead.
- `CONTAINER_APP_JOB_EXECUTION_NAME`: injected by Container Apps; see
  idempotency below. Nothing needs to set it by hand.
- `RUST_LOG` (default `info`) controls tracing output.

`API_BASE_URL` must be an absolute URL without credentials, query or fragment.
The API deployment that is driven by this job should set
`FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1` (and `CACHE_SWEEPER_DISABLED=1` if the
in-process sweeper is not wanted); otherwise its background loops duplicate the
scheduled work. Transactional rule-row locks still keep overlapping API
replicas, manual runs and job retries from emitting the same alert transition
twice.

## Requests and idempotency

Every selected job is one request:

```http
POST /api/v1/maintenance/run
Authorization: Bearer <MAINTENANCE_TOKEN>
Idempotency-Key: telemetry_alerts:<execution-name-or-minute>
Content-Type: application/json

{"job":"telemetry_alerts"}
```

The `Idempotency-Key` is `<job>:<CONTAINER_APP_JOB_EXECUTION_NAME>` inside
Container Apps: every replica retry of the same execution reuses it, while the
next scheduled or manually started execution gets a fresh one. Outside Container
Apps (no execution name) it is `<job>:<UTC minute the run started>`, e.g.
`cache_cleanup:2026-07-27T03:00:00Z`, so retries within the scheduled minute
dedup and the next day's run is a new key. The minute is sampled once per run;
the second job of `all` never rolls into a new key because the first was slow.
The API currently uses the key for correlation only, and the jobs are safe to
repeat: alert transitions are transactional and cache cleanup is a sweep.

## Exit status and retries

- Exit `0` only when every requested job returned a matching `2xx`
  `MaintenanceRunResponse` (`evaluated/triggered/resolved` or `deleted` counts
  are logged at INFO).
- Any request error, non-`2xx` status, unparsable body, or a response for a
  different job than requested fails that job; in `all` mode the remaining job
  still runs and the process exits `1` at the end. HTTP `408`, `429` and `5xx`
  are logged as transient, other `4xx` as deployment/configuration errors, so
  the execution history in Container Apps tells the two apart.
- Startup validation failures (missing or short token, bad URL, unknown
  `MAINTENANCE_JOB`) exit `1` before any request is sent.

The Container Apps retry policy (`replica_retry_limit`) re-runs the replica on
non-zero exit; a failed execution stays visible in the job's execution history
and the alert rules built on it. Per-request timeout is 120 s (5 s connect), so
even `all` finishes well inside the replica timeout.

## Running a job by hand

Start an ad-hoc execution and override the selection without touching the
schedule:

```sh
az containerapp job start \
  --name maintenance --resource-group <name_prefix>-rg \
  --env-vars MAINTENANCE_JOB=cache_cleanup
```

Locally, `docker run` the image with `API_BASE_URL`, `MAINTENANCE_TOKEN` and
optionally `MAINTENANCE_JOB`; the key falls back to the minute-scoped form.
