# Flow-Like GCP executor

The synchronous HTTP executor. The API reaches it at `EXECUTOR_URL` when
`EXECUTION_BACKEND=http`, calling `POST /execute` for runs it waits on and `POST
/execute/sse` for runs it streams to the client. It is the Kubernetes executor's
server mode and nothing else:
there is no job-once mode, no queue, no database, no Secret Manager and no cloud
SDK. Every run arrives as an `ExecutionRequest` whose body already carries the
presigned URLs and the scoped runtime credentials it needs, and progress leaves
through the callback URL inside the signed `executor_jwt`. The image is keyless
because it holds nothing to be keyed with.

Deploy it as one Cloud Run service (`executor` in `deployments/gcp/dev`) with
internal ingress and the API's service account as its only invoker.

## Routes

All on **one port** — Cloud Run publishes exactly one container port per
service, so the second `METRICS_PORT` listener the Azure and Kubernetes
executors bind has no home here. `/metrics` is mounted on the serving router
exactly as the GCP API image does it; the executor's ingress is internal and its
only invoker is the API, so the route is reachable from inside the project and
from nowhere else. Setting `METRICS_PORT` is a boot error, not a silently
ignored variable.

| Route | Owner | Purpose |
|---|---|---|
| `POST /execute` | `flow_like_executor` | run to completion, events to the callback URL — the API's `http` backend for runs it waits on |
| `POST /execute/stream` | `flow_like_executor` | NDJSON event stream |
| `POST /execute/sse` | `flow_like_executor` | SSE event stream — the API's `http` backend for runs it streams |
| `GET /health` | `flow_like_executor` | `200` for the life of the process |
| `GET /health/live` | this crate | Cloud Run liveness probe; `200` for the life of the process |
| `GET /health/ready` | this crate | Cloud Run startup probe; `503 draining` after SIGTERM |
| `GET /metrics` | this crate | Prometheus text format |

## Environment contract

### Required

```text
BACKEND_PUB=<standard base64 of the API's ES256 public key PEM>
```

Validated at boot with the exact decoder and key parser
`flow_like_executor::jwt` applies per request — standard base64, untrimmed,
then `DecodingKey::from_ec_pem`. That module would otherwise read the variable
lazily and fall back to fetching JWKS from `API_URL`, which this deployment does
not set (the API's address arrives as `API_BASE_URL`), so a missing or malformed
key would surface as a `500` on the first execution instead of a revision that
never takes traffic.

### Mode and port

| Variable | Notes |
|---|---|
| `EXECUTOR_SERVER_MODE` | baked into the image as `true`. Unset is accepted (there is no other mode). Any set value other than `1`/`true` (case-insensitive, untrimmed — the Kubernetes executor's exact reading) is a **hard startup error**: that value selects the Kubernetes job-once mode, and this image has none. |
| `PORT` | injected by Cloud Run from the service's `port`; never declare it in the Terraform env map (the platform rejects the revision). Default `8080` in the image so `docker run` works. |
| `METRICS_PORT` | **forbidden** — see Routes. |

### Executor tuning (`ExecutorConfig::from_env`)

| Variable | Default | Notes |
|---|---|---|
| `EXECUTOR_TIMEOUT_SECS` | `3540` (image) | must be `1..=3570`. Cloud Run cuts a service request at `timeout_seconds`, 3600 at most; when the connection goes, hyper drops the handler, `execute()` is cancelled and the terminal callback is never sent. The executor's own timeout has to fire first — 30 s of margin for the callback batcher — so the run ends as a reported failure, not a vanished one. The library default of 3600 is unsatisfiable here by construction and is refused at boot. Set it lower than the service's actual `timeout_seconds` if that is below 3600. |
| `EXECUTOR_BATCH_INTERVAL_MS` | `1000` | |
| `EXECUTOR_MAX_BATCH_SIZE` | `100` | |
| `EXECUTOR_CALLBACK_TIMEOUT_MS` | `5000` | |
| `EXECUTOR_CALLBACK_RETRIES` | `3` | |

Every one of these is checked to parse before `ExecutorConfig::from_env` reads
it. That constructor silently falls back to its default on a value it cannot
parse, which would turn `EXECUTOR_TIMEOUT_SECS=48O` into 3600 — the one value
this platform can never honour.

### Telemetry

| Variable | Notes |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP/gRPC span exporter; unset disables tracing |
| `GCP_REQUIRE_OTEL` | `true` makes a missing endpoint fatal — the same knob, with the same meaning, as on the GCP API image |
| `RUST_LOG` | `info` in the image |

### API proxy routing

The root's `worker_common_gcp_env` (`GCP_PROJECT_ID`, the bucket names and
`*_PROVIDER` selectors, `EXECUTION_STATE_BACKEND`, `FIRESTORE_*`) remains
unused. Hosted completion and remote embedding calls use the signed callback
URL from each execution as their API proxy base. `API_BASE_URL` is the fallback
for model calls created without a run context. Provider credentials stay on the
API service.

### Forbidden environment

Startup fails, rather than silently ignoring, if any of these is set:
`GOOGLE_APPLICATION_CREDENTIALS`, `GOOGLE_APPLICATION_CREDENTIALS_JSON`,
`GOOGLE_CREDENTIALS`, `GOOGLE_OAUTH_ACCESS_TOKEN`, `CLOUDSDK_AUTH_ACCESS_TOKEN`,
`GCE_METADATA_HOST`, `GCE_METADATA_IP`, `GCE_METADATA_ROOT`,
`METADATA_SERVER_DETECTION`, `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (and the
lowercase spellings) — enforced by
`flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env` — plus the
GCP API image's own list: `GOOGLE_SERVICE_ACCOUNT`,
`GOOGLE_SERVICE_ACCOUNT_PATH`, `GOOGLE_SERVICE_ACCOUNT_KEY`, `SERVICE_ACCOUNT`,
`GOOGLE_SKIP_SIGNATURE`, `GOOGLE_ALLOW_HTTP`, `GOOGLE_ALLOW_INVALID_CERTIFICATES`,
`GOOGLE_PROXY_URL`, `GOOGLE_PROXY_CA_CERTIFICATE`, `GOOGLE_PROXY_EXCLUDES`,
`STORAGE_EMULATOR_HOST`, `PUBSUB_EMULATOR_HOST`, `FIRESTORE_EMULATOR_HOST`,
`DATASTORE_EMULATOR_HOST`, `SECRET_MANAGER_EMULATOR_HOST`, and every
`CLOUDSDK_API_ENDPOINT_OVERRIDES_*` / `GOOGLE_*_CUSTOM_ENDPOINT` member.

The list is enforced here even though the executor configures no Google client
of its own, because two paths make it live regardless. A request whose
`credentials` block is keyless makes `flow_like::credentials` build a bare
`GoogleCloudStorageBuilder`, which resolves through object_store's own chain —
service-account key, ADC file, then the metadata server — and every `GOOGLE_*`
key above is read straight out of the environment on that path. The catalog's
Vertex nodes build Application Default Credentials
(`google_credentials::Builder::default()`), which read the key-file variables
and then fall back to the metadata server the `GCE_METADATA_*` variables would
redirect. The proxy variables are honoured by the callback client and the OTLP
exporter. The scan runs before telemetry opens its first socket, and again
inside `Config::from_env`.

## Cloud Run configuration

Mirrors the `executor` service in `deployments/gcp/dev/main.tf`:

- `ingress = INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER`, invoker = the API's
  service account only. The URL is what the API's `EXECUTOR_URL` points at; it
  never appears in the load balancer's URL map.
- `timeout_seconds = 3600`, the platform maximum, and `EXECUTOR_TIMEOUT_SECS`
  below it (see above). If a root lowers `timeout_seconds`, lower
  `EXECUTOR_TIMEOUT_SECS` with it — the boot check knows only the platform
  ceiling, not the service's value.
- `cpu_idle = false`, as the root sets it: the callback batcher's final flush
  can outlive the response it belongs to, and a throttled CPU stalls it.
- Startup probe `GET /health/ready`, liveness `GET /health/live`. Readiness has
  no dependency to report on — this process holds no connection and no rotating
  token — so it closes only after SIGTERM.
- On SIGTERM the process closes readiness, stops accepting connections and lets
  in-flight runs finish until Cloud Run's SIGKILL roughly ten seconds later. No
  drain sleep: Cloud Run has already taken the instance out of rotation, and the
  seconds are better spent on the runs.

## Metrics

`/metrics` renders whatever is recorded through the process-global `metrics`
recorder. The one producer is the request middleware, which increments
`http_requests_total{route,method,status}` per response, keyed by the *matched*
route so a probe of unmatched URLs cannot mint unbounded label values. It is a
counter only: `/execute/sse` and `/execute/stream` return their headers before
the run finishes, so a duration measured at the middleware would time the
response start rather than the execution, and reporting it under
`http_request_duration_seconds` would be a number that lies for exactly the two
endpoints the API calls. The Kubernetes executor's described-but-never-recorded
`flow_executions_total` / `executor_active_jobs` series are deliberately not
advertised.

## Build

```sh
docker buildx build -f apps/backend/gcp/executor/Dockerfile .
```

Context is the `flow-like` workspace root. The Dockerfile is the GCP
queue-worker's stage for stage — `rust:1.97.1-bookworm` builder with BuildKit
cache mounts, `cargo build --locked --release --package gcp-executor`, stripped
binary into `debian:bookworm-slim` with `ca-certificates` and `libssl3`, uid
10001, `STOPSIGNAL SIGTERM`. No configuration secret is embedded; only the api
and queue-worker images carry `flow-like.config.json`.

The catalog and executor both use the remote-only `server` bundle, matching the
Azure and GCP queue workers. ONNX metadata, ONNX Runtime and local model weights
are excluded from this image. Text embedding models with remote execution
configuration call the authenticated API proxy.

## Differences from the Kubernetes executor

- Server-only. `EXECUTOR_SERVER_MODE` set to anything but `1`/`true` is a
  startup error instead of selecting job-once mode.
- One listener. `/metrics` on `PORT`; `METRICS_PORT` refused.
- `BACKEND_PUB` required and validated at boot; no JWKS fallback is relied on.
- `EXECUTOR_*` values must parse, and `EXECUTOR_TIMEOUT_SECS` must sit below the
  Cloud Run request ceiling.
- Cloud Run probe routes `/health/live` and `/health/ready`, with readiness
  closing on SIGTERM.
- The GCP forbidden-environment scan.
- No `local-ml` / ONNX; the queue-worker feature set instead.
