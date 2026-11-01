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

## Run reply channels

Interactive runs receive replies over NATS. The executor subscribes before it
asks the client for input, and the client waits for an executor acknowledgement.
The API signs credentials limited to that run's inbox and acknowledgement
subjects. The signing seed stays in the API; the broker receives only public
operator and account tokens.

`setup-env.py` uses OpenSSL with Ed25519 support to create the trust
configuration and sets these values:

| Variable | Purpose |
| --- | --- |
| `CHANNEL_TRANSPORT=nats` | Select NATS for interactive channel grants |
| `CHANNEL_NATS_URL=nats://nats:4222` | Private broker endpoint used by the API and execution gateways |
| `CHANNEL_NATS_WS_URL` | Browser endpoint, derived from the public API origin with `/nats` |
| `CHANNEL_NATS_ACCOUNT_SEED` | API signing seed; keep it private |
| `NATS_SERVER_CONFIG` | Base64-encoded broker configuration with public trust and exact browser origins |
| `CHANNEL_TTL_SECONDS=7200` | Upper bound for channel lifetime |

The edge proxy routes `/nats` to the broker's WebSocket listener. Keep that
route when adding an external reverse proxy and use `wss://` with HTTPS hosting.
The native broker and monitoring ports are not published. Isolated executors
reach the configured broker through their per-run gateway.

The broker accepts HTTP(S) browser origins. Desktop clients using
`tauri://localhost` send replies through the authenticated HTTP fallback, which
forwards them to NATS. Collaboration signaling remains a separate service.

For an existing installation, generate only the new NATS settings:

```bash
python3 ../scripts/nats_bootstrap.py --output .nats.env \
  --web-origin https://app.example.com \
  --websocket-url wss://api.example.com/nats
```

Merge those assignments into the existing private `.env`, then recreate the
API, execution manager, edge proxy and NATS services. The helper refuses to
overwrite its output. Changing the web origin also requires updating
`websocket.allowed_origins` in the decoded broker configuration and recreating
NATS; changing an API CORS value alone does not update broker origins.

The bundled broker runs as one instance. Core NATS does not retain replies
during an outage; acknowledgements and bounded retries detect failed delivery.
The HTTP fallback still needs the broker. Use an externally managed NATS
cluster when broker availability must survive a node failure, and set the API
signer to an account trusted by that cluster. Use a stable internal Service or
load-balancer endpoint. Isolated gateways support `nats://` on the trusted
private network; use `trusted_shared` mode for direct native `tls://` connections.
Setting `CHANNEL_TRANSPORT=http`
selects the database-backed polling transport instead.

### Test the channel transport

From the repository root, install the workspace JavaScript dependencies and
make `nats-server` available on `PATH`. The browser suite uses Puppeteer's
Chromium; `CHROME_PATH` selects an existing Chrome executable.

```bash
cargo build -p flow-like-channels --features nats --example nats-channel-worker
python3 tests/nats-channels-browser/run.py
NATS_SERVER_BIN="$(command -v nats-server)" cargo test -p flow-like-channels \
  --features nats real_broker_reply_steer_cancel_forwarder_permissions_and_expiry \
  -- --include-ignored
```

The browser runner starts a fresh authenticated broker and exercises replies,
steering, cancellation, duplicate delivery, HTTP forwarding, channel isolation
and expiry. `NATS_SERVER_BIN` and `NATS_WORKER_BIN` can select custom binaries.
To exercise the actual execution gateway too, build
`cargo build -p flow-like-execution-manager --bin execution-gateway` and run
with `NATS_GATEWAY_BIN=target/debug/execution-gateway`.

To test an already running Compose or Kubernetes broker, run
`bun tests/nats-channels-browser/verify.ts --config /private/path/fixture.json`.
The private JSON file must contain `url`, `ws_url`, `account_seed` and
`channel_id`. Set `NATS_E2E_BROWSER_PORT` to a free local port and include
`http://127.0.0.1:<port>` in that test broker's origin list. Use an isolated test
deployment and remove its credentials after testing.

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

External OIDC is the default (`OIDC_MODE=external`), with no bundled identity
services. `setup-env.py --oidc bundled` adds Keycloak and its dedicated PostgreSQL
database using `docker-compose.oidc.yml`. Both modes use the same Flow-Like OIDC
authentication path.

`FLOW_LIKE_RUNTIME_CONFIG_FILE` selects the host-side JSON file mounted into the
API and sink services at `/app/flow-like.config.json`. The API selects that mount
through `FLOW_LIKE_CONFIG_FILE`. Replace the example's OIDC authority, client
settings, public domains, legal links and signaling URL when using an external
provider. Set `user_info_url` explicitly as well as `jwks_url` and
`discovery_url`. The API reads the whole
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

### Bundled identity provider

Generate a new environment with `--oidc bundled`. Setup generates a `flow-like`
realm for Keycloak to import, with a public `flow-like` client, authorization-code login and PKCE S256.
The client adds the expected audience to access tokens so the API can validate
them. Password grants and public account registration are disabled. The
Keycloak administrator manages users; the generated `owner` account can sign
in to Flow-Like and must change its temporary password on first use.

| Setting | Purpose |
| --- | --- |
| `OIDC_MODE` | `external` by default; `bundled` requires the OIDC overlay |
| `OIDC_PUBLIC_URL` | Stable HTTPS origin, default `https://auth.localhost:8443` |
| `OIDC_HOSTNAME`, `OIDC_PORT` | Hostname and TLS listener port derived from that origin |
| `OIDC_BIND_ADDRESS` | Published listener address, default `127.0.0.1` |
| `OIDC_IMAGE` | Keycloak image reference |
| `OIDC_DB_PASSWORD` | Password for the separate identity database |
| `OIDC_ADMIN_USERNAME`, `OIDC_ADMIN_PASSWORD` | Initial Keycloak administrator credentials |
| `OIDC_BOOTSTRAP_USERNAME`, `OIDC_BOOTSTRAP_PASSWORD` | Initial Flow-Like sign-in account and temporary password |
| `OIDC_BOOTSTRAP_EMAIL` | Initial account email selected with `--oidc-bootstrap-email` |
| `OIDC_REALM_FILE` | Generated realm import file |
| `OIDC_TLS_CERT_FILE`, `OIDC_TLS_KEY_FILE` | Host-side server certificate chain and private key |
| `OIDC_CA_CERT_FILE` | Host-side CA bundle mounted for the API's OIDC requests |

For `--output .env.local`, setup creates `.env.local.oidc/` with mode `0700`.
Its CA private key has mode `0600`; mounted files have mode `0644` so the
container users can read them. Keep the directory private. Setup refuses to
replace the environment file or artifact directory. Passwords remain in the
private environment file and are not written into the public hub configuration.

The local certificate files are `ca.crt`, `tls.crt` and `tls.key`. Trust `ca.crt`
in the host operating system and browser before signing in. The API trusts the
mounted CA only for OIDC requests and still verifies certificates and hostnames.
The desktop client uses host trust. Setup never installs a CA into a workstation
trust store.

To use a public hostname, supply its certificate chain, matching private key
and CA bundle when generating the installation:

```bash
python3 scripts/setup-env.py --oidc bundled \
  --web-origin https://app.example.com \
  --api-url https://api.example.com \
  --s3-endpoint https://storage.example.com \
  --oidc-url https://auth.example.com:8443 \
  --oidc-tls-cert /private/certs/auth-fullchain.pem \
  --oidc-tls-key /private/certs/auth-key.pem \
  --oidc-ca-cert /private/certs/ca-bundle.pem \
  --oidc-bootstrap-email owner@example.com
```

Provide all three certificate arguments together, using an unencrypted private
key. Setup copies these files into the generated artifact directory; renewing
the original source files alone does not update the deployment. Review `OIDC_BIND_ADDRESS`
before exposing the listener beyond loopback. Browsers and containers use the
same HTTPS origin; the Compose network alias resolves its hostname to Keycloak
inside the stack. Keep this issuer origin stable because tokens identify their
issuer by that exact URL. Changing providers or issuers does not migrate
existing Flow-Like accounts.

Review the generated hub's signaling address and legal links for public hosting.
Local certificates expire after one year. Replace the server certificate and key
before expiry, preserving the CA trusted by clients, then recreate `oidc`. For a
CA change, update client trust and recreate the API with the new CA bundle too.

The generated hub uses `flow-like://auth` for native sign-in. Web runtime
configuration overrides this with the web origin's `/callback` path. Both exact
callbacks are registered in Keycloak. A desktop build with a different
`NEXT_PUBLIC_REDIRECT_URL` needs that exact callback registered too; repeat
`--oidc-redirect-uri <uri>` during setup for each additional callback. Logout
returns to the web origin. No wildcard callback is generated.

Keycloak imports the realm on its first startup and skips it when that realm
already exists. Later edits to the import file or bootstrap passwords do not
change existing accounts or clients. Use the Keycloak administration console
for those changes; do not delete the database volume to update settings.
Back up the identity database, environment file and artifact directory together.
See [Keycloak realm import](https://www.keycloak.org/server/importExport) for
the import lifecycle.

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
| `OpenRouter` | `OPENROUTER_API_KEY` | Fixed `https://openrouter.ai/api/v1/embeddings` |
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

For EmbeddingGemma 2, set `remote.implementation` to `Internal` and
`remote.model_id` to `embeddinggemma-2`. The `/embeddings/embed` API accepts text
strings, structured image/audio/video inputs, combined multimodal inputs, and
mixed batches for this model. Media fields contain URLs or base64 strings; the
API forwards them to the gateway without downloading them.

For Gemini Embedding 2 on OpenRouter, set `remote.implementation` to `OpenRouter`
and `remote.model_id` to `google/gemini-embedding-2`. The API uses
`OPENROUTER_API_KEY`; the chat setting `OPENROUTER_ENDPOINT` does not change the
embedding destination. This model supports text, images, audio, video, and PDF
files. An input with `type: "content"` and ordered `parts` produces one combined
vector, including repeated image parts. Existing structured image/audio/video
and `multimodal` inputs remain accepted. Other hosted providers retain their
text input contract. See [embedding models](/topics/genai/models/#embedding-models)
for source formats, media limits, and workflow nodes, and the
[Node.js](/dev/sdks/nodejs/#embeddings) or [Python](/dev/sdks/python/#embeddings)
SDK examples for ordered content.

For OpenRouter models with a fixed output size, turn off **Request vector
dimensions** in the Bit editor or set `remote.supports_dimensions` to `false`.
The API then omits the upstream `dimensions` parameter. Set the Bit's
`vector_length` to the model's native output size; every returned vector must
still match that length. When `supports_dimensions` is absent, null, or `true`,
the API requests the Bit's `vector_length`, preserving existing behavior.

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

Cohere, Voyage AI, and OpenRouter receive the query or document task type through
their native API fields. Leave the Bit prefixes empty unless the deployment
specifically needs additional text. Cohere accepts at most 96 items per request;
Voyage AI accepts at most 1,000. Cohere and Voyage AI requests disable upstream
truncation so oversized inputs return an error instead of silently losing text.

External providers require an explicit `parameters.pricing` object. For providers
other than OpenRouter, choose exactly one input tariff, expressed as integer
micro-USD per million tokens or million UTF-8 bytes. For example, a rate of
USD 0.10 per million tokens is:

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
nonnegative integers; zero rates produce zero cost when settlement uses those
configured rates. Output token pricing must be omitted or zero. Internal
embeddings continue to use their built-in estimated byte tariff.

OpenRouter requires token pricing. Set `input_micro_usd_per_million_tokens` to
the highest input rate among the model's enabled modalities and upstream
providers so the reservation covers media. For `google/gemini-embedding-2`, the
API reserves the full 8,192-token context for each item at that rate; encoded
file lengths do not determine the reservation. Other OpenRouter models use the
text reservation described above. The API settles against OpenRouter's reported
`usage.cost`, which accounts for the modalities actually processed, and adds
`request_micro_usd` if configured. Missing or invalid cost leaves the reservation
unresolved. Byte pricing is not supported for OpenRouter. Check the
[model's current provider prices](https://openrouter.ai/google/gemini-embedding-2)
before setting the reservation rate; a text-only rate does not cover audio or
video.

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
