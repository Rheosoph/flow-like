---
title: Configuration
description: Configure public URLs, storage, execution limits and service credentials
sidebar:
  order: 23
---

Generate `apps/backend/docker-compose/.env` with `scripts/setup-env.py`, then
maintain it as private deployment configuration. The checked-in `.env.example`
contains names and defaults, without working secrets. Setup refuses to overwrite
an existing file.

Use `KEY=value` assignments. The helper scripts reject duplicate keys and
shell commands; do not treat the file as a shell script.

## Public and private URLs

| Variable | Default | Meaning |
| --- | --- | --- |
| `BIND_ADDRESS` | `127.0.0.1` | Bind address for published listeners |
| `WEB_PORT`, `API_PORT`, `SIGNALING_PORT` | `3001`, `8080`, `4444` | Edge proxy ports |
| `PUBLIC_API_URL`, `NEXT_PUBLIC_API_URL` | `http://localhost:8080` | Client-facing API origin |
| `NEXT_PUBLIC_REDIRECT_URL` | `http://localhost:3001/callback` | OIDC login callback |
| `NEXT_PUBLIC_REDIRECT_LOGOUT_URL` | `http://localhost:3001/` | Post-logout URL |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Exact API browser origins |
| `REALTIME_ALLOWED_ORIGINS` | `http://localhost:3001` | Exact signaling and storage CORS origins |
| `API_BASE_URL` | `http://execution-callback:8080` | Private execution callback origin |
| `S3_PUBLIC_ENDPOINT` | `http://s3.localhost:9000` | Exact signed object origin |

Keep the private callback origin distinct from the public API URL. The execution
proxy enforces callback paths against that private origin.

For public hosting, terminate TLS at a maintained reverse proxy and configure
its trusted addresses in `proxy/nginx.conf` before accepting forwarded client
headers. The supplied proxy overwrites client-provided forwarding headers.
Rebuild `web` after changing `NEXT_PUBLIC_*` values.

API CORS origins cannot contain wildcards, credentials, paths or query strings.
Add desktop origins to both origin lists when required:
`tauri://localhost,http://tauri.localhost,https://tauri.localhost`.

## Hub configuration and identity

`FLOW_LIKE_CONFIG` selects the JSON embedded in the API build.
`FLOW_LIKE_RUNTIME_CONFIG_FILE` selects the file mounted into sink services.
Keep both on the same maintained configuration. Replace the example's OIDC
authority, client settings, public domains, legal links and signaling URL.

The `signaling` service exchanges collaboration offers, answers and ICE
candidates. The hub's optional `realtime.ice` setting can configure Cloudflare
TURN credentials:

```json
"realtime": {
  "ice": {
    "provider": "cloudflare",
    "turn_key_id_secret_ref": "CLOUDFLARE_TURN_KEY_ID",
    "turn_key_api_token_secret_ref": "CLOUDFLARE_TURN_KEY_API_TOKEN",
    "ttl_seconds": 14400
  }
}
```

Set those two secrets in `.env`. The API resolves them through its secret
store and supplies temporary ICE configuration to clients. Restart the API
after changing the hub configuration or rotating the long-lived TURN key.
Leaving `realtime.ice` null provides no managed TURN relay.

## Execution mode and capacity

| Variable | Default | Meaning |
| --- | --- | --- |
| `EXECUTION_ISOLATION_MODE` | `per_run` | One gVisor sandbox per execution |
| `COMPOSE_PROFILES` | `per-run` | Manager, gateway and queue-bridge services |
| `EXECUTION_BACKEND` | `http` | Interactive dispatch |
| `ASYNC_EXECUTION_BACKEND` | `redis` | Background dispatch |
| `EXECUTOR_URL` | `http://execution-gateway:9000` | Internal manager load balancer |
| `EXECUTION_MANAGER_REPLICAS` | `1` | Independent managers on this daemon |
| `MAX_CONCURRENT_EXECUTIONS` | `10` | Active runs per manager |
| `SANDBOX_WARM_POOL_SIZE` | `2` | Additional unused slots per manager; minimum 1 |
| `SANDBOX_CREATE_CONCURRENCY` | `2` | Parallel preparation per manager |
| `SANDBOX_IDLE_TIMEOUT_SECONDS` | `300` | Maximum unused-slot age |
| `EXECUTION_MANAGER_WORKER_THREADS` | `2` | Tokio workers, independent of run count |
| `SANDBOX_MEMORY_MB`, `SANDBOX_CPUS` | `1024`, `1` | Runner memory and CPU limits |
| `SANDBOX_PIDS`, `SANDBOX_TMP_MB` | `128`, `256` | Runner PID and temporary-filesystem limits |
| `QUEUE_BRIDGE_REPLICAS`, `QUEUE_WORKER_CONCURRENCY` | `1`, `10` | Background dispatchers and concurrency |

`SANDBOX_IMAGE` and `SANDBOX_GATEWAY_IMAGE` must identify immutable images
already available to the daemon. Run `scripts/prepare-images.py` after changing
runner or gateway code.

Only exact HTTPS integration hosts in `EXECUTION_ALLOWED_HTTPS_HOSTS` are
permitted in addition to the run's callbacks and storage. Configure supported
HTTP clients to use the supplied proxy. Raw TCP/UDP access is unavailable.

`API_REPLICAS`, `WEB_REPLICAS`, `COMPILER_REPLICAS` and
`SIGNALING_REPLICAS` scale those services separately. Resource limits use their
corresponding `*_CPUS` and `*_MEMORY` settings. See
[Scaling](/self-hosting/docker-compose/scaling/) before increasing counts.

For a separate trusted installation, generate with `--mode trusted`. This
selects `trusted_shared`, the `trusted` profile and
`http://runtime-gateway:9000`. `RUNTIME_REPLICAS` applies only to that shared
mode. Preflight rejects mixed execution profiles.

## Execution and credential lifetimes

| Variable | Default |
| --- | --- |
| `EXECUTION_TIMEOUT_SECONDS` | `3600` |
| `SANDBOX_STARTUP_TIMEOUT_SECONDS` | `120` |
| `EXECUTION_TERMINAL_GRACE_SECONDS` | `60` |
| `EXECUTION_CLEANUP_TIMEOUT_SECONDS` | `30` |
| `EXECUTION_STOP_GRACE_PERIOD` | `65m` |
| `EXECUTION_QUEUE_MAX_WAIT_SECONDS` | `300` |
| `EXECUTION_CREDENTIAL_MARGIN_SECONDS` | `120` |
| `STS_SESSION_TTL_SECONDS`, `CHANNEL_TTL_SECONDS` | `7200` |

Actual remaining credential lifetime must cover queue wait, execution, startup,
terminal acknowledgement, cleanup and the margin. Cache hits receive the same
check. There is no automatic credential renewal. Increasing run duration requires
a matching review of provider limits, channel lifetime and stop grace.

`REDIS_EXECUTION_QUEUE=exec:jobs:v3` selects the current queue protocol.
`EXECUTION_QUEUE_MAX_DEPTH=10000` bounds ready, pending and quarantined entries
together. Do not change queue names or clear retained entries as a way to bypass
capacity or uncertain execution results.

## Datastores and secrets

The bundled Redis instance uses separate API, runtime, signaling, sink and
metrics ACL identities. Setup generates each password. Redis uses AOF persistence
with one-second fsync and `noeviction`; memory pressure causes command failures
instead of evicting execution state.

`DATABASE_POOL_MAX_CONNECTIONS=10` applies per API replica.
`POSTGRES_MAX_CONNECTIONS=100` is the bundled server limit. Preflight includes
rollout overlap and administrative connections in its comparison.

The generated `BACKEND_KEY` stays in the API. Runners receive
`BACKEND_PUB` and a signed execution capability. Keep the manager token,
maintenance token, storage root keys and sink secrets outside workflows.

The bundled S3 settings select `STORAGE_PROVIDER=aws`,
`RUNTIME_CREDENTIALS_PROVIDER=aws` and `S3_STS_PROVIDER=rustfs`.
The stock API includes the required AWS feature. In this configuration, empty
AWS bucket overrides fall back to the generic names; an empty
`CDN_BUCKET_NAME` falls back to the content bucket.
See [Storage](/self-hosting/docker-compose/storage/) for external providers.

For external PostgreSQL/Redis, add `docker-compose.external-datastores.yml`
to `COMPOSE_FILE`, set `DATASTORE_MODE=external` and supply
`DATABASE_URL`, `REDIS_URL`, `RUNTIME_REDIS_URL`,
`SIGNALING_REDIS_URL`, `SINK_REDIS_URL` and `METRICS_REDIS_URL`.
Apply reviewed schema changes explicitly with `docker compose run --rm db-init`.
The overlay removes automatic initialization; it does not move existing data.

## Compiler, event services and model providers

The deployment implements HTTP compilation:
`COMPILATION_BACKEND=http` and
`COMPILER_URL=http://compiler-gateway:8081`. There is no compiler Redis
consumer. Defaults allow two concurrent compilation jobs and two parallel
targets per job, with a 600-second request timeout.

Setup generates `SINK_SECRET`, `SINK_TRIGGER_JWT` and
`SINK_TOKEN_ENCRYPTION_KEY`. Enabled event adapters come from the hub
configuration. Preserve the encryption key while stored sink credentials use it.

Model-provider keys and endpoint variables are listed in `.env.example`.
Populate only the providers your installation uses, and keep their secrets out
of hub JSON.

## Validate configuration changes

```bash
python3 scripts/preflight.py
python3 scripts/up.py --build
docker compose ps --all
```

Use `docker compose config --quiet` when checking interpolation manually.
Rendered configuration without `--quiet` contains deployment secrets.
