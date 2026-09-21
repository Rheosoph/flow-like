---
title: GCP scheduling and maintenance
description: Operate cron dispatch and API maintenance through Cloud Scheduler and Cloud Run Jobs.
sidebar:
  order: 30
---

Cloud Scheduler starts Cloud Run Jobs for cron dispatch and maintenance. Each
job runs once and exits. Monitor both Scheduler delivery logs and Cloud Run
execution outcomes: a successful request to start a job does not prove that
the job's API calls succeeded.

## Cron dispatcher

Run the scheduler job every minute and leave `SINK_SCHEDULER_PROVIDER` unset on
the API. The job lists cron sinks, claims their due windows in Firestore, and
triggers occurrences through the API. Give it a 240-second task timeout with
the process deadline below it (default 200 seconds).

| Variable | Value |
| --- | --- |
| `API_BASE_URL` | HTTPS API origin without credentials, query, or fragment; `API_URL` is a fallback and `API_BASE_URL` wins |
| `SINK_TRIGGER_JWT` | Required compact cron-scoped token from Secret Manager |
| `SCHEDULER_STATE_BACKEND` | Exactly `firestore`, set by the image |
| `GCP_PROJECT_ID` | Required application project |
| `FIRESTORE_DATABASE` | Default `(default)` |
| `FIRESTORE_COLLECTION_PREFIX` | Optional prefix applied by the client |
| `FIRESTORE_SCHEDULER_COLLECTION` | Default `scheduler`; supply the unprefixed collection ID |
| `FIRESTORE_MAX_RETRIES` | Default `8`, range `0..20` |
| `SCHEDULER_MAX_CATCHUP_SECS` | Default `600`, range `60..86400` |
| `SCHEDULER_TICK_DEADLINE_SECS` | Default `200`, range `1..3600`; below the actual task timeout |
| `RUST_LOG` | Default `info` |

The client adds `FIRESTORE_COLLECTION_PREFIX` itself. Passing an already
prefixed scheduler collection name would direct claims to a different
collection. Keep the API URL on its certificate-matching HTTPS hostname.
The development override `ALLOW_INSECURE_API_BASE_URL=1` or `true` permits
HTTP; leave it unset for this cloud deployment.

The scheduler identity needs `roles/datastore.user` for scheduler state and
`roles/secretmanager.secretAccessor` on the one token secret. API calls use
the bearer token through the public API endpoint; they need no executor
invoker, Pub/Sub, storage, or database grant. Firestore credentials come from
the job's metadata-server identity.

The job rejects ambient Google credentials, metadata endpoint overrides, and
proxy settings from the
[shared credential guard](https://github.com/Rheosoph/flow-like/blob/main/packages/gcp-data/src/metadata.rs).
It also rejects Firestore/Datastore emulator variables, endpoint-override
families `CLOUDSDK_API_ENDPOINT_OVERRIDES_*` and `GOOGLE_*_CUSTOM_ENDPOINT`,
and `SINK_SECRET`, even if empty. Give the job the scoped token only.

### Register and rotate the token

As an API administrator, call `POST /api/v1/admin/sinks/register` with:

```json
{"sink_type":"cron","name":"GCP cron scheduler"}
```

Store the returned `token` in Secret Manager and retain its `jti` for
revocation. The HS256 token has `sub=sink-trigger`, `iss=flow-like`, and
`sink_types=["cron"]`. It is long-lived and registered for revocation; manage
rotation through the admin sink-token endpoints. Inject the compact token
without a `Bearer ` prefix or surrounding whitespace. The
[registration handler](https://github.com/Rheosoph/flow-like/blob/main/packages/api/src/routes/admin/sinks/register_sink.rs)
defines this interface.

### Claims and missed occurrences

One tick calls `GET /api/v1/sink/schedules`, reads each enabled schedule's
Firestore state, and computes due occurrences in `(last_fired_at, now]`.
A missing state document opens its window 60 seconds before now. Catch-up
is bounded by `SCHEDULER_MAX_CATCHUP_SECS` and 30 occurrences per schedule;
up to 16 schedules run concurrently.

The job advances `last_fired_at` to now with a condition on the document's
`updateTime`, or uses create-if-absent for a new document. A losing claim
skips the schedule. The winner calls `POST /api/v1/sink/trigger/async` for
each occurrence with `Idempotency-Key: cron:<event_id>:<occurrence_rfc3339>`
and a cron payload containing `scheduled_for`.

The Firestore claim prevents overlapping jobs from claiming the same window.
The API's idempotency response cache lasts approximately 15 minutes per
replica. If a trigger fails after the claim, the job logs it and continues
the remaining occurrences, then exits nonzero. The claim stays advanced so
successful occurrences are not repeated. Inspect ERROR logs for missed
occurrences and handle them individually; a new tick or task retry skips
windows already claimed.

| Exit code | Meaning |
| --- | --- |
| `0` | All claimed occurrences fired; lost claims and rejected expressions do not fail the job |
| `1` | Listing, state, claim, trigger, or deadline failure |
| `2` | Startup refused configuration, credentials, backend, token, URL, or Firestore client construction |

Configure bounded task retries. A retry makes a fresh tick against the same
state, claiming only remaining windows. Cloud Run's `CLOUD_RUN_EXECUTION`
and `CLOUD_RUN_TASK_ATTEMPT` appear in startup logs for correlation. Run a
manual tick with:

```sh
gcloud run jobs execute <scheduler-job> --region <region> --wait
```

The image depends on GCP metadata identity and cannot authenticate from a
workstation by adding a service-account key file.

## Maintenance job

The maintenance job calls `POST /api/v1/maintenance/run` with the API's shared
bearer token. It holds no database credentials and does not request a Google
token. Set these values:

| Variable | Value |
| --- | --- |
| `API_BASE_URL` | HTTPS API URL, without credentials, query, or fragment; `API_URL` is a fallback |
| `MAINTENANCE_TOKEN` | Same value as the API, at least 32 bytes after trimming, injected from Secret Manager |
| `MAINTENANCE_JOB` | `telemetry_alerts`, `cache_cleanup`, `run_sweep`, `state_cleanup`, `payments`, or `all` (default, case-insensitive) |
| `ALLOW_INSECURE_API_BASE_URL` | `1`/`true` permits HTTP for trusted development; leave unset for the public API URL |

The maintenance image also rejects the shared credential, metadata override,
and proxy settings, and its HTTP client disables proxy lookup. In `all` mode
it makes five independent requests, in the order listed above, and attempts
later jobs even if an earlier one fails.

A daily cleanup schedule, for example 03:00 UTC, provides only daily run
reconciliation. Create a separate more frequent `run_sweep` job when stuck
runs must become terminal sooner. The API's `RUN_SWEEPER_BATCH_SIZE` defaults
to 500 and is capped at 900. Set `RUN_SWEEPER_GRACE_SECS` above the longest
legitimate queue delay plus `EXECUTOR_TIMEOUT_SECS` to avoid classifying active
runs as stale. Run sweep updates canonical SQL rows; it leaves separately
configured execution state backends unchanged.

When payment servicing is enabled, schedule a separate `MAINTENANCE_JOB=payments`
execution at least once per minute. A daily `all` job does not provide timely
payment recovery when the API has no continuous CPU. See
[Payments rollout and recovery](/dev/platform-administration/#payments-rollout-and-recovery)
for configuration and alert requirements. Provider sandbox validation remains a
rollout prerequisite.

Set `FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1` on the API when scheduled
maintenance owns alert evaluation. Transactional rule updates and conditional
sweeps make repeats safe. `Idempotency-Key: <job>:<CLOUD_RUN_EXECUTION>` is
stable across task retries and used for correlation by the API. A fresh job
execution gets a new key. Local runs use the UTC start minute as the suffix.

Each request has a five-second connect timeout and a 300-second overall
timeout. Five sequential requests can consume roughly 25 minutes; configure
a job timeout with margin, such as 1800 seconds, and keep the API's request
timeout compatible with the client. Cloud Run task retries repeat all
selected jobs, including earlier successes.

Transport errors, non-2xx responses, malformed bodies, and responses for a
different job fail that request. The process exits nonzero after completing
the selection if any request failed. Logs classify 408, 429, and 5xx as
transient and other 4xx as deployment/configuration errors. Both classes use
the configured task retry policy; there is no in-process retry loop. Retain
Scheduler delivery logs as well as failed Cloud Run execution logs.

Build from the repository root and run the configured job manually with:

```sh
docker build -f apps/backend/gcp/maintenance/Dockerfile -t flow-like-gcp-maintenance .
gcloud run jobs execute <maintenance-job> --region <region> --wait
```

For a local smoke check, export the API's existing maintenance token, then pass
it through to the container:

```sh
docker run --rm \
  -e API_BASE_URL=https://api.example.com \
  -e MAINTENANCE_TOKEN \
  -e MAINTENANCE_JOB=telemetry_alerts \
  flow-like-gcp-maintenance
```

When provisioning a token for the first time, `openssl rand -base64 48`
generates a suitable value. Configure that same value on the API before using
it for a request.
