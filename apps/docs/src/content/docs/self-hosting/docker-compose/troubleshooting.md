---
title: Troubleshooting
description: Diagnose Docker Compose configuration, health, storage, execution, and build failures
sidebar:
  order: 27
---

Start with the effective service state and recent logs:

```bash
docker compose config --quiet
docker compose ps --all
docker compose logs --tail=200
```

`db-init` is expected to exit after a successful schema update. Long-running
services should become healthy.

## A service does not start

Inspect that service and its dependencies:

```bash
docker compose logs db-init postgres
docker compose logs api redis
docker compose logs runtime
```

Recreate the one-time initializer after correcting a database issue:

```bash
docker compose up --force-recreate db-init
docker compose up -d
```

Do not run several schema initializers concurrently.

## A host port is already in use

Docker reports the conflicting host port during startup. Either stop the other
listener or change the matching value in `.env`, for example:

```dotenv
API_PORT=8180
COMPILER_METRICS_PORT=9192
```

If the API or web port changes, also update browser-facing URLs, OIDC redirects,
reverse-proxy configuration, and the hub configuration.

## Health checks fail

Check the gateway and published services:

```bash
curl --fail --verbose http://localhost:8080/health
curl --fail --verbose http://localhost:3001/health
curl --fail --verbose http://localhost:4444/health
curl --fail --verbose http://localhost:8081/health
```

Check internal endpoints from their containers:

```bash
docker compose exec api curl --fail http://localhost:8080/api/v1/health
docker compose exec runtime curl --fail http://localhost:9000/health
```

The runtime is not published to the host by default, so
`http://localhost:9000` on the Docker host is not the expected diagnostic path.

## API cannot dispatch a run

Verify the runtime name resolves and its internal health endpoint responds:

```bash
docker compose exec api curl --fail http://runtime:9000/health
docker compose logs api runtime
```

Then check the configured lanes:

```bash
docker compose exec api sh -lc \
  'printf "sync=%s async=%s executor=%s\n" \
  "$EXECUTION_BACKEND" "$ASYNC_EXECUTION_BACKEND" "$EXECUTOR_URL"'
```

For the template, interactive runs use HTTP and background runs use Redis.
Confirm `QUEUE_WORKER_ENABLED=true`, the same `REDIS_EXECUTION_QUEUE` is present
on API and runtime, and Redis is healthy.

## Backend JWT errors

The current variables are `BACKEND_KEY`, `BACKEND_PUB`, and `BACKEND_KID`.
Check presence without printing the secret:

```bash
docker compose exec api sh -lc \
  'test -n "$BACKEND_KEY" && test -n "$BACKEND_PUB" && echo "API keys present"'
docker compose exec runtime sh -lc \
  'test -n "$BACKEND_PUB" && echo "Runtime public key present"'
```

Generate a new matching set from the Compose directory:

```bash
../../../tools/gen-execution-keys.sh --export
```

After replacing all three `.env` values, recreate the API, runtime, and
compiler together. Rotating the signing key invalidates tokens signed by the
old key; plan the restart accordingly.

## Object storage fails

First read the API startup error. It distinguishes an unknown provider, a
missing required value, and a store-construction failure.

Common causes include:

- leaving `RUNTIME_CREDENTIALS_PROVIDER` or `CDN_BUCKET_NAME` explicitly empty;
- leaving the selected provider's bucket/container overrides explicitly empty;
- selecting AWS with the checked-in API image, which omits its AWS runtime
  feature;
- an empty or incorrect endpoint;
- using virtual-hosted requests with a provider that needs path style;
- a bucket/container that does not exist;
- master credentials without list/read/write/delete permissions;
- missing `RUNTIME_ROLE_ARN` or temporary-credential permissions;
- an Azure account without the required SAS behavior;
- a GCP service account that cannot exchange/downscope tokens;
- an R2 API token that cannot create temporary credentials.

Check which non-secret values reached the API:

```bash
docker compose exec api sh -lc \
  'printf "storage=%s runtime=%s meta=%s content=%s logs=%s cdn=%s\n" \
  "$STORAGE_PROVIDER" "$RUNTIME_CREDENTIALS_PROVIDER" \
  "$META_BUCKET" "$CONTENT_BUCKET" "$LOG_BUCKET" "$CDN_BUCKET_NAME"'
```

For the selected provider, also confirm the corresponding
`AWS_*_BUCKET`, `AZURE_*_CONTAINER`, or `GCP_*_BUCKET` names are non-empty.

Do not paste full container environments or credential-bearing logs into a
public issue.

## PostgreSQL problems

```bash
docker compose exec postgres sh -lc \
  'pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
docker compose logs postgres db-init
docker compose exec postgres sh -lc \
  'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
```

Back up PostgreSQL before changing schemas, replacing the service, or deleting
volumes.

## Compiler failures

```bash
curl --fail http://localhost:8081/health
curl --fail http://localhost:9092/metrics
docker compose logs compiler api
```

Check `COMPILATION_BACKEND`, `COMPILER_URL`, target configuration, callback
timeouts, and available CPU/memory. Compilation logs can contain source paths
or package metadata; review them before sharing.

## Server-side Events do not trigger

```bash
docker compose logs sink-services api redis
```

Confirm:

- the Event is active and configured for remote execution;
- its sink type is enabled in the hub configuration;
- `SINK_SECRET` is present on the API;
- `SINK_TRIGGER_JWT` is present on `sink-services` and is scoped to the
  required sink type;
- `SINK_TOKEN_ENCRYPTION_KEY` is set in production;
- `sink-services` can reach `http://api:8080` and Redis.

Keep the scheduler at one replica while diagnosing duplicate triggers.

## Build failures

The first build downloads dependencies and compiles several large images.
Check disk space and the failed build stage:

```bash
docker system df
docker compose build api
docker compose build runtime
```

Use a no-cache build only when you have evidence that a stale build layer is
the cause; it discards useful compilation caches and can make diagnosis much
slower.

## Resetting local state

Restarting or rebuilding containers does not require deleting volumes:

```bash
docker compose down
docker compose up -d --build
```

`docker compose down -v` deletes Compose-managed database, Redis, and
observability volumes. Use it only for an intentional development reset after
backing up anything important. External object-storage data is separate and is
not removed by that command.
