# Azure executor server

This image is the long-lived synchronous executor the Azure API reaches at
`EXECUTOR_URL` when `EXECUTION_BACKEND=http`. It is server-only: the process
serves `POST /execute`, `POST /execute/stream`, `POST /execute/sse` and
`GET /health` on `PORT`, and `GET /metrics` on `METRICS_PORT`, and it has no
job-once mode. Every run arrives as a signed `ExecutionRequest` whose body
carries the presigned URLs and cloud `RuntimeCredentials` the run may use, so
the process opens no Azure client, holds no managed-identity token, and needs
no queue or database. The user-assigned identity the root attaches serves the
platform's registry pull; the process itself never requests a token with it.

Required environment variables:

- `EXECUTOR_SERVER_MODE`: must be `true` (or `1`). The image bakes it in; any
  other value is refused at startup with a message, because there is nothing
  else this binary could run.
- `BACKEND_PUB`: base64 of the API's P-256 public PEM (the ES256 verifier for
  execution JWTs; public key material, not a secret). It is decoded at startup
  the same way the executor decodes it per request - standard base64 of the
  untrimmed value, then an EC public key PEM - so an empty or malformed value
  fails the revision instead of every run. There is no JWKS fallback on
  Azure because the root does not pass `API_URL` to workers.

Optional settings:

- `PORT` (default `8080`) and `METRICS_PORT` (default `9090`); they must
  differ. The Container App probes `/health` on `PORT`.
- Executor tuning read by `ExecutorConfig::from_env()`: `EXECUTOR_TIMEOUT_SECS`
  (default `3600`), `EXECUTOR_BATCH_INTERVAL_MS`, `EXECUTOR_MAX_BATCH_SIZE`,
  `EXECUTOR_CALLBACK_TIMEOUT_MS`, `EXECUTOR_CALLBACK_RETRIES`.
- `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP/gRPC trace export; traces are off when
  it is unset, and a value that cannot initialise the exporter is a startup
  error rather than a silently disabled exporter.
- `RUST_LOG` (image default `info`).
- `API_BASE_URL`: fallback Flow-Like API URL for model calls created without a
  run context. Normal executions use the signed callback URL carried in their
  execution JWT. Hosted-provider credentials stay on the API service.

The root's shared worker environment (`AZURE_STORAGE_ACCOUNT_NAME`, the
container names, `COSMOS_*`, `AZURE_QUEUE_STORAGE_ACCOUNT_NAME`,
`AZURE_CLIENT_ID`) is accepted and ignored: none of it is a credential, and
this process never opens a data-plane client of its own.

Forbidden environment variables (startup failure, same list as the Azure API
plus the queue worker's storage entries and the client-certificate pair):
`ACS_EMAIL_ACCESS_KEY`, `ACS_EMAIL_CONNECTION_STRING`,
`AZURE_COMMUNICATION_CONNECTION_STRING`, `AZURE_COMMUNICATION_KEY`,
`AZURE_STORAGE_CONNECTION_STRING`, `AZURE_STORAGE_ACCOUNT_KEY`,
`AZURE_STORAGE_ACCESS_KEY`, `AZURE_STORAGE_KEY`, `AZURE_STORAGE_MASTER_KEY`,
`AZURE_STORAGE_CLIENT_SECRET`, `AZURE_CLIENT_SECRET`,
`AZURE_CLIENT_CERTIFICATE_PATH`, `AZURE_CLIENT_CERTIFICATE_PASSWORD`,
`AZURE_STORAGE_SAS_KEY`, `AZURE_STORAGE_SAS_TOKEN`, `AZURE_STORAGE_TOKEN`,
`AZURE_STORAGE_USE_EMULATOR`, `AZURE_USE_EMULATOR`, `AZURE_USE_AZURE_CLI`,
`COMMUNICATION_SERVICES_CONNECTION_STRING`, `AZURE_SKIP_SIGNATURE`,
`AZURE_STORAGE_SKIP_SIGNATURE`, `AZURE_STORAGE_ENDPOINT`, `AZURE_ENDPOINT`,
`AZURITE_BLOB_STORAGE_URL`.

Security and delivery behavior:

- The executor runs untrusted flows in-process, and the `object_store` and
  `azure_identity` builders those flows can reach read keys, SAS tokens,
  client secrets and endpoint overrides from the environment. That is why a
  process-wide credential is refused rather than merely unused: anything in
  the environment is exfiltrable by any node.
- The catalog and executor both use the remote-only `server` bundle. ONNX
  metadata, ONNX Runtime, local model weights and desktop automation nodes are
  excluded. Text embedding models with remote execution configuration call the
  authenticated API proxy.
- `/metrics` is served only on `METRICS_PORT`, never on the ingress port. It
  carries `http_requests_total` / `http_request_duration_seconds` labelled by
  matched route (not raw URI, so an unauthenticated scanner cannot grow the
  label set), plus `flow_executions_total`, `flow_execution_duration_seconds`
  and `executor_active_jobs` under the docker-compose runtime's names. On the
  streaming routes the handler returns at the first event, so those three read
  as time-to-first-event there; the run's true duration lives in the API's run
  record.
- `SIGTERM` (revision rotation, scale-in) closes the listeners and drains
  in-flight runs so callbacks and final stream events are delivered; the
  Container App's termination grace period is the hard ceiling. Both servers
  are awaited to completion rather than raced, so a finished metrics listener
  cannot take the executor down mid-run.
- The image mirrors the queue worker's Dockerfile: `rust:1.97.1-bookworm` →
  `debian:bookworm-slim`, `--locked --release`, stripped, uid 10001,
  `STOPSIGNAL SIGTERM`, BuildKit cache mounts, `ca-certificates` for the
  presigned-URL and callback TLS paths, and no embedded configuration secret.
