---
title: Installation
description: Step-by-step guide to deploy Flow-Like with Docker Compose.
sidebar:
  order: 22
---

## 1. Clone the repository

```bash
git clone https://github.com/Rheosoph/flow-like.git
cd flow-like/apps/backend/docker-compose
```

## 2. Configure environment

Copy the example environment file:

```bash
cp .env.example .env
```

Edit `.env` with your configuration. At minimum, configure:

- Storage provider credentials (see [Storage Providers](/self-hosting/docker-compose/storage/))
- Database password (change the default)

## 3. Generate JWT keypair

The execution JWT system enables stateless trust between the API and runtime:

```bash
# From the repository root
./tools/gen-execution-keys.sh
```

Copy the output to your `.env` file:

```env
EXECUTION_KEY=<base64-encoded-private-key>
EXECUTION_PUB=<base64-encoded-public-key>
EXECUTION_KID=execution-es256-v1
```

## 4. Create storage buckets

Before starting, create three buckets in your storage provider:

- `flow-like-meta` (or your custom name)
- `flow-like-content` (or your custom name)
- `flow-like-logs` (or your custom name)

Update the bucket names in `.env` if using custom names.

## 5. Start services

```bash
docker compose up -d
```

This will:
1. Start PostgreSQL and wait for it to be healthy
2. Run database migrations via `db-init`
3. Start the API service
4. Start the execution runtime

## 6. Verify installation

Check that all services are running:

```bash
docker compose ps
```

Expected output:
```
NAME                              STATUS
docker-compose-api-1              running (healthy)
docker-compose-postgres-1         running (healthy)
docker-compose-runtime-1          running (healthy)
```

Check API health:

```bash
curl http://localhost:8080/health
```

View logs:

```bash
# All services
docker compose logs -f

# Specific service
docker compose logs -f api
```

## 7. Access the API

The API is available at `http://localhost:8080` by default.

For production deployments, place a reverse proxy (nginx, Caddy, Traefik) in front to handle TLS termination.

## Updating

To update to a newer version:

```bash
git pull
docker compose down
docker compose build --no-cache
docker compose up -d
```

## Uninstalling

Remove all containers and volumes:

```bash
docker compose down -v
```

:::caution
The `-v` flag removes the PostgreSQL data volume. Omit it to preserve your database.
:::
