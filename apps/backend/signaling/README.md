# Flow-Like Signaling Server

Redis-backed WebSocket signaling server for Flow-Like realtime collaboration. It speaks the `y-webrtc` signaling protocol used by browser peers and can run as a single Bun process, multiple local workers, or multiple nodes behind a load balancer.

## What It Does

- Accepts WebSocket connections on `/`.
- Handles `subscribe`, `unsubscribe`, `publish`, and `ping` JSON messages.
- Publishes local messages to matching WebSocket subscribers.
- Fans messages out across other signaling nodes through Redis pub/sub.
- Tracks room presence in Redis so published messages include the global subscriber count.
- Exposes a plain HTTP health response on any non-WebSocket path, for example `/health`.

The server does not store collaborative document state. Yjs document state lives in the connected peers; this service only helps peers discover and exchange signaling messages.

## Requirements

- Bun 1.2 or newer.
- Redis 7 or compatible.

## Configuration

| Variable | Default | Description |
| --- | --- | --- |
| `PORT` | `4444` | HTTP/WebSocket port. |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Redis connection URL used for fanout and presence. |
| `SIGNAL_CHANNEL` | `signal:publish` | Redis pub/sub channel for cross-node message fanout. |
| `NODE_ID` | random UUID | Stable node identifier used to avoid echoing messages back from Redis and to store per-node presence counts. |

## Run Locally

Start Redis first:

```bash
docker run --rm -p 6379:6379 redis:7-alpine
```

Then start the signaling server:

```bash
cd apps/backend/signaling
bun install
bun run start
```

The WebSocket endpoint is:

```text
ws://localhost:4444
```

Health checks can use:

```bash
curl http://localhost:4444/health
```

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

The default runtime config advertises the signaling endpoint to clients as:

```json
"signaling": ["ws://localhost:4444"]
```

For public HTTPS deployments, expose this service through a TLS-capable reverse proxy and configure clients with the public `wss://...` endpoint.

## Protocol

Clients connect to `/` over WebSocket and exchange JSON messages.

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

- `clients`: global subscriber count for the topic, based on Redis presence.
- `_origin`: node identifier of the publishing signaling node.

Messages larger than 64 KiB are rejected with close code `1009`.

## Tests

Run the server and Redis before executing local tests.

```bash
cd apps/backend/signaling
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

All signaling nodes must share the same Redis instance and `SIGNAL_CHANNEL`. Each node keeps local WebSocket subscriptions in memory and uses Redis for cross-node fanout and presence. Presence keys use the `topic:presence:` prefix and expire after 90 seconds; active nodes refresh their topic presence every 10 seconds.

When running multiple processes on one host, use unique `NODE_ID` values or allow the server to generate random IDs. Behind a load balancer, keep normal WebSocket proxying enabled and forward upgrade headers.
