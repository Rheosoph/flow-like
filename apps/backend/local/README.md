# Local Development Backend

This directory contains a simplified setup for local development, using Docker only for infrastructure (postgres, redis) while running the API and runtime natively for faster iteration.

## Quick Start

### 1. Start Infrastructure

```bash
cd apps/backend/local
docker compose up -d
```

This starts:
- **PostgreSQL** on port 5432 (user: `flowlike`, password: `flowlike_dev`, db: `flowlike`)
- **Adminer** on http://localhost:8082 for browser-based database access
- **Redis** on port 6379
- **db-init** job to run database migrations

To log into Adminer, use:
- **System:** `PostgreSQL`
- **Server:** `postgres`
- **Username:** `flowlike`
- **Password:** `flowlike_dev`
- **Database:** `flowlike`

### 2. Start the API

```bash
cd apps/backend/local/api
cargo run
```

The API will start on **http://localhost:8080**

### 3. Start the Runtime (in another terminal)

```bash
cd apps/backend/local/runtime
cargo run
```

The runtime will start on **http://localhost:9000**

## Configuration

### Execution Backends

Two separate backends control execution:

| Env Var | Endpoint | Default | Options |
|---------|----------|---------|--------|
| `EXECUTION_BACKEND` | `/invoke` (streaming) | `http` | http, lambda_stream |
| `ASYNC_EXECUTION_BACKEND` | `/invoke/async` | `redis` | http, redis, sqs, kafka |

Configure in `api/.env`:
```bash
# Streaming: Direct HTTP with SSE
EXECUTION_BACKEND="http"

# Async: Redis queue (requires QUEUE_WORKER_ENABLED=true in runtime)
ASYNC_EXECUTION_BACKEND="redis"
```

### API (.env)
- `API_PORT` - API server port (default: 8080)
- `DATABASE_URL` - PostgreSQL connection string
- `REDIS_URL` - Redis connection string
- `EXECUTOR_URL` - Runtime URL for HTTP execution
- `EXECUTION_BACKEND` - Streaming backend: `http`
- `ASYNC_EXECUTION_BACKEND` - Async backend: `http`, `redis`

### Runtime (.env)
- `RUNTIME_PORT` - Runtime server port (default: 9000)
- `QUEUE_WORKER_ENABLED` - Enable Redis queue polling (required for `ASYNC_EXECUTION_BACKEND=redis`)
- `REDIS_URL` - Redis connection string

## Stopping

```bash
cd apps/backend/local
docker compose down
```

To also remove data volumes:
```bash
docker compose down -v
```
