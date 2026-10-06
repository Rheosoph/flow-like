---
title: Realtime signaling
description: Configure authenticated collaboration, Redis fanout and signaling rollouts
---

The signaling server lets browser and desktop peers discover each other and
exchange `y-webrtc` messages for realtime collaboration. Yjs document state lives
in the peers; the server does not store it. Run one Bun process with local
delivery, or use Redis pub/sub to relay messages between replicas.

## Choose fanout and configure identity

`REALTIME_FANOUT_MODE=redis` is the default and requires `REDIS_URL`.
`REALTIME_FANOUT_MODE=local` creates no Redis clients and supports exactly one
replica. A second local-mode replica silently splits rooms because each process
sees only its own sockets. The server cannot discover its replica count, so the
deployment must enforce this choice. Unknown modes stop startup; local mode
ignores a configured Redis URL and logs a warning.

All replicas must use the same API public signing key, expected JWT claims,
Redis instance and `SIGNAL_CHANNEL`.

| Variable | Default | Purpose |
| --- | --- | --- |
| `PORT` | `4444` | HTTP/WebSocket listener |
| `REALTIME_FANOUT_MODE` | `redis` | Redis fanout or single-process `local` mode |
| `REDIS_URL` | none | Required Redis URL in Redis fanout mode |
| `REDIS_AUTH_MODE` | `local` | Self-hosted Redis authentication, or `azure-entra` |
| `REDIS_CLUSTER_MODE` | `false` | Enable only for an OSS cluster topology |
| `SIGNAL_CHANNEL` | `signal:publish` | Shared Redis pub/sub channel |
| `NODE_ID` | Random UUID | Unique replica identity for fanout and presence |
| `BACKEND_PUB` | none | Standard-base64 encoded ES256 public PEM matching the API |
| `BACKEND_KID` | none | Optional exact JWT key ID pin |
| `REALTIME_JWT_ISSUER` | `flow-like` | Exact accepted issuer |
| `REALTIME_JWT_AUDIENCE` | `y-webrtc` | Exact accepted audience |
| `REALTIME_ALLOWED_ORIGINS` | none | Required comma-separated exact browser/desktop origins |
| `REALTIME_MAX_CONNECTIONS_PER_SUB` | `16` | Per-process WebSocket limit for an authenticated subject; integer 1–10000 |
| `REALTIME_ALLOW_INSECURE_LOCAL_DEV` | `false` | Unauthenticated local protocol testing only |

Use HTTP(S) origins without wildcards. Desktop clients additionally use exactly
`tauri://localhost`, `http://tauri.localhost` or `https://tauri.localhost`:

```dotenv
REALTIME_ALLOWED_ORIGINS=https://app.example.com,tauri://localhost,http://tauri.localhost,https://tauri.localhost
```

Other non-HTTP(S) schemes and missing `Origin` headers are rejected. Public
deployments need a WSS endpoint in the Hub configuration. Preserve `Origin` and
`Sec-WebSocket-Protocol` through the load balancer. Do not log the protocol header:
it carries a bearer credential during the upgrade.

Compose generates the shared public key, uses Redis fanout and pins
`BACKEND_KID=backend-es256-v1` by default. Its web origin is
`http://localhost:3001`. Helm deployments configure `signaling.fanoutMode` and
`signaling.allowedOrigins`. See the deployment-specific
[Compose configuration](/self-hosting/docker-compose/configuration/) or
[Helm configuration](/self-hosting/kubernetes/configuration/).

## Health, presence and outages

### Encrypted device data and service connections

Device service tunnels use the authenticated `/ws/devices` endpoint for discovery
and relay fallback. Clients try an ordered WebRTC data channel first. A separate
Noise session encrypts tunnel payloads between Studio and the device agent; the
signaling server can see routing identities and traffic size, but cannot read
service requests, deployment files or device data.

Studio uses these streams for initial device inspection, log and message reads,
retained metrics, encrypted telemetry and archive reads. Deployment manifests and
files travel as binary streams. Upload creation, checkpoints, commit and service
activation remain management operations. Uploads retain their transfer identity,
verified file hashes and device-reported offsets, so an interrupted upload can
resume after reconnecting without restarting completed files.
Upgrade device agents before rolling out updated Studio. Studio checks the agent's
authenticated streaming capability before opening a data tunnel and requires it
for these transfers.

Open a deployment's **Endpoint** tab and use **Connect through Studio** to send
HTTP requests and read streaming responses in web or Desktop Studio. Select a
configured listener, enter a request path and supply any application credentials
in the request headers. The request tool does not follow redirects, use browser
cookies or upgrade to WebSocket. Request bodies are limited to 10 MiB; HTTP headers
are limited to 64 KiB, with a 32 KiB limit on the page's custom-header input.
Responses stream without a total body-size limit. The page retains a bounded text
excerpt and continues consuming the stream. Opening a connection has a 30-second
deadline, request/header progress a 60-second idle deadline, and each pending
response-body read a five-minute deadline. Use **Stop request** to end a stream.

Select the `hosting` listener and choose **Open deployed app** to use the deployed
Pages, Chat, forms and quick actions in either Web or Desktop Studio. The app uses
the device's published event versions and streams execution results into Studio's
existing interfaces. Page actions, workflow prompts, attachments and permitted
media downloads use the same encrypted service tunnel. The service's access token
is requested when required and stays in memory for that open session.
Protected assets are limited to 32 MiB each and 128 MiB across an open session,
with at most 64 cached assets. Downloads address the app's upload area or an
expiring reference issued by the running deployment. Attachments are limited to
about 3.5 MiB each, and the encoded request including chat history must fit within
10 MiB. Package micro-widgets that require Studio registry access are unavailable
in this view.

Each open app has its own query cache, navigation state and history identity.
Closing it, locking device keys or losing the live device session cancels its
requests, revokes media URLs and clears that session's conversations and Page
data. Reopening starts a new session; interrupted runs are not automatically
replayed. Studio's other app histories remain separate.

The rich interface requires hosting to be configured on the deployment. For a
deployment containing only forms or quick actions, open **Limits** in the deploy
wizard and enable **Open these forms and quick actions as a deployed app**. Review
the listener and access settings before deploying. Leaving this option off keeps
the existing **Run now** workflow. An additional named TCP listener does not
create this interface.
Scheduled events, bots and background services continue running on the device;
their logs and status travel over the data tunnel, and their existing management
controls remain available in Studio.

Desktop Studio also offers **Open local port** for a database client, browser or
another local application. Choose a port, or use `0` to allocate an available one.
The listener binds only to `127.0.0.1`. Other processes on that computer can reach
it, so application authentication still applies. Each accepted connection opens a
separate encrypted device stream. Leaving the service page, locking device keys,
closing the desktop window or losing the live session closes the local forwarding
operation. A native 30-second lease also expires if the page stops renewing it.
A dropped application connection is not replayed.

Service connections and listener discovery require `service_connect` within the
deployment's placement, project or device scope. Editing listeners requires
`deploy`; connecting does not grant access to deployment settings. The device
resolves a placement and service ID to a configured listener in its current,
ready deployment revision. Clients cannot choose a destination address.

The `hosting` service is included when native hosting is configured. Under
**Additional service listeners**, deployment editors can name up to sixteen
existing loopback listeners, using unique IDs and `tcp`, `http` or `https` as the
protocol. This can expose a local database or another deployed API. Saving the
configuration does not start that listener. The reserved `hosting` ID cannot be
reused. A scoped `service_connect` grant permits the configured services in that
scope, rather than an arbitrary port on the device.

Noise encryption ends at the device agent. Plain HTTP or TCP continues to its
configured local listener. For HTTP requests to HTTPS services, the agent opens
TLS upstream and verifies the certificate name, validity and exact SHA-256 leaf
fingerprint. Native hosting uses the currently bound device certificate. An
additional HTTPS listener requires a configured TLS server name and certificate
fingerprint; update its fingerprint when that certificate changes. Desktop port
forwarding carries raw TCP bytes, so a client's own TLS and certificate checks
continue through the tunnel unchanged.

Each tunnel supports up to sixteen concurrent byte streams with a 256 KiB receive
window per stream. The receiver returns credit as it consumes bytes. Desktop
allows eight local listeners, sixteen connections per listener and sixty-four
local connections in total; the tunnel's stream limit may be reached first.

Internal data streams use the existing permissions for each operation. Uploads
require `deploy` access to the project and belong to the controller that began the
transfer. Reading device data keeps the same scope and capability checks as a
management read; it does not require `service_connect`.

Tunnel frames use separate relay queues and bandwidth budgets from management
frames. The default relay allowance is 8 MiB/s of encoded traffic per account and
device, with a 32 MiB/s device output allowance. Pending tunnel queues share a
128 MiB limit per signaling process. These are resource limits, not measured
throughput guarantees.

Clients renew signaling credentials on the existing socket and rotate Noise keys
without reopening service connections. The device checks current access during
renewal and periodically while streams remain open. Expired or revoked access,
queue overflow, missing frames and heartbeat failure close affected streams.
Redis pub/sub provides no replay: callers must open a new connection after a
transport failure, and must decide whether an application operation can be retried.

### Readiness and collaboration presence

| Endpoint | Meaning |
| --- | --- |
| `/health` | Returns 200 while the process serves, regardless of Redis connectivity |
| `/ready` | In Redis mode, returns 200 only when publisher, subscriber and presence clients are ready; otherwise 503 |

In local mode both endpoints return 200 once the server listens. In Redis mode,
lost readiness refuses new WebSocket upgrades with 503. Existing sockets remain
open and can still deliver locally. Use `/ready` for traffic admission and
`/health` for process liveness so a Redis outage does not trigger repeated
disconnects through container restarts. Unknown HTTP paths return 404.

Redis presence hashes use `topic:presence:`, refresh every ten seconds and expire
after ninety seconds. Relayed messages include `clients`, the sum of the
per-replica presence counts, and `_origin`, the publishing node ID. Local mode
reports only the process's own subscriber count. Redis pub/sub is ephemeral and
does not provide durable collaboration history.

## Run and test locally

Install Bun 1.2 or newer. Redis 7 or a compatible service is needed only for
Redis fanout. From the repository root:

```sh
cd apps/backend/signaling
bun install --frozen-lockfile
export REALTIME_FANOUT_MODE=local
export BACKEND_PUB='<standard-base64-public-pem>'
export BACKEND_KID=backend-es256-v1
export REALTIME_ALLOWED_ORIGINS=http://localhost:3000
bun run start
```

Supply the actual public key used by the local API. The server listens at
`ws://localhost:4444`.

```sh
curl --fail http://localhost:4444/health
curl --fail http://localhost:4444/ready
```

To exercise fanout, start a local Redis process and launch each signaling
replica with `REALTIME_FANOUT_MODE=redis`, a shared `REDIS_URL` and its own port.
Leave `NODE_ID` unset for unique generated IDs, or assign distinct values.

For an isolated, unauthenticated protocol test, use
`NODE_ENV=development REALTIME_FANOUT_MODE=local REALTIME_ALLOW_INSECURE_LOCAL_DEV=true bun run start`.
This mode is rejected in production and with Azure Redis authentication. Keep
it off shared or Internet-accessible endpoints.

Run unit and local fanout tests with `bun test`. The tests start their own local
processes and do not require a separately running Redis service. Additional
scripts exercise an already running server:

```sh
bun run test:ws
bun run test:comprehensive
bun run test:redis-fanout
bun run test:multi-worker
```

The last two need Redis fanout. Browser fixtures are
`apps/backend/signaling/tests/test-client.html` and
`apps/backend/signaling/tests/test-yjs-webrtc.html`. The separate
`tests/test-public-wss.mjs` script targets the public hosted endpoint; use local
tests when checking changes without contacting that service.

## Azure Managed Redis

The Azure signaling image uses
`apps/backend/azure/signaling/Dockerfile`. Assign its Container App a
user-assigned managed identity and configure:

```yaml
REALTIME_FANOUT_MODE: redis
REDIS_AUTH_MODE: azure-entra
REDIS_URL: rediss://<cache-hostname>:<tls-port>
REDIS_CLUSTER_MODE: "false"
AZURE_CLIENT_ID: <managed-identity-client-id>
AZURE_TOKEN_CREDENTIALS: ManagedIdentityCredential
BACKEND_PUB: <standard-base64-public-pem>
BACKEND_KID: backend-es256-v1
REALTIME_ALLOWED_ORIGINS: https://app.example.com
REALTIME_ALLOW_INSECURE_LOCAL_DEV: "false"
```

The URL must use TLS, contain no credentials, name a DNS host rather than an IP,
and end in the trusted `.redis.azure.net` suffix. Set `AZURE_REDIS_HOST_SUFFIX`
only for the applicable sovereign-cloud endpoint. Static Redis keys and Azure
service-principal secrets cause startup failure. `AZURE_TOKEN_CREDENTIALS`, when
set, must equal `ManagedIdentityCredential`.

The client restricts `DefaultAzureCredential` to the assigned managed identity,
verifies TLS certificates and hostnames, and refreshes tokens at 70% of their
lifetime. It reauthenticates its RESP3 connections without an access-key fallback.
The infrastructure must disable Redis access-key authentication, grant the
identity Redis data access, and supply private-endpoint DNS and connectivity.
Keep the public Redis endpoint disabled for that private deployment.

Use `REDIS_CLUSTER_MODE=true` only when the database actually uses OSS cluster
policy. An installation fixed at one signaling replica can choose local fanout
and omit Redis and Azure Redis identity settings. Switch to Redis fanout before
scaling it beyond one replica.

## Authentication and protocol

The client connects through `/ws/session/<room-digest>`, which contains no
credential. Its short-lived JWT is carried in `Sec-WebSocket-Protocol`; the
server returns only `flowlike.realtime.v1`. The server also accepts authenticated
upgrades on `/`, `/ws` and `/ws/`.

Before upgrading, the server checks the ES256 signature, issuer, audience,
optional pinned key ID, time claims, token type, scope, subject, `app_id` and
`board_id`. The only authorized topic is `<app_id>:<board_id>`. Topic changes
outside that grant close the connection. Connections also close at token expiry.

Protocol messages use the authorized topic:

```json
{ "type": "subscribe", "topics": ["<app_id>:<board_id>"] }
{ "type": "publish", "topic": "<app_id>:<board_id>", "data": { "any": "payload" } }
{ "type": "unsubscribe", "topics": ["<app_id>:<board_id>"] }
{ "type": "ping" }
```

Send each JSON object as a separate WebSocket message. A ping receives
`{"type":"pong"}`. Authenticated sockets may hold one topic, accept messages
up to 64 KiB, and send at most 10,000 messages per connection, subject to
ten-second message/publish rate windows. Oversized messages close with code
1009. Invalid JSON, unknown types, unauthorized topics and rate violations close
the socket. Excess connections for a subject receive HTTP 429.

## Upgrade an older signaling deployment

The authenticated client and server protocol must be rolled out together. Older
clients without a token receive 401 from the current server. Current clients
require the credential-free session path and the server's subprotocol echo,
which older servers do not provide.

Before switching traffic, configure the server with the API's public signing key
and key ID, plus the exact web and required desktop origins. Deploy compatible
web and signaling versions in the same window. Older desktop versions must be
updated to restore realtime collaboration; document that compatibility
requirement in the installation's release notes.
