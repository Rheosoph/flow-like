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
| `FRONTEND_BASE_URL` | `http://localhost:3001` | Existing web origin for hosted App links (`/a/<app-id>/<route>`); setup uses `--web-origin` |
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
Recreate `web` after changing `NEXT_PUBLIC_*` values. Compose maps them to the
web image's public runtime URL settings; an image rebuild is unnecessary.

API CORS origins cannot contain wildcards, credentials, paths or query strings.
Add desktop origins to both origin lists when required:
`tauri://localhost,http://tauri.localhost,https://tauri.localhost`.

## Images

| Variable | Default | Meaning |
| --- | --- | --- |
| `FLOW_LIKE_IMAGE_TAG` | `dev` | Tag used for every empty `*_IMAGE` value |
| `API_IMAGE`, `WEB_IMAGE`, `DB_INIT_IMAGE`, `COMPILER_IMAGE`, `SIGNALING_IMAGE`, `SINK_SERVICES_IMAGE`, `OBJECT_STORE_INIT_IMAGE` | empty | Full image reference override |
| `RUNTIME_IMAGE` | empty | Runner and queue-bridge image |
| `EXECUTION_MANAGER_IMAGE` | empty | Manager and gateway image |
| `SANDBOX_IMAGE`, `SANDBOX_GATEWAY_IMAGE` | empty | Immutable references the manager starts per execution |

An empty `*_IMAGE` renders
`ghcr.io/rheosoph/flow-like-docker-compose-<workload>:${FLOW_LIKE_IMAGE_TAG}`.
The tag must match `[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}`. Channel tags such as
`dev`, `beta` and `latest` move with each publication; release tags such as
`1.4.0` and immutable `sha-<commit>-run-<run>-<attempt>` tags do not. See
[Containers](/self-hosting/containers/) for the full scheme. The self-hosted
packages are public; cloud-provider packages are not.

`scripts/pull-images.py` pulls the nine images at one tag and writes their
`repository@sha256:...` digests into the `*_IMAGE` values, records the tag in
`FLOW_LIKE_IMAGE_TAG` and copies the runtime and execution-manager pins into
`SANDBOX_IMAGE` and `SANDBOX_GATEWAY_IMAGE`. Preflight rejects a digest-pinned
`SANDBOX_IMAGE` that differs from `RUNTIME_IMAGE`, and a digest-pinned
`SANDBOX_GATEWAY_IMAGE` that differs from `EXECUTION_MANAGER_IMAGE`, so the
queue bridge, manager and sandboxes always run the same build. Bare
`sha256:<id>` values written by `prepare-images.py` are local image IDs and are
exempt from that comparison.

Set a `*_IMAGE` value explicitly to use a mirror, a fork or a locally built tag.
`up.py --build` builds from the checked-out sources: it first writes
`flow-like-<workload>:local` into every empty `*_IMAGE` so the build never
shadows a published name on that daemon, then runs `docker compose up --build`.
It cannot build digest-pinned references. A plain `docker compose build` with an
empty `*_IMAGE` tags the local build under the published name, and later
`up.py` runs keep using it because Compose only pulls missing images;
`pull-images.py` restores the published digests.

## Hub configuration and identity

`FLOW_LIKE_RUNTIME_CONFIG_FILE` selects the host-side JSON file mounted into the
API and sink services at `/app/flow-like.config.json`. The API selects that mount
through `FLOW_LIKE_CONFIG_FILE`. Replace the example's OIDC authority, client
settings, public domains, legal links and signaling URL. The API reads the whole
document once at startup. Recreate the affected services after changing the file.

Alternatively, supply `FLOW_LIKE_CONFIG_JSON` or
`FLOW_LIKE_CONFIG_SECRET_REF` through the API's configured SecretStore, and set
`FLOW_LIKE_CONFIG_FILE=` explicitly. The Compose default preserves this empty
value. More than one nonempty source fails preflight and API startup.
`setup-env.py` selects this alternate source automatically when JSON or a
reference is supplied in its environment. A secret reference requires its value
to be available inside the API container.

Sink services still reads `supported_sinks` from the mounted file. Keep that
setting aligned with the API when selecting JSON or a SecretStore reference.
Do not print rendered Compose configuration containing environment-based JSON.
See the [runtime API contract](/self-hosting/containers/#runtime-api-configuration)
for document limits, secret references and failure behavior.

If all three runtime sources are empty, the API uses its compiled public
fallback. `FLOW_LIKE_CONFIG` is the repository-relative file that local builds
compile into the API and audit-worker images. Never put deployment credentials
in it or publish them in an image.

### Audit worker policy

The `audit-worker` service reads only the `audit` section of the
`flow-like.config.json` compiled into its image. Compose passes the API's
`FLOW_LIKE_CONFIG` build argument to the worker as well, so a local build embeds
the same file in both images. The runtime sources above never reach the worker:
it has no config mount and no `FLOW_LIKE_CONFIG_*` variables. The published
worker image is built without a `FLOW_LIKE_CONFIG` override and runs the default
audit policy. The API still takes its own `audit` section from its runtime
source; keep the two aligned.

To change the worker's policy, edit the file `FLOW_LIKE_CONFIG` names, set
`AUDIT_WORKER_IMAGE` to a local tag such as `flow-like-audit-worker:local`, then
rebuild and restart the worker:

```bash
docker compose build audit-worker
python3 scripts/up.py
```

To pause sealing, stop the service with `docker compose stop audit-worker`. The
API keeps recording, and new records stay pending until
`docker compose start audit-worker`; the next `up.py` also starts it. In a Swarm
stack, run `docker service scale <stack>_audit-worker=0` and scale it back to 1
to resume.

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

See [Realtime signaling](/self-hosting/signaling/) for replica fanout, upgrade
compatibility and proxy-header requirements.

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
already available to the daemon. `scripts/pull-images.py` pins published
digests; run `scripts/prepare-images.py` instead after changing runner or
gateway code locally.

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

Percent-encode credentials in connection URLs. The schema updater uses guarded
Prisma `db push`; versioned PostgreSQL release migrations and automatic schema
rollback are not implemented. Review each change and verify restore before
rolling API instances onto the new schema.

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

### Hosted embedding providers

Hosted embedding Bits send document and query requests through the API. The API
selects the upstream provider from `parameters.remote` and resolves credentials
from its server secret store. Install provider credentials on the API; runners
use their existing authenticated model proxy connection.

Set the required variables below in the deployment's private `.env`, then
recreate the API container. Compose and Swarm pass these variables to the API.
Custom secret names also need explicit entries in the API's environment.

| `remote.implementation` | Default API key secret | Endpoint configuration |
| --- | --- | --- |
| `Internal` | `INTERNAL_EMBEDDING_SECRET` | `INTERNAL_EMBEDDING_ENDPOINT` |
| `CloudflareWorkersAI` | `HOSTED_CLOUDFLARE_API_TOKEN` | Fixed Cloudflare API; account ID from `HOSTED_CLOUDFLARE_ACCOUNT_ID` |
| `OpenAI` | `HOSTED_OPENAI_API_KEY` | Fixed `https://api.openai.com/v1/embeddings` |
| `AzureOpenAI` | `HOSTED_AZURE_API_KEY` | `HOSTED_AZURE_ENDPOINT`; `remote.model_id` is the Azure deployment name |
| `HuggingfaceEndpoint` | `HOSTED_HUGGINGFACE_API_KEY` | `HOSTED_HUGGINGFACE_EMBEDDING_ENDPOINT`, a Text Embeddings Inference service with `/v1/embeddings` support |
| `OpenAICompatible` | `HOSTED_OPENAI_COMPATIBLE_API_KEY` | `HOSTED_OPENAI_COMPATIBLE_EMBEDDING_ENDPOINT` |
| `Cohere` | `HOSTED_COHERE_API_KEY` | Fixed `https://api.cohere.com/v2/embed` |
| `VoyageAI` | `HOSTED_VOYAGE_API_KEY` | Fixed `https://api.voyageai.com/v1/embeddings` |

For configurable endpoints, supply the service base URL, its `/v1` URL, or the
complete `/v1/embeddings` URL. Azure also accepts a resource base URL and uses
`/openai/v1/embeddings`. `remote.secret_name` can select another API key secret.
`remote.endpoint_secret_name` can select another endpoint secret for Internal,
Azure, Hugging Face, or a compatible service. The legacy `remote.endpoint` URL
does not select the destination.

Before upgrading, review existing Bits that use `CloudflareWorkersAI` or
`HuggingfaceEndpoint`. Earlier releases treated these names as aliases for
`Internal`; they now select the named provider. If a Bit should continue using
the internal gateway, set its implementation to `Internal` and preserve its
model ID. Bits intended for Cloudflare or Hugging Face need the provider secrets
listed above.

For example, this fragment routes an embedding Bit through Cloudflare:

```json
{
  "remote": {
    "implementation": "CloudflareWorkersAI",
    "model_id": "@cf/qwen/qwen3-embedding-0.6b"
  }
}
```

Keep the Bit's `vector_length`, `input_length`, query prefix, and document prefix
consistent with the selected deployment. The API adds the appropriate prefix
once and validates the returned vector dimensions and item count. Evaluate a
provider change before using its vectors with an existing index; matching
dimensions alone do not establish compatibility.

Cloudflare Qwen3 accepts at most 32 inputs per request. Cloudflare
EmbeddingGemma and BGE Large accept at most 100. The API rejects larger batches
before reserving usage. Cloudflare BGE Large requires `pooling: "Mean"`; create a
separate Bit and re-embed data when migrating an existing CLS index.

Cohere and Voyage AI receive the query or document task type through their native
API fields. Leave the Bit prefixes empty unless the deployment specifically needs
additional text. Cohere accepts at most 96 items per request; Voyage AI accepts
at most 1,000. Requests disable upstream truncation so oversized inputs return an
error instead of silently losing text.

External providers require an explicit `parameters.pricing` object. Choose
exactly one input tariff, expressed as integer micro-USD per million tokens or
million UTF-8 bytes. For example, a rate of USD 0.10 per million tokens is:

```json
{
  "pricing": {
    "input_micro_usd_per_million_tokens": 100000
  }
}
```

Set the amount to the deployment's actual tariff. Before dispatch, token pricing
reserves an allowance equal to the submitted UTF-8 byte count, including
prefixes, plus the configured input limit for every item. Settlement uses the
provider's reported token usage. If the provider omits usage, the cost remains
unresolved and the reservation is retained.

For an explicitly estimated byte tariff, use
`input_micro_usd_per_million_bytes` and a positive `max_input_bytes` limit for the
whole request, including prefixes. This mode settles from submitted bytes.
`request_micro_usd` optionally adds a fee for each request. All prices must be
nonnegative integers; an explicit zero means zero provider cost. Output token
pricing must be omitted or zero. Internal embeddings continue to use their
built-in estimated byte tariff.

Embedding responses expose `usage_available` when the provider reports token
usage. `usage_estimated` marks token counts estimated from submitted bytes. A
response with both flags false has no token usage to report; its zero counts do
not establish that inference was free. The Node.js and Python SDKs expose both
flags, with absent values when connected to older servers.

## Runtime cost estimates

App analytics and the admin usage dashboard price every run from its measured
duration. The reference sizing bills an x86_64 function holding 2 GB for the
whole run plus an arm64 function holding 1.2 GB for half of it, at AWS Lambda
list prices, including the per-invocation fee.

| Variable | Default | Purpose |
| --- | --- | --- |
| `COMPUTE_COST_MULTIPLIER` | `1.0` | Scales the whole estimate for deployments whose functions are sized differently |

A deployment that reserves twice the memory of the reference sizing sets
`COMPUTE_COST_MULTIPLIER=2`. The API reads the variable once per process, so a
change takes effect on restart. Values that are not a non-negative number are
logged and ignored. The estimate covers compute only: storage, network and
managed services are not included, and usage limits still apply to AI spend
alone.

## Validate configuration changes

```bash
python3 scripts/preflight.py
python3 scripts/up.py
docker compose ps --all
```

`up.py` starts with `--no-build`; pass `--build` only for installations whose
nine `*_IMAGE` values are all local tags.

Use `docker compose config --quiet` when checking interpolation manually.
Rendered configuration without `--quiet` contains deployment secrets.
