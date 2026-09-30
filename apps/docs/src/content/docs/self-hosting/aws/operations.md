---
title: AWS service operations
description: Configure Lambda execution, email automation, storage accounting, and scheduled maintenance for the AWS backend.
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
[API entrypoint](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/aws/api/src/main.rs)
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
handler. The [SQS handler](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/aws/executor-async/src/main.rs)
defines the batch response behavior.

## Email automation

Inbound email events run on the server. SES stores the complete MIME message,
including attachments, in a temporary S3 bucket. The receipt action publishes
an SNS notification after storage; a Lambda passes the object reference and
SMTP envelope to the API. This preserves recipients such as BCC addresses that
may be absent from the message headers. The API records pending deliveries
before returning HTTP 202, then dispatches them through the existing
asynchronous executor. Repeated receipt notifications use the same delivery ID.

### Deploy receiving

1. Apply the current database migrations and deploy the API with its `ses`
   feature. The `aws-api` package already enables it. Configure `SINK_SECRET`
   and the asynchronous execution backend first. The API advertises
   `supported_sinks.inbound_email` when its resolved mail configuration is
   enabled.
2. Choose a dedicated domain such as `events.example.com`, verify it with SES
   in a [region that supports receiving](https://docs.aws.amazon.com/ses/latest/dg/regions.html#region-receive-email),
   and create or select an active receipt rule set. The mail stack adds a rule
   to this existing set; it does not activate or replace the set.
3. As a platform administrator, register a service token with
   `POST /api/v1/admin/sinks` and the body below. Mail ingress requires a
   registered token so revocation takes effect immediately.

   ```json
   { "sink_type": "inbound_email", "name": "AWS mail ingress" }
   ```

   Store the returned token as an SSM SecureString named `SINK_TRIGGER_JWT`
   below a path dedicated to the gateway, such as `/flow-like/prod-mail`. Do
   not reuse the API's `SECRET_PREFIX`.

   ```sh
   aws ssm put-parameter --type SecureString \
     --name /flow-like/prod-mail/SINK_TRIGGER_JWT --value file://sink-token.txt
   ```

   Without `--key-id`, SSM encrypts it with the AWS managed `aws/ssm` key. When
   you pass a customer managed key, also pass its ARN as the stack's
   `SecretKmsKeyArn`.
4. Build and deploy
   [the SAM template](https://github.com/Rheosoph/flow-like/blob/dev/apps/backend/aws/mail-ingress/template.yaml)
   from the repository root. AWS SAM CLI and Docker are required.

   ```sh
   sam build --template-file apps/backend/aws/mail-ingress/template.yaml
   sam deploy --guided --resolve-image-repos --capabilities CAPABILITY_IAM
   ```

   Set `ReceiptDomain`, `ReceiptRuleSetName`, the API's HTTPS `ApiBaseUrl`, its
   existing IAM `ApiRoleName`, and `SecretPrefix` to the path from step 3. The
   stack writes the gateway's routing settings to `MAIL_INGRESS_CONFIG` below
   that path. The function reads both parameters at startup: it may call
   `ssm:GetParameter` on them and `kms:Decrypt` only for the token through SSM.
   The token is never a stack parameter or Lambda environment variable. After
   rotating the token or changing `ApiBaseUrl`, redeploy with a new
   `BootstrapVersion` so new Lambda environments load the parameters. Stacks
   deployed before this change passed the token as `SinkTriggerJwt`: store it
   in SSM first and remove that parameter from `samconfig.toml`.

   `AfterReceiptRule` places the new rule after a named existing rule; empty
   inserts it first. Review the order before deployment: this rule stops
   further rule-set processing for matching mail. Earlier rules that stop
   processing can prevent delivery to Flow-Like.
5. Store one `MAIL_CONFIG` JSON document in the API's existing secret store.
   On AWS, use an SSM SecureString named `${SECRET_PREFIX}/MAIL_CONFIG` with
   the API's existing parameter encryption key. Substitute the domain and
   bucket from the stack outputs:

   ```json
   {
     "enabled": true,
     "domain": "events.example.com",
     "bucket": "your-receipt-bucket"
   }
   ```

   No additional API environment variables are required. The defaults are
   `prefix: "raw/"`, `ttl_seconds: 3600`, `max_bytes: 26214400` (25 MiB),
   `sending_enabled: true`, and `min_send_interval_seconds: 5`. Add these
   fields only when the deployment needs a different limit. Without a
   configured mail document or legacy environment settings, automation is
   disabled. The same settings can be supplied in the private
   `mail_automation` section of the runtime configuration document;
   `MAIL_CONFIG` takes precedence. Existing `INBOUND_MAIL_*` and
   `MAIL_AUTOMATION_*` environment overrides remain supported. When only the
   legacy `INBOUND_MAIL_DOMAIN` variable enables mail, `sending_enabled`
   defaults to `false`; `MAIL_AUTOMATION_ENABLED=true` turns sending on.

   Settings are loaded at API startup. Recycle the API after changing the
   parameter. Terraform deployments include its version in the existing
   configuration revision, so a parameter change causes that update.

   The stack grants the API role read, decrypt, and delete access to this
   receipt prefix. Its bucket uses default SSE-KMS encryption. Keep the SES S3
   action's `KmsKeyArn` unset: that option enables SES client-side encryption,
   which requires a different reader. The SES writer role has the data-key and
   decrypt permissions needed for the bucket's server-side encryption.
   See [SES receiving permissions](https://docs.aws.amazon.com/ses/latest/dg/receiving-email-permissions.html).
6. Publish an MX record for the receiving domain, priority `10`, using the
   `ReceivingMxTarget` output. Create an active inbound email event with server
   execution enabled, then send a test message to its generated address or
   alias. Verify one run and readable attachments. SES accepts the configured
   domain; the API discards unknown or disabled addresses.

### Use the event outputs

The [Inbound Email Event](/nodes/events/events-inbound-email/) (version 4) has
these outputs besides its execution pin:

| Output | Type | Contents |
| --- | --- | --- |
| Message | `MailMessageRef` | The stored delivery, for Reply Platform Email |
| Mail Session | `MailSession` | The app and event that own the address, for Send Platform Email |
| Addresses | Struct | `sender`, `from`, `to`, `cc`, `reply_to`, the SMTP `envelope_from`, and the receiving `recipient` |
| Content | Struct | `subject`, `text` and `html` previews, `text_truncated`, `html_truncated`, and the complete bodies as `text_path` and `html_path` files |
| Attachments | `InboundEmailAttachment[]` | Per attachment: `filename`, `content_type`, `size`, `path` (FlowPath), `content_id`, `disposition`, `charset`, and `embedded` |
| Delivery | Struct | `automated`, `expires_at`, `raw_path` (the original MIME message), `omitted_attachments`, `received_at`, `message_id`, `provider_delivery_id`, `headers`, and `authentication` |

Attachments is an array of structs, so it does not connect to a file input
directly. Connect it to **For Each**, or **Get Element** for one entry, then
**Break Struct**, and pass `path` to a file node such as **Read to String** or
**Copy**, or to [Sign URL](/nodes/data/files/operations/sign-url/) for a
download link.

`embedded` is true for parts the HTML body shows through `cid:<content_id>`,
such as signature logos; skip them when you only want real attachments. Text
attachments without a charset, or in UTF-8 or US-ASCII, keep their original
bytes. Those in another declared `charset` are stored as UTF-8.

Bodies, attachments, and the raw message are temporary request files. Flow-Like
deletes them after `Delivery.expires_at`, which is at least
`MAIL_CONFIG.ttl_seconds` after dispatch, so copy files you need later to app
storage. The paths and mail references are serializable locators without
credentials. Passing a reference does not grant access: the backend checks app,
event, and execution ownership when sending or replying.

### Size and content limits

- The complete MIME message must fit `MAIL_CONFIG.max_bytes`: 25 MiB by
  default, which leaves about 18 MiB for attachments after encoding. The
  setting accepts up to 40 MiB; SES itself receives messages up to 40 MB. While
  dispatching, the API holds about four times the message size in memory, so
  size ECS tasks for the limit you choose.
- On SES, larger mail is dropped: no run starts and the sender gets no bounce.
  The API logs a warning with the app ID, event ID, and limit. Postfix
  deployments reject such mail during SMTP with `552`, so the sender gets a
  bounce; their limit is `MAIL_MAX_MESSAGE_BYTES` (10 MiB by default).
- At most 100 attachments are extracted. `Delivery.omitted_attachments` counts
  the rest, which remain in `raw_path`.
- A message forwarded as an attachment arrives as one `.eml` file; its inner
  attachments are not listed.
- Outlook rich-text mail arrives as a single `winmail.dat` attachment.
- Cloud share links from Outlook, OneDrive, or iCloud are links in the body,
  not attachments.
- Mail with a failed SES virus verdict is rejected and starts no run.

### Recovery and retention

The stack retries failed Lambda invocations and records exhausted deliveries
in the `FailedReceiptsQueue`. Set `AlarmTopicArn` to receive its queue alarm.
Repair the failure and replay the original SNS Lambda event while its S3
object still exists. A one-minute EventBridge schedule independently calls
the dispatch endpoint to recover pending deliveries and remove expired mail
files. Disabling this schedule also stops that cleanup.

Only ingestion failures fail a receipt invocation. The function also requests
dispatch right after ingestion; when that request fails or would outlast the
invocation, it logs a warning and leaves the delivery to the schedule. Failed
scheduled invocations land in the same queue. They carry no mail and can be
deleted instead of replayed.

`RawRetentionDays` defaults to one day. The original SES object is shared by
all recipients of a message. The API deletes it when it cleans up or expires
the delivery; the stack grants the API role `s3:DeleteObject` on the receipt
prefix for this. The S3 lifecycle expiration removes objects that cleanup
missed. S3 removal is asynchronous, so this is an expiration age rather than
an exact deletion deadline. The failure queue retains metadata for 14 days,
which can outlive the raw message.
Replaying that metadata cannot recover an expired object.

The API also writes decoded bodies, attachments, and a raw-message copy to
the platform's temporary store. Configure encryption at rest and a lifecycle
rule on that store as a fallback for interrupted cleanup. Set its expiration
age long enough for `MAIL_CONFIG.ttl_seconds` and active runs. The API removes
these files after the receipt expires; event payloads, run logs, user-created
copies, and execution history follow their own retention settings. Receipt
expiration does not erase those copies. SES spam and authentication verdicts
are available to the flow as metadata.

### Enable sending

Sending requires the platform mail configuration with `provider: "ses"` and
the receiving domain verified for sending in the selected AWS region. Grant
the API role `ses:SendEmail` for that domain identity. Sending is enabled with
the mail document above; set `sending_enabled: false` for receive-only
deployments. A [verified SES domain](https://docs.aws.amazon.com/ses/latest/dg/creating-identities.html)
allows the API to send from each event address. Platform transactional messages
continue to use the configured `from_email`.

Connect the inbound event's Session output to Send Platform Email. To answer
the received message, connect both Session and Message to Reply Platform Email.
The backend checks that the session belongs to the current app and event, and
chooses the event's active From address. A reply derives its recipient, subject,
and threading headers from the stored original message. Reply requires that
the original receipt has not expired. Callers provide neither sender addresses
nor raw headers.

The corresponding endpoints are `POST /api/v1/apps/{app_id}/mail/send` with
`{session,to,cc,bcc,subject,text,html}` and
`POST /api/v1/apps/{app_id}/mail/reply` with `{session,message,text,html}`.
Both return the session and a request ID after provider acceptance. The default
limit is one message per app every five seconds, configured with
`MAIL_CONFIG.min_send_interval_seconds`. A message can address at most 20
recipients across To, CC, and BCC, with at most 1 MiB of body content. The send
endpoint and node currently support text and HTML bodies without attachments.

Request [SES production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html)
in the sending region before sending to arbitrary recipients. In the sandbox,
recipients must be verified or use the SES mailbox simulator. Sender identities
must remain verified after production access is granted.

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
