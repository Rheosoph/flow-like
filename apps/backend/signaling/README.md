# Flow-Like Signaling Server

WebSocket signaling server for Flow-Like realtime collaboration. It speaks the `y-webrtc` signaling protocol used by browser peers and runs either as a single self-contained Bun process or as multiple replicas that fan messages out through Redis.

> **Invariant: more than one replica requires `REALTIME_FANOUT_MODE=redis`.**
> A process cannot see its own replica count, so the deployment must declare it.

## What It Does

- Accepts authenticated WebSocket connections on `/`, `/ws`, and the client's
  credential-free `/ws/session/<room-digest>` path.
- Verifies the API-issued ES256 realtime JWT during the WebSocket upgrade and
  derives the client's only allowed topic from its `app_id` and `board_id`.
- Handles `subscribe`, `unsubscribe`, `publish`, and `ping` JSON messages.
- Publishes local messages to matching WebSocket subscribers.
- Fans messages out across other signaling replicas through Redis pub/sub, in
  `redis` fan-out mode only.
- Reports the subscriber count of a topic on every relayed message: from Redis
  presence in `redis` mode, from this process' own subscriber count in `local`
  mode.
- Exposes `/health` (liveness: `200` while the process serves) and `/ready`
  (readiness: `503` while the Redis fan-out is not fully connected).

The server does not store collaborative document state. Yjs document state lives in the connected peers; this service only helps peers discover and exchange signaling messages.

## Fan-Out Modes

`REALTIME_FANOUT_MODE` is explicit and never inferred from the presence of
`REDIS_*` variables. Unknown values abort startup.

| | `redis` (default) | `local` |
| --- | --- | --- |
| Replicas supported | any number | exactly one |
| Redis clients created | publisher, subscriber, presence | none — nothing is dialled |
| Cross-replica delivery | Redis pub/sub on `SIGNAL_CHANNEL` | none |
| `clients` count | sum of per-replica presence hashes in Redis | `server.subscriberCount(topic)` of this process |
| Presence heartbeat | every 10 s, TTL 90 s | not armed |
| `/health` liveness | always `200` while the process serves | always `200` |
| `/ready` readiness | `200` only while publisher, subscriber, and presence connections are ready; `503` otherwise, and new upgrades are refused with `503` | always `200` |
| Operational cost | a managed Redis instance, its network path, and its identity/access configuration | none |

Startup fails loudly when:

- `REALTIME_FANOUT_MODE` is neither `local` nor `redis`.
- `REALTIME_FANOUT_MODE=redis` and `REDIS_URL` is unset.

In `local` mode any `REDIS_URL` value is ignored and logged as a warning, since
messages then reach this replica's own sockets only.

## Requirements

- Bun 1.2 or newer.
- Redis 7 or compatible — only in `redis` fan-out mode.

## Configuration

| Variable | Default | Description |
| --- | --- | --- |
| `PORT` | `4444` | HTTP/WebSocket port. |
| `REALTIME_FANOUT_MODE` | `redis` | `redis` to fan out across replicas, `local` for a single replica with no Redis at all. |
| `REDIS_AUTH_MODE` | `local` | `local` for self-hosted Redis or `azure-entra` for keyless Azure Managed Redis. |
| `REDIS_URL` | none | Redis connection URL used for fanout and presence. Required in `redis` fan-out mode; ignored in `local`. Azure mode requires a credential-free `rediss://` URL. |
| `REDIS_CLUSTER_MODE` | `false` | Set to `true` only when the Redis database uses OSS cluster topology. |
| `SIGNAL_CHANNEL` | `signal:publish` | Redis pub/sub channel for cross-node message fanout. |
| `NODE_ID` | random UUID | Stable node identifier used to avoid echoing messages back from Redis and to store per-node presence counts. |
| `AZURE_CLIENT_ID` | none | Azure mode only: client ID of the user-assigned managed identity. |
| `AZURE_TOKEN_CREDENTIALS` | enforced | Azure mode only: when supplied, must be `ManagedIdentityCredential`. |
| `AZURE_REDIS_HOST_SUFFIX` | `.redis.azure.net` | Azure mode only: trusted endpoint suffix; override only for the applicable sovereign Azure cloud. |
| `BACKEND_PUB` | none | Standard-base64 encoded ES256 PEM public key used by the API. Required unless insecure local development is explicitly enabled. |
| `BACKEND_KID` | none | Optional exact JWT key ID pin. Recommended for deployed environments. |
| `REALTIME_JWT_ISSUER` | `flow-like` | Exact accepted realtime JWT issuer. |
| `REALTIME_JWT_AUDIENCE` | `y-webrtc` | Exact accepted realtime JWT audience. |
| `REALTIME_ALLOWED_ORIGINS` | none | Comma-separated, exact browser origins. Required in authenticated mode; wildcards are rejected. Besides exact HTTP(S) origins, exactly three desktop literals are accepted: `tauri://localhost`, `http://tauri.localhost`, `https://tauri.localhost` (see below). |
| `REALTIME_MAX_CONNECTIONS_PER_SUB` | `16` | Maximum concurrent WebSocket connections per authenticated subject (token `sub`). Over-cap upgrades are rejected with HTTP `429`. Integer between 1 and 10000. |
| `REALTIME_ALLOW_INSECURE_LOCAL_DEV` | `false` | Explicit escape hatch for local development only. Rejected with `NODE_ENV=production` or Azure Redis authentication. |

### Desktop (Tauri) Origins

The Flow-Like desktop app connects from a Tauri WebView, which sends
`Origin: tauri://localhost` on macOS/Linux and `http://tauri.localhost` or
`https://tauri.localhost` on Windows. To admit desktop clients, add exactly
those literals to `REALTIME_ALLOWED_ORIGINS`, for example:

```text
REALTIME_ALLOWED_ORIGINS=https://app.example.com,tauri://localhost,http://tauri.localhost,https://tauri.localhost
```

These three strings are matched as exact literals — there is no wildcard and no
general non-HTTP(S) scheme support, and an upgrade without an `Origin` header
is still rejected.

## Run Locally

A single local process needs no Redis. The normal path is authenticated and
uses the same public-key values as the API plus the browser origin:

```bash
cd apps/backend/signaling
bun install
export REALTIME_FANOUT_MODE=local
export BACKEND_PUB='<standard-base64-public-pem>'
export BACKEND_KID='backend-es256-v1'
export REALTIME_ALLOWED_ORIGINS='http://localhost:3000'
bun run start
```

To exercise cross-replica fan-out locally, start Redis and select `redis` mode:

```bash
docker run --rm -p 6379:6379 redis:7-alpine
REALTIME_FANOUT_MODE=redis REDIS_URL=redis://127.0.0.1:6379 bun run start
```

For a deliberately unauthenticated local protocol test only, opt in explicitly:

```bash
NODE_ENV=development \
REALTIME_FANOUT_MODE=local \
REALTIME_ALLOW_INSECURE_LOCAL_DEV=true \
bun run start
```

The insecure mode is off by default, is incompatible with Azure Redis mode,
and must not be used for shared or Internet-accessible environments.

The WebSocket endpoint is:

```text
ws://localhost:4444
```

Health checks can use:

```bash
curl http://localhost:4444/health   # liveness
curl http://localhost:4444/ready    # readiness
```

`/health` is pure liveness: it returns `200` as long as the process serves, so
a restart-on-failure supervisor never kills live WebSockets over a Redis
outage. `/ready` is the readiness check for orchestrators that gate traffic:
in `redis` fan-out mode it returns `200` only while the publisher, presence,
and subscriber Redis connections are ready, and `503` otherwise. While fan-out
is unready, NEW upgrades on `/`, `/ws`, `/ws/`, and `/ws/session/…` are
rejected with `503`, but established sockets stay open and keep local
delivery. In `local` mode the process has no external dependency, so both
endpoints return `200` as soon as it listens. Unknown HTTP paths always return
`404`.

## Azure Managed Redis

The Azure image is built through:

```text
apps/backend/azure/signaling/Dockerfile
```

Configure the Container App with a user-assigned managed identity and these
non-secret values:

```yaml
REALTIME_FANOUT_MODE: redis
REDIS_AUTH_MODE: azure-entra
REDIS_URL: rediss://<cache-hostname>:<tls-port>
REDIS_CLUSTER_MODE: "false"
AZURE_CLIENT_ID: <managed-identity-client-id>
AZURE_TOKEN_CREDENTIALS: ManagedIdentityCredential
BACKEND_PUB: <standard-base64-public-pem>
BACKEND_KID: backend-es256-v1
REALTIME_JWT_ISSUER: flow-like
REALTIME_JWT_AUDIENCE: y-webrtc
REALTIME_ALLOWED_ORIGINS: https://<frontend-origin>
REALTIME_ALLOW_INSECURE_LOCAL_DEV: "false"
```

That configuration is for the zone-spread, multi-replica profile. A container
app pinned to exactly one replica instead sets `REALTIME_FANOUT_MODE: local`,
drops every `REDIS_*` and
`AZURE_*` value above, and provisions no cache at all. Scaling that app beyond
one replica without switching back to `redis` mode is a startup error.

`REDIS_CLUSTER_MODE=false` matches the deployment module's `NoCluster` policy.
Use `true` only if the Azure Managed Redis database is explicitly changed to an
OSS cluster policy.

Azure mode fails during startup when the endpoint is plaintext, contains user
information, uses an IP address, falls outside the trusted Azure Redis DNS
suffix, or when any Redis access key/service-principal secret variable is set.
It uses `DefaultAzureCredential` restricted to the assigned managed identity,
TLS certificate/hostname verification, and RESP3 streaming credentials. Tokens
refresh at 70% of their lifetime and the Redis client sends `AUTH` again on the
publisher, command, and pub/sub connections without falling back to access
keys.

The infrastructure must also:

- Disable Azure Managed Redis access-key authentication.
- Assign the signaling identity to the cache's Redis access policy.
- Provide private-endpoint DNS and network reachability from the Container Apps environment.
- Keep the public Redis endpoint disabled.

Redis pub/sub is intentionally ephemeral; it is cross-node signaling transport,
not authoritative document or execution state.

## Docker Compose

The self-hosted backend compose stack builds this service through:

```text
apps/backend/docker-compose/signaling/Dockerfile
```

The compose service sets:

```yaml
PORT: 4444
REDIS_URL: redis://redis:6379
SIGNAL_CHANNEL: signal:publish
```

It relies on the default `REALTIME_FANOUT_MODE=redis`; scaling the signaling
service to more than one replica needs no further fan-out change because every
replica already shares the compose stack's Redis.

The compose stack runs the service in authenticated mode (`NODE_ENV=production`
by default), so two variables are **required** in `.env` or the container
crashloops at boot:

```bash
# Standard-base64 ES256 public PEM, shared with the api service
BACKEND_PUB=<standard-base64-public-pem>
# Exact browser origins; the compose web app serves on port 3001
REALTIME_ALLOWED_ORIGINS=http://localhost:3001
```

Add the Tauri desktop literals to `REALTIME_ALLOWED_ORIGINS` when desktop
clients connect to this stack (see "Desktop (Tauri) Origins" above). For an
unauthenticated local protocol test instead, set
`SIGNALING_NODE_ENV=development` plus `REALTIME_ALLOW_INSECURE_LOCAL_DEV=true`.
`BACKEND_KID` defaults to `backend-es256-v1`, matching the api and compiler
services, so key-ID pinning is on by default.

The default runtime config advertises the signaling endpoint to clients as:

```json
"signaling": ["ws://localhost:4444"]
```

For public HTTPS deployments, expose this service through a TLS-capable reverse proxy and configure clients with the public `wss://...` endpoint.

## Protocol

The Flow-Like web client connects to a non-secret room-digest path and carries
the short-lived realtime JWT in `Sec-WebSocket-Protocol`; it never places the
JWT in a URL, query string, log message, or peer awareness state. The server
returns only the stable `flowlike.realtime.v1` subprotocol. It validates the
JWT's ES256 signature, exact issuer and audience, key ID (when pinned), time
claims, token type, scope, subject, `app_id`, and `board_id` before upgrading.

The authorized Redis topic is exactly `<app_id>:<board_id>`. Subscribe,
unsubscribe, or publish attempts for another topic close the connection.
Authenticated connections may hold one topic, receive at most 64 KiB per
message and 10,000 messages total, and are bounded by ten-second message and
publish windows. Each authenticated subject may hold at most
`REALTIME_MAX_CONNECTIONS_PER_SUB` (default 16) concurrent connections;
upgrades beyond the cap are rejected with HTTP `429`. Connections close when
the access token expires.

Subscribe to one or more topics:

```json
{ "type": "subscribe", "topics": ["room-id"] }
```

Publish to a topic:

```json
{ "type": "publish", "topic": "room-id", "data": { "any": "payload" } }
```

Unsubscribe from topics:

```json
{ "type": "unsubscribe", "topics": ["room-id"] }
```

Ping the server:

```json
{ "type": "ping" }
```

The server responds to `ping` with:

```json
{ "type": "pong" }
```

Published messages delivered to subscribers include:

- `clients`: subscriber count for the topic — global, from Redis presence, in
  `redis` mode; this replica's own subscriber count in `local` mode, where it
  is the same number because there is only one replica.
- `_origin`: node identifier of the publishing signaling node.

Messages larger than 64 KiB are rejected with close code `1009`. Invalid JSON,
unknown message types, unauthorized topics, and rate-limit violations close the
connection rather than being silently ignored.

### Rollout / Compatibility

The authenticated protocol is breaking in both directions: the new client
connects only through `/ws/session/<room-digest>` and requires the server's
`flowlike.realtime.v1` subprotocol echo, while clients from before this
release send no token and are rejected with `401`. Plan the rollout as one
step:

- Deploy the new signaling server and the new web UI in the same window. An
  old server fails every new client's upgrade; a new server rejects every old
  client.
- Before switching traffic, the signaling environment must carry `BACKEND_PUB`
  and `BACKEND_KID` matching the API's realtime token signer, and
  `REALTIME_ALLOWED_ORIGINS` must cover the web origin plus the three Tauri
  desktop literals (`tauri://localhost`, `http://tauri.localhost`,
  `https://tauri.localhost`).
- Desktop builds older than this release lose realtime collaboration until
  updated; everything else keeps working. Treat this release as the minimum
  desktop version for realtime and communicate it as such.

## Tests

The unit and fan-out suites need neither a running server nor Redis:

```bash
cd apps/backend/signaling
bun test
```

`tests/fanout.test.ts` starts real server processes and asserts that `local`
mode opens no TCP connection to the configured Redis endpoint, stays healthy,
accepts upgrades, relays between two sockets on the same process, and reports
the correct `clients` count.

The remaining scripts expect a running server, and Redis for the fan-out ones:

```bash
bun run test:auth
bun run test:ws
bun run test:comprehensive
bun run test:redis-fanout
bun run test:multi-worker
```

Browser-based manual tests are also available:

- `tests/test-client.html`
- `tests/test-yjs-webrtc.html`

The public endpoint smoke test uses `wss://signaling.flow-like.com`:

```bash
bun run tests/test-public-wss.mjs
```

## Scaling Notes

Running more than one replica requires `REALTIME_FANOUT_MODE=redis` plus a
shared `REDIS_URL` pointing every replica at the same Redis instance. With
`local` fan-out a second replica
receives its own disjoint set of peers: rooms split silently, and `clients`
counts only the peers that happen to share a replica. The process cannot detect
this by itself, which is why the deployment declares it.

All signaling nodes must then share the same Redis instance and `SIGNAL_CHANNEL`. Each node keeps local WebSocket subscriptions in memory and uses Redis for cross-node fanout and presence. Presence keys use the `topic:presence:` prefix and expire after 90 seconds; active nodes refresh their topic presence every 10 seconds.

When running multiple processes on one host, use unique `NODE_ID` values or allow the server to generate random IDs. Behind a load balancer, keep normal WebSocket proxying enabled and forward upgrade headers. In particular, preserve `Origin` and `Sec-WebSocket-Protocol`; do not log the protocol header because it carries a bearer credential during the upgrade.
