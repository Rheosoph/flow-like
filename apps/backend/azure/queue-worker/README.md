# Azure Queue Storage queue worker

This image consumes one of four Flow-Like workloads from Azure Queue Storage:
execution jobs and compilation jobs (dispatched by the API), or released-content
blob events for file tracking and media transformation (delivered by an Event
Grid system topic). Deploy one Container App per workload from the same digest
with separate user-assigned identities. Each identity needs, scoped to its own
queue, `Storage Queue Data Message Processor` (receive + delete), `Storage
Queue Data Reader` (KEDA reads `ApproximateMessagesCount`), a custom role
granting `Microsoft.Storage/storageAccounts/queueServices/queues/messages/write`
(visibility-timeout renewal, which Message Processor does not cover), and
`Storage Queue Data Message Sender` on the `-poison` sibling.

Required environment variables:

- `AZURE_QUEUE_WORKLOAD`: `execution`, `compilation`, `file-tracking` or
  `media-transformation`
- `AZURE_QUEUE_STORAGE_ACCOUNT_NAME`: storage account hosting the queues
- `AZURE_QUEUE_NAME`: the queue this replica consumes
- `AZURE_QUEUE_POISON_NAME`: must be `<AZURE_QUEUE_NAME>-poison`
- `AZURE_CLIENT_ID`: user-assigned managed-identity client ID
- the existing executor/compiler runtime configuration and Key Vault-injected
  runtime values required by those packages

Blob-event workloads (Event Grid → Queue Storage, CloudEvents 1.0 or Event
Grid schema, base64 or raw JSON):

- `file-tracking` additionally needs `AZURE_CONTENT_CONTAINER`, the Cosmos
  settings (`COSMOS_ENDPOINT`, `COSMOS_DATABASE`, `COSMOS_AUTH_MODE`,
  `COSMOS_FILES_CONTAINER` default `files`) and the managed-identity
  PostgreSQL settings (`AZURE_POSTGRES_AUTH_MODE=managed_identity`,
  `AZURE_POSTGRES_HOST`, `AZURE_POSTGRES_DATABASE`, `AZURE_POSTGRES_USER`).
  It keeps a size ledger per blob in the `files` container (`pk` = app id) and
  applies size deltas to `App.totalSize` / `User.totalSize`. Its SQL role must
  exist and be allowed to `UPDATE` both tables. Because the PostgreSQL token
  cannot be refreshed in the pool, the process requests its own shutdown when
  the token enters its safety window and hard-stops before expiry; KEDA
  restarts it.
- `media-transformation` additionally needs `AZURE_STORAGE_ACCOUNT_NAME` and
  `AZURE_CONTENT_CONTAINER`; its identity needs `Storage Blob Data
  Contributor` on the content container. It converts `media/` uploads to WebP
  next to the original and deletes the original; `.webp` inputs are ignored,
  videos kept, other extensions deleted (AWS parity).

Optional bounded settings:

- `AZURE_QUEUE_VISIBILITY_TIMEOUT_SECS` (default `300`, range `60..604800`)
- `AZURE_QUEUE_RENEWAL_INTERVAL_SECS` (default `60`, range `10..3600`; must be
  below half the visibility timeout)
- `AZURE_QUEUE_PROCESS_TIMEOUT_SECS` (default `EXECUTOR_TIMEOUT_SECS + 600`;
  must exceed the executor run timeout)
- `AZURE_QUEUE_MAX_DEQUEUE_COUNT` (default `3`, range `1..100`)
- `AZURE_QUEUE_BATCH_SIZE` (pinned to `1` until waiting-message renewal exists)
- `AZURE_QUEUE_POLL_MIN_INTERVAL_SECS` (default `1`, range `1..60`)
- `AZURE_QUEUE_POLL_MAX_INTERVAL_SECS` (default `30`, range `1..300`)

Security and delivery behavior:

- Only Entra managed-identity authentication is implemented. Connection
  strings, account keys, SAS tokens, and client secrets cause startup failure.
- All traffic goes to `https://<account>.queue.core.windows.net` over TLS,
  which resolves to the account's `queue` private endpoint.
- Queue Storage has no long poll, so the worker polls with a bounded backoff:
  tight while messages flow, up to the ceiling once the queue drains. KEDA
  scales the replica to zero, so idle polling costs nothing.
- Messages are leased with a visibility timeout that is renewed while work
  runs. Every renewal returns a **new** pop receipt and invalidates the old
  one, so the receipt is a single mutable local that the renewal arm of the
  processing `select!` reassigns in place; delete and poison moves read it at
  call time. Renewal is never spawned into a separate task.
- A lost lease (any renewal failure) aborts the work without settling and logs
  `queue_lease_lost`. The message reappears on its own.
- Queue Storage has no broker delivery cap and no dead-letter queue.
  `DequeueCount` is enforced here before any work starts, and exhausted or
  permanently invalid messages are moved to `<queue>-poison` (put first, then
  delete the original) with the reason carried in the body, because queue
  messages have no user-settable properties. Every move logs
  `queue_message_poisoned` at ERROR.
- Bodies are base64-encoded on the wire; raw JSON is also accepted so the same
  decoder handles Event Grid-delivered payloads. Decoded bodies above 48 KiB
  are rejected — anything larger is staged to Blob Storage by the producer and
  arrives as a claim-check reference, which this worker resolves.
- Queue Storage mints its own `MessageId`, so the signed job identity travels
  in the versioned dispatch envelope (`v`, `job_id`, `payload`) and is compared
  against the resolved payload's `job_id` exactly as the broker message ID used
  to be.
- This is at-least-once processing. Callback/run persistence remains the final
  idempotency boundary when settlement fails after completed work.
