---
title: Installation
description: Configure and start the complete Flow-Like stack with Docker Compose
sidebar:
  order: 22
---

This guide builds the images from the repository and starts the single-host
stack defined in `apps/backend/docker-compose/docker-compose.yml`.

## 1. Clone the repository

```bash
git clone https://github.com/Rheosoph/flow-like.git
cd flow-like/apps/backend/docker-compose
```

Confirm that Docker Compose can read the project:

```bash
docker compose version
docker compose config --services
```

The service list should include `web`, `api-gateway`, `api`, `runtime`,
`compiler`, `sink-services`, `signaling`, `postgres`, `redis`, and `db-init`.

## 2. Create local configuration

Copy the environment template:

```bash
cp .env.example .env
```

At minimum, review these groups in `.env`:

- the PostgreSQL password;
- the object-storage and runtime-credential providers, credentials, selected
  provider's bucket/container names, and a non-empty `CDN_BUCKET_NAME`;
- the public web, API, callback, and signaling URLs;
- the API and runtime replica counts;
- the backend signing keys generated in the next step.

The default hub configuration is
`flow-like.config.example.json`. Before a public deployment, maintain a copy
with your OpenID Connect provider, public domains, legal links, feature flags,
and supported sinks. Point both configuration variables at that file:

```dotenv
FLOW_LIKE_CONFIG=apps/backend/docker-compose/flow-like.config.json
FLOW_LIKE_RUNTIME_CONFIG_FILE=./flow-like.config.json
```

`FLOW_LIKE_CONFIG` is copied into the API image during the build, so rebuild the
API after changing it. `FLOW_LIKE_RUNTIME_CONFIG_FILE` is mounted into
`sink-services` at startup.

## 3. Generate backend signing keys

Flow-Like uses one ES256 keypair to sign and verify backend JWTs. From the
Compose directory, run:

```bash
../../../tools/gen-execution-keys.sh --export
```

Copy the three emitted values into `.env`:

```dotenv
BACKEND_KEY=<base64-encoded-private-key>
BACKEND_PUB=<base64-encoded-public-key>
BACKEND_KID=<generated-key-id>
```

Keep `BACKEND_KEY` private. Runtime and compiler services receive only the
public key.

:::note
The generator stores its PEM working files under `tools/`. `*.pem` is ignored
by Git, but the values in `.env` still need the same protection as any other
deployment secret.
:::

## 4. Configure object storage

Create the metadata, content, and log buckets or containers named in `.env`.
The Compose stack does not create external object storage.

The copied template selects AWS, but the checked-in Compose API image omits its
AWS runtime-credential feature. Use
[Storage Providers](/self-hosting/docker-compose/storage/) to choose an
operational provider and replace the copied empty
`RUNTIME_CREDENTIALS_PROVIDER`, `CDN_BUCKET_NAME`, and provider-specific
storage names. Explicit empty values do not fall back.

## 5. Configure server-side Events

The `sink-services` container handles schedules and configured bot adapters.
For production, set high-entropy values for:

```dotenv
SINK_SECRET=<shared-trigger-signing-secret>
SINK_TOKEN_ENCRYPTION_KEY=<token-encryption-key>
```

Generate the scoped token needed by the current combined sink service:

```bash
bun run ../../../tools/gen-sink-jwt.ts --type cron --secret "$SINK_SECRET"
```

Copy the JWT value printed by the command into `SINK_TRIGGER_JWT`. The enabled
sink types themselves come from the hub configuration file. If you are not
using server-side Events yet, the rest of the stack can start without this
token, but `sink-services` will report that its API calls cannot authenticate.

## 6. Validate and start

Check the interpolated configuration without printing it into an issue or
other public log—it contains secrets:

```bash
docker compose config --quiet
docker compose up -d --build
```

The first build compiles several Rust and web images and can take substantially
longer than later cached builds.

## 7. Verify the deployment

Inspect every container, including the one-time initializer:

```bash
docker compose ps --all
```

`db-init` should finish successfully; the long-running services should become
healthy. Then check the published endpoints:

```bash
curl --fail http://localhost:8080/health
curl --fail http://localhost:3001/health
curl --fail http://localhost:4444/health
curl --fail http://localhost:8081/health
```

The runtime is internal by default:

```bash
docker compose exec runtime curl --fail http://localhost:9000/health
```

Open the web app at `http://localhost:3001`. The browser-facing API is
`http://localhost:8080`.

## Add monitoring

Start the optional observability services with the same project:

```bash
docker compose --profile monitoring up -d
```

Grafana is published on `http://localhost:3002` and Prometheus on
`http://localhost:9091` with the template defaults. Change the Grafana
credentials before exposing it.

## Update

Review changes to `.env.example`, the hub configuration template, and release
notes before rebuilding:

```bash
git pull
docker compose config --quiet
docker compose up -d --build --remove-orphans
```

Run migrations through the normal `db-init` dependency instead of editing the
database schema manually.

## Stop or remove

Stop containers while retaining named volumes:

```bash
docker compose down
```

`docker compose down -v` also deletes the Compose-managed PostgreSQL, Redis,
Prometheus, Grafana, and Tempo volumes. It does not delete external object
storage.

:::caution[Back up before deleting volumes]
Use `-v` only when you intend to remove the local deployment data. Back up the
database and any monitoring state you need first.
:::
