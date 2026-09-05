---
title: Installation
description: Generate configuration, pin execution images and start Docker Compose
sidebar:
  order: 22
---

This procedure creates a new installation from
`apps/backend/docker-compose/docker-compose.yml`. Complete the Linux and gVisor
[prerequisites](/self-hosting/docker-compose/prerequisites/) first. Existing
installations should follow the upgrade section below instead of replacing
their environment file.

## 1. Get the deployment

```bash
git clone https://github.com/Rheosoph/flow-like.git
cd flow-like/apps/backend/docker-compose
```

## 2. Generate private configuration

```bash
python3 scripts/setup-env.py
```

The script creates `.env` with mode `0600`, generates independent service and
storage credentials, generates a matching ES256 signing keypair and prepares the
sink trigger token. It refuses to replace an existing file and does not print
secrets.

For public hosting, provide the planned origins when generating the file:

```bash
python3 scripts/setup-env.py \
  --web-origin https://app.example.com \
  --api-url https://api.example.com \
  --s3-endpoint https://storage.example.com
```

Choose one of these setup commands for a new installation. The public variant
also requires the TLS gateway configuration described under
[Storage](/self-hosting/docker-compose/storage/#public-tls-storage).

Review capacity, allowed integrations and URLs in `.env`. Keep
`EXECUTION_ISOLATION_MODE=per_run` and `COMPOSE_PROFILES=per-run` for isolated
execution.

Create a maintained hub configuration from the example and replace its OIDC,
domain, signaling and legal-link placeholders. Point both selectors to it:

```dotenv
FLOW_LIKE_CONFIG=apps/backend/docker-compose/flow-like.config.json
FLOW_LIKE_RUNTIME_CONFIG_FILE=./flow-like.config.json
```

The API embeds `FLOW_LIKE_CONFIG` during its build. Sink services read the
runtime file. The web's `NEXT_PUBLIC_*` settings are also build arguments.
Rebuild the affected images when these values change.

## 3. Build and pin execution images

```bash
python3 scripts/prepare-images.py
```

This builds the runner and native Rust manager/gateway images, then records
their immutable local image IDs in `SANDBOX_IMAGE` and
`SANDBOX_GATEWAY_IMAGE`. It preserves the generated secrets. The daemon must
already have both images; the manager does not pull mutable tags during
execution.

Local image IDs belong to this daemon. When moving hosts, build and pin again,
or distribute images through a registry and use their `repository@sha256:...`
digests.

## 4. Validate and start

```bash
python3 scripts/preflight.py
python3 scripts/up.py --build
```

Preflight checks the rendered Compose graph, secret and profile configuration,
connection budgets, image pins and gVisor settings. `up.py` repeats validation
before starting the selected services. Use `--config-only` with preflight when
you need configuration checks without contacting the Docker daemon.

Initial startup runs storage bootstrap and database initialization before
starting the API. Bootstrap creates private metadata, content and log buckets
and separate API/STS identities. Database initialization holds an advisory lock
and rejects destructive schema changes. A failed initializer blocks API startup.

## 5. Verify readiness

```bash
docker compose ps --all
docker compose logs --tail=100 db-init object-store-init execution-manager
curl --fail http://localhost:8080/health
curl --fail http://localhost:3001/health
docker compose exec execution-manager /app/execution-manager healthcheck
```

The initializers should exit successfully. Long-running services should become
healthy, and the manager must prepare at least one unused sandbox before serving
requests. Confirm login, a representative execution, content upload/download and
run-log retrieval.

Run the [storage authorization checks](/self-hosting/docker-compose/storage/#verify-prefix-authorization)
on disposable test prefixes. Before admitting untrusted tenants, also test the
exact deployed gVisor images for blocked host/metadata access, denied callback
routes, cancellation, resource exhaustion and recovery after manager loss.
These checks and representative load tests establish the limits of your host;
installation success alone does not measure them.

## Add monitoring

Append `monitoring` to the existing profiles:

```dotenv
COMPOSE_PROFILES=per-run,monitoring
```

Then run `python3 scripts/up.py`. Grafana uses the generated password and binds
to `http://localhost:3002`. See
[Monitoring](/self-hosting/docker-compose/monitoring/) before relying on alerts.

## Upgrade an existing installation

Back up PostgreSQL, object data and storage IAM state, Redis, hub configuration,
signing keys and the `execution_manager_state` volume. Preserve existing bucket
names and credentials. Enabling bundled RustFS does not migrate external data.

Stop new event production and execution requests, then let queued and active
runs settle. Inspect quarantined attempts before replaying anything that may
have produced external effects. For the `exec:jobs:v3` queue transition, drain
or reconcile old queues and stop old managers before switching producers and
consumers. Do not mix queue protocol versions.

Merge new environment settings into the protected existing file. Build and pin
updated execution images, run preflight and start the reviewed version:

```bash
python3 scripts/prepare-images.py
python3 scripts/preflight.py
python3 scripts/up.py --build
```

Preserve the manager's SQLite state volume across this cutover. It retains
assignment and cancellation records used to reject replay. Verify schema,
storage and representative runs before restoring traffic.

## Stop without deleting data

```bash
docker compose down
```

The configured execution stop grace defaults to 65 minutes so assigned
hour-long runs can drain. Do not use `down --volumes` as an upgrade command:
it removes database, Redis, RustFS, execution ownership and monitoring volumes.
