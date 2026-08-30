# Flow-Like Docker Compose Backend

Single-machine deployment using Docker Compose with shared execution runtime.

## Quick Start

```bash
cd apps/backend/docker-compose
cp .env.example .env
# Configure storage credentials, BACKEND_KEY/BACKEND_PUB, and SINK_SECRET in .env
# For cron sinks, generate SINK_TRIGGER_JWT with:
# bun run ../../../tools/gen-sink-jwt.ts --type cron --secret "$SINK_SECRET"

docker compose up -d
```

**With monitoring:**
```bash
docker compose --profile monitoring up -d
```

## Documentation

Full step-by-step documentation: **[docs.flow-like.com/self-hosting/docker-compose](https://docs.flow-like.com/self-hosting/docker-compose/)**

| Guide | Description |
|-------|-------------|
| [Overview](https://docs.flow-like.com/self-hosting/docker-compose/overview/) | Architecture and components |
| [Prerequisites](https://docs.flow-like.com/self-hosting/docker-compose/prerequisites/) | System requirements |
| [Installation](https://docs.flow-like.com/self-hosting/docker-compose/installation/) | Step-by-step setup |
| [Configuration](https://docs.flow-like.com/self-hosting/docker-compose/configuration/) | Environment variables |
| [Storage Providers](https://docs.flow-like.com/self-hosting/docker-compose/storage/) | AWS, Azure, GCP, R2 |
| [Monitoring](https://docs.flow-like.com/self-hosting/docker-compose/monitoring/) | Prometheus & Grafana |
| [Scaling](https://docs.flow-like.com/self-hosting/docker-compose/scaling/) | Multi-instance setup |
| [Troubleshooting](https://docs.flow-like.com/self-hosting/docker-compose/troubleshooting/) | Common issues |

## Services

| Service | Port | Description |
|---------|------|-------------|
| api-gateway | 8080 | Published API entrypoint/load balancer |
| api | internal 8080 | Flow-Like API replicas |
| web | 3001 | Web application |
| runtime | internal 9000 | Execution runtime replicas |
| signaling | 4444 | Realtime collaboration signaling |
| postgres | 5432 | Database |
| redis | 6379 | Job queue |
| grafana | 3002 | Dashboards (monitoring profile) |
| prometheus | 9091 | Metrics (monitoring profile) |

## Execution Flows

- Normal web executions call `/api/v1/apps/{app_id}/board/{board_id}/invoke`, then the API streams to a runtime replica over HTTP at `http://runtime:9000`.
- Async executions and sink-triggered executions enqueue through Redis on `REDIS_EXECUTION_QUEUE`; runtime replicas poll that queue.
- Cron sinks are handled by the singleton `sink-services` process. It syncs active cron schedules from `/api/v1/sink/schedules` and triggers them through `/api/v1/sink/trigger/async`.
- Realtime collaboration uses the API only for JWT/room-key issuance; browser peers use the `signaling` service from the hub config (`ws://localhost:4444` in the template). The compose signaling image builds the Redis-backed implementation in `apps/backend/signaling`.

### Hosted model proxy

Configure hosted provider credentials on the `api` service. Compose does not
mount those credentials into `runtime`. Each run carries its authenticated API
callback address, and `API_BASE_URL=http://api:8080` supplies the deployment
fallback for hosted completion and remote embedding calls.

### Signaling Upgrade Note

**Upgrading an existing stack:** the `signaling` container now **requires** `BACKEND_PUB` and `REALTIME_ALLOWED_ORIGINS` in `.env` and crashloops at boot when either is empty (`BACKEND_PUB` was previously documented as optional). Set `REALTIME_ALLOWED_ORIGINS` to the web origin (`http://localhost:3001` in the template) and append `tauri://localhost,http://tauri.localhost,https://tauri.localhost` when Tauri desktop clients connect. For a deliberately unauthenticated local test only, set `SIGNALING_NODE_ENV=development` plus `REALTIME_ALLOW_INSECURE_LOCAL_DEV=true`. See the "Realtime Collaboration" block in `.env.example`.

## Scaling And Swarm

The default compose configuration declares `API_REPLICAS=2` and `RUNTIME_REPLICAS=3`. API replicas are not directly published to the host; `api-gateway` owns the host API port and forwards to the internal API service. Runtime replicas are internal and are reached through Docker DNS at `runtime:9000`.

For Docker Swarm, use `docker-stack.yml`. Build and push the images first, set the `*_IMAGE` values in `.env` to registry-backed image names, then deploy from this directory with:

```bash
set -a && . ./.env && set +a
docker compose build
for image in \
  "${DB_INIT_IMAGE:-flow-like-db-init:latest}" \
  "${API_IMAGE:-flow-like-api:latest}" \
  "${RUNTIME_IMAGE:-flow-like-runtime:latest}" \
  "${COMPILER_IMAGE:-flow-like-compiler:latest}" \
  "${SINK_SERVICES_IMAGE:-flow-like-sink-services:latest}" \
  "${SIGNALING_IMAGE:-flow-like-signaling:latest}" \
  "${WEB_IMAGE:-flow-like-web:latest}"; do
  docker push "$image"
done
docker stack config -c docker-stack.yml
docker stack deploy -c docker-stack.yml flowlike
```

Swarm does not build images from the stack file and does not load `.env` automatically. Keep `sink-services` at one replica to avoid duplicate cron triggers.

## Supported Event Sinks

The docker-compose deployment supports server-side event sinks for triggering flows. Configure which sinks are enabled in the `flow-like.config.json` file under `supported_sinks`:

| Sink | Default | Description | Requirements |
|------|---------|-------------|--------------|
| `http` | ✅ | REST API endpoints | None |
| `webhook` | ✅ | Incoming webhooks | None |
| `cron` | ✅ | Scheduled triggers | None |
| `github` | ✅ | Repository webhooks | Public endpoint |
| `rss` | ✅ | Feed polling | None |
| `discord` | ✅ | Discord bot | Bot token, persistent process |
| `telegram` | ✅ | Telegram bot | Bot token, persistent process |
| `slack` | ✅ | Slack bot | Bot token, persistent process |
| `email` | ✅ | IMAP polling | IMAP credentials |
| `mqtt` | ❌ | MQTT broker | MQTT broker |

See `flow-like.config.example.json` for a full configuration template.

## Build Caching

The Dockerfiles use BuildKit cache mounts to persist Cargo registry and build artifacts across rebuilds. This significantly speeds up subsequent builds by avoiding recompilation of unchanged dependencies.

**First build:** ~15-20 minutes (full compilation)
**Subsequent builds:** ~1-3 minutes (incremental)

To clear the build cache:
```bash
docker builder prune --filter type=exec.cachemount
```

## Common Commands

```bash
# View logs
docker compose logs -f api

# Check health
curl http://localhost:8080/health

# Stop services
docker compose down

# Remove all data
docker compose down -v
```
