# AWS maintenance Lambda

This Lambda is a stateless EventBridge Scheduler target. It has no database
credentials and can invoke only maintenance jobs allowlisted by the Flow-Like
API.

## Configuration

Set these Lambda environment variables:

- `API_BASE_URL`: public or private base URL of the Flow-Like API.
- `MAINTENANCE_TOKEN`: a random token of at least 32 bytes. Configure the same
  secret on the API. Store it in AWS Secrets Manager or SSM and inject it at
  deployment time rather than committing it.

`API_BASE_URL` must use HTTPS because the token is a bearer credential. For
trusted local development or explicitly encrypted private service networking,
HTTP can be enabled with `ALLOW_INSECURE_API_BASE_URL=1`.

Generate a suitable token, for example:

```sh
openssl rand -base64 48
```

Create the function with the `arm64` architecture (the Docker image targets
Amazon Linux 2023 ARM64) and a timeout above the Lambda's 120-second HTTP
timeout; 180 seconds is a sensible starting point.

The API deployment that is driven by this Lambda should set
`FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1`; otherwise its in-process evaluator is
redundant. Transactional rule-row locks still prevent overlapping API replicas,
manual runs, and scheduler retries from emitting the same transition twice.

## EventBridge Scheduler

For telemetry alerts, use a fixed-rate schedule such as every five minutes and
the following target input:

```json
{
  "job": "telemetry_alerts",
  "schedule_arn": "<aws.scheduler.schedule-arn>",
  "scheduled_time": "<aws.scheduler.scheduled-time>",
  "execution_id": "<aws.scheduler.execution-id>",
  "attempt_number": "<aws.scheduler.attempt-number>"
}
```

Create a separate fixed-rate schedule for stuck run reconciliation. Five
minutes is a reasonable starting interval:

```json
{
  "job": "run_sweep",
  "schedule_arn": "<aws.scheduler.schedule-arn>",
  "scheduled_time": "<aws.scheduler.scheduled-time>",
  "execution_id": "<aws.scheduler.execution-id>",
  "attempt_number": "<aws.scheduler.attempt-number>"
}
```

Configure `RUN_SWEEPER_GRACE_SECS` and `RUN_SWEEPER_BATCH_SIZE` on the API.
The batch size defaults to 500 and is capped at 900. Each invocation handles
the oldest stale runs first. A response that fills the batch indicates that
the next scheduled invocation may still have backlog to reconcile.
Set the grace period above the longest legitimate queue delay plus
`EXECUTOR_TIMEOUT_SECS`; otherwise the sweep can classify a live run as stale.
The job reconciles the canonical SQL run row only. It does not mutate a
separately configured execution state backend.

Create a daily schedule for expired-state cleanup with the same envelope and
`"job": "state_cleanup"`. It calls the state store's expired-run and
expired-event deletion; the operation is idempotent and safe to repeat.

Set Lambda reserved concurrency to `1`; the API's transactional alert updates
and conditional sweeps remain the final correctness boundary.

There are two distinct failure paths to configure:

- EventBridge Scheduler invokes Lambda asynchronously. Configure the
  Scheduler retry policy and Scheduler DLQ for failures delivering an event to
  the Lambda service.
- Configure the Lambda's **Asynchronous invocation** settings for failures
  returned by this handler: bounded `MaximumRetryAttempts` (for example `2`),
  a `MaximumEventAgeInSeconds` appropriate to the schedule (for example `900`),
  and an on-failure destination or Lambda DLQ.

The Lambda sends:

```http
POST /api/v1/maintenance/run
Authorization: Bearer <MAINTENANCE_TOKEN>
Idempotency-Key: aws-scheduler:telemetry_alerts:<schedule-arn>:<scheduled-time>
Content-Type: application/json

{"job":"telemetry_alerts"}
```

The API currently uses `Idempotency-Key` for correlation rather than a durable
deduplication ledger. Telemetry alert retries are safe because each rule's
state transition is transactional. Future jobs must either be inherently
idempotent or add durable idempotency storage before being enabled here.

Every request failure or non-`2xx` response fails the Lambda invocation. HTTP
`408`, `429`, and `5xx` responses are logged as transient; other `4xx`
responses are logged as deployment/configuration errors. Lambda's asynchronous
invocation configuration controls retries of either class and retains
exhausted events; the Scheduler's retry/DLQ settings cover only delivery to the
Lambda service.
