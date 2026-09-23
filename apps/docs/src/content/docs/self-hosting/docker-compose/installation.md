---
title: Installation
description: Generate configuration, pin published images and start Docker Compose
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
domain, signaling and legal-link placeholders. Select the host-side file:

```dotenv
FLOW_LIKE_RUNTIME_CONFIG_FILE=./flow-like.config.json
```

Compose mounts that file into the API and sink services at startup. Recreate the
API and sink services after editing it. The audit worker does not read it; its
audit policy is compiled into its image, see
[Audit worker policy](/self-hosting/docker-compose/configuration/#audit-worker-policy).
The web container also reads its public
URLs at startup, so recreate `web` after changing `NEXT_PUBLIC_*`. These changes
do not require rebuilding images. See
[Configuration](/self-hosting/docker-compose/configuration/#hub-configuration-and-identity)
for alternate config sources.

## 3. Select images

The Compose files reference the published self-hosted images
`ghcr.io/rheosoph/flow-like-docker-compose-<workload>` at the tag in
`FLOW_LIKE_IMAGE_TAG` (default `dev`). Every `*_IMAGE` variable starts empty,
which selects that published image. The self-hosted packages are public; no
registry login is required. Cloud-provider packages are not public and are not
used by this deployment.

Per-run execution requires immutable image references. Pin the published
digests:

```bash
python3 scripts/pull-images.py
```

The script pulls the nine first-party images at `FLOW_LIKE_IMAGE_TAG`, writes
`<WORKLOAD>_IMAGE=repository@sha256:...` for each, copies the runtime and
execution-manager pins into `SANDBOX_IMAGE` and `SANDBOX_GATEWAY_IMAGE`, and
preserves every other line and secret. Pass `--tag` to choose a release
(`1.4.0`), a channel (`dev`, `beta`, `latest`) or an immutable
`sha-<commit>-run-<run>-<attempt>` tag, and `--registry` for a fork or mirror.
Forks and private mirrors need `docker login ghcr.io` beforehand; the script
never handles tokens.

The published tag is a multi-architecture index. `docker pull` selects the
daemon's platform and the recorded digest refers to that index, so the same
pinned `.env` works on AMD64 and ARM64 daemons. See
[Containers](/self-hosting/containers/) for the tag scheme.

### Build locally instead

To run code that has not been published, or to build runner images with local
modifications:

```bash
python3 scripts/prepare-images.py
```

This resets `RUNTIME_IMAGE` and `EXECUTION_MANAGER_IMAGE` to local tags when
they are empty or digest-pinned, builds the runner and native Rust
manager/gateway images, then records their immutable local image IDs in
`SANDBOX_IMAGE` and `SANDBOX_GATEWAY_IMAGE`. Other services still use the
published images. To build everything locally, run `up.py --build`: it writes
`flow-like-<workload>:local` into every empty `*_IMAGE` before building, so the
local builds never replace the published names on the daemon.

Local image IDs belong to this daemon. When moving hosts, build and pin again,
or use `pull-images.py` with published digests.

## 4. Validate and start

```bash
python3 scripts/preflight.py
python3 scripts/up.py
```

Preflight checks the rendered Compose graph, secret and profile configuration,
connection budgets, image pins and gVisor settings. It also checks that the
sandbox digest pins match `RUNTIME_IMAGE` and `EXECUTION_MANAGER_IMAGE`.
`up.py` repeats validation before starting the selected services with
`docker compose up -d --no-build`: Compose pulls images that are missing on
the daemon and fails instead of silently building when a pull fails. Use
`up.py --build` for the local build path; it assigns local tags to empty
`*_IMAGE` values and refuses digest pins. Use `--config-only` with
preflight when you need configuration checks without contacting the Docker
daemon.

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

Merge new environment settings into the protected existing file, including
`FLOW_LIKE_IMAGE_TAG` and the empty `*_IMAGE` lines when upgrading from a
release that built every image locally. Pin the reviewed release, run preflight
and start it:

```bash
python3 scripts/pull-images.py --tag 1.4.0
python3 scripts/preflight.py
python3 scripts/up.py
```

Installations that build locally run `prepare-images.py` and `up.py --build`
instead.

Preserve the manager's SQLite state volume across this cutover. It retains
assignment and cancellation records used to reject replay. Verify schema,
storage and representative runs before restoring traffic.

When replacing the older Python supervisor images, rebuild and pin both the
runner and Rust manager/gateway images. The Rust supervisor retains the HTTP
dispatch and ownership-record formats. Preserve signing keys, retained Redis
deliveries and SQLite claims; clearing them can bypass replay protection.

## Stop without deleting data

```bash
docker compose down
```

The configured execution stop grace defaults to 65 minutes so assigned
hour-long runs can drain. Do not use `down --volumes` as an upgrade command:
it removes database, Redis, RustFS, execution ownership and monitoring volumes.
