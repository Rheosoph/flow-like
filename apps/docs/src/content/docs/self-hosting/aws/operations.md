---
title: AWS service operations
description: Configure Lambda execution, storage accounting, and scheduled maintenance for the AWS backend.
sidebar:
  order: 20
---

The AWS API accepts requests in Lambda, dispatches asynchronous runs through
SQS, and receives execution callbacks. The same API can instead run as a
long-running [ECS service](/self-hosting/aws/ecs-api/). A separate file tracker consumes S3
notifications from SQS and updates storage totals. Apply the
[database schema and runtime grants](/self-hosting/aws/database/) before enabling
the API or file tracker.

## Build and configure the API

Install Rust and Cargo Lambda, then build from the repository root:

```sh
cargo lambda build --release -p aws-api
```

The API uses `lambda_http` with streaming responses. Configure `SECRET_PREFIX`
for SSM secret lookup and `CDN_BUCKET_NAME` for the CDN store. External S3-compatible
CDN stores can also use `CDN_BUCKET_ENDPOINT` and `CDN_BUCKET_ACCESS_KEY_ID`, with
`CDN_BUCKET_SECRET_ACCESS_KEY` in the secret store. See the
[API entrypoint](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/aws/api/src/main.rs)
for how the store is constructed.

`DSQL_CLUSTER_ENDPOINT` selects Aurora DSQL. The API and file tracker otherwise
read `DATABASE_URL` from the environment or SSM under `SECRET_PREFIX` for a
PostgreSQL or CockroachDB deployment. The
[database environment contract](/self-hosting/aws/database/#environment-contract)
lists the DSQL settings, forbidden password sources, and IAM permissions.

## Asynchronous execution

Configure the API's SQS execution backend and attach the queue to the
`aws-executor-async` Lambda. Each message carries an execution request; the
executor sends progress and terminal events to the API's callback URLs.

```sh
cargo lambda build --release -p aws-executor-async
```

Enable **Report batch item failures** on the SQS event-source mapping. The
handler returns failed message IDs so successful messages in the same batch
can settle. It processes records sequentially, so start with a batch size of
one and increase it only when the combined processing time fits the Lambda
invocation timeout. Configure the queue's visibility and redrive policy for
that timeout and monitor its dead-letter queue.

| Variable | Default | Purpose |
| --- | --- | --- |
| `EXECUTOR_BATCH_INTERVAL_MS` | `1000` | Interval for callback event batches |
| `EXECUTOR_MAX_BATCH_SIZE` | `100` | Events per callback batch |
| `EXECUTOR_CALLBACK_TIMEOUT_MS` | `5000` | Timeout for each callback request |
| `EXECUTOR_CALLBACK_RETRIES` | `3` | Callback retry count |
| `EXECUTOR_TIMEOUT_SECS` | `3600` | Library execution limit; lower it to fit the deployed Lambda timeout and leave time for final callbacks |

For local invocation, use `cargo lambda watch` and pass an SQS event fixture to
`cargo lambda invoke --data-file <fixture.json>`. The message body must contain
the actual execution request; an API Gateway fixture does not exercise this
handler. The [SQS handler](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/aws/executor-async/src/main.rs)
defines the batch response behavior.

## Storage accounting

The file tracker commits each object's accounting row and its aggregate size
deltas in one SQL transaction. Duplicate or older S3 sequencers leave totals
unchanged. Deleted objects retain a zero-size accounting row so a delayed
notification cannot count them again. Totals cover current object versions;
noncurrent versions, incomplete multipart uploads, and S3 overhead are excluded.

A tombstone only guards against a notification the queue can still redeliver,
so the `state_cleanup` maintenance job deletes tombstones older than 30 days.
`FILE_ACCOUNTING_TOMBSTONE_RETENTION_DAYS` on the API changes that window and
`0` switches the sweep off. Values below 14 days are raised to it, because that
is the longest an S3 notification can sit in SQS. Rows for live objects are
never swept; they carry the size every later delta is measured against.

Grant the Lambda role `s3:GetObject` on tracked objects and `s3:ListBucket` on
their buckets. The tracker calls `HeadObject` after acquiring the SQL object
write intent, with an eight-second timeout, and reads again after a transaction
conflict. The bucket permission lets a missing object return 404. An
access-denied response is retried, so missing IAM permissions cannot silently
turn a live object into a deletion.

### Upgrade from the DynamoDB accounting tracker

The previous tracker updated DynamoDB object sizes separately from SQL totals.
It must be stopped and drained before the SQL accounting tracker starts.

1. Disable its SQS event-source mapping and wait for active invocations to
   finish. Retain queued messages for the replacement and pause object writes
   while reconciling the baseline.
2. Reconcile `App.totalSize` and `User.totalSize` against the legacy inventory.
   Previous failures may have caused drift that the migration cannot infer.
   Use an S3 inventory when the old inventory is incomplete, preserving whether
   each key contributes to an app or a user-owned app.
3. Apply the migration creating `FileAccountingObject`. Set `FILES_TABLE_NAME`
   to the legacy DynamoDB table and `FILES_LEGACY_BUCKET_NAME` to its bucket.
   Legacy keys contain no bucket identifier. If that table mixed buckets with
   overlapping keys, reconcile them before cutover.
4. Grant `dynamodb:GetItem` on the legacy table, enable the replacement, and
   resume writes. Keep the legacy table read-only. The first successful new
   event imports an object's old contribution once; later events use SQL state.
   Other buckets start with no legacy contribution.

Keep the table and both legacy settings until all baseline rows have been
imported or reconciled. New installations with zero initial totals omit both
settings. Never resume the old worker after cutover or remove SQL accounting
tombstones during ordinary app/user cleanup; delayed S3 events can outlive their
owners. Set `FILE_ACCOUNTING_TOMBSTONE_RETENTION_DAYS=0` while the baseline
import is still configured: the import runs once per object and keys on the
accounting row being absent, so a pruned row lets a later event for the same key
re-apply a baseline the totals no longer carry. Re-enable the sweep once
`FILES_TABLE_NAME` is gone.

Build with `cargo lambda build --release -p file-tracker`. For the accounting
regression tests, set `FLOW_LIKE_TEST_DATABASE_URL` to a disposable PostgreSQL
database that permits creating and dropping schemas, then run
`cargo test -p file-tracker`. Tests cover rollback, legacy import, duplicate and
out-of-order delivery, and concurrent accounting updates.

## Scheduled maintenance

The maintenance Lambda calls the API's allowlisted jobs and holds no database
credentials. Set `API_BASE_URL` to the API's HTTPS base URL and inject the same
`MAINTENANCE_TOKEN` on both services from SSM or Secrets Manager. Generate a
token with at least 32 bytes, for example `openssl rand -base64 48`.
`ALLOW_INSECURE_API_BASE_URL=1` permits HTTP only for trusted local development
or explicitly encrypted private service networking.

The container image targets ARM64. Configure a Lambda timeout above its
120-second HTTP timeout, such as 180 seconds, and reserved concurrency of one.
Set `FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1` on the API when scheduled maintenance
owns alert evaluation.

Create separate EventBridge Scheduler targets for the jobs you need. For
five-minute alert evaluation, use this input:

```json
{
  "job": "telemetry_alerts",
  "schedule_arn": "<aws.scheduler.schedule-arn>",
  "scheduled_time": "<aws.scheduler.scheduled-time>",
  "execution_id": "<aws.scheduler.execution-id>",
  "attempt_number": "<aws.scheduler.attempt-number>"
}
```

Use the same envelope with `"job": "run_sweep"` for stuck-run reconciliation
and `"job": "state_cleanup"` for daily expired-state cleanup.
`RUN_SWEEPER_BATCH_SIZE` on the API defaults to 500 and is capped at 900; a full
batch can indicate remaining backlog. Set `RUN_SWEEPER_GRACE_SECS` above the
longest legitimate queue delay plus `EXECUTOR_TIMEOUT_SECS`. A shorter grace
can classify a live run as stale. Reconciliation updates the canonical SQL run
row and leaves separately configured execution state backends unchanged.

State cleanup deletes expired runs and events, then sweeps staged content-store
payloads by age. Payloads over 100 KiB are staged before their referencing row
is written, so a failed insert can leave an orphan object. Set
`EXECUTION_STAGED_PAYLOAD_MIN_AGE_SECS` on the API to control the minimum age
(default `172800`; values below one event lifetime are ignored). Logs report
`scanned`, `deleted`, and `stopped_early` for this sweep. The same job prunes
expired storage-accounting tombstones and reports them as `deletedTombstones`;
a sweep that fails is logged and does not fail the job. One pass removes at most
100,000 rows, so a large first cleanup finishes over several days.

The Lambda sends `POST /api/v1/maintenance/run` with the bearer token, the job
body, and an `Idempotency-Key` derived from the job, schedule ARN, and scheduled
time. That key provides log correlation. Transactional alert updates and
conditional sweeps provide repeat safety.

Configure both failure paths: Scheduler retries and a Scheduler dead-letter
queue cover delivery to Lambda; Lambda asynchronous invocation settings cover
handler failures. For the latter, set bounded retry attempts, an event age
appropriate to the schedule, and an on-failure destination or Lambda dead-letter
queue. Request failures and non-2xx responses fail the invocation. Logs classify
408, 429, and 5xx as transient, and other 4xx as deployment/configuration errors.
