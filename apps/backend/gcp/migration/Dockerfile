# syntax=docker/dockerfile:1.7
# =============================================================================
# Flow-Like schema migration job for GCP - Cloud SQL for PostgreSQL, IAM auth
# =============================================================================
# A Cloud Run *job*, executed by hand from the runbook and never on a schedule.
# It runs `prisma db push` against the application database as the migration
# service account and exits with Prisma's exit code. Build context is the
# `flow-like` workspace root: the schema and its Postgres rewrite script are
# copied from packages/api, so a schema change rebuilds this image through the
# shared `flow-like/packages` fingerprint.
#
# --accept-data-loss IS DELIBERATELY ABSENT. Without it Prisma compares the
# schema with the live database and, if the diff would drop a table or column
# that still holds data, applies NOTHING, prints exactly what it would have
# dropped and exits 1 - the additive parts of the same diff are held back too.
# That refusal is the safe posture both runbooks mandate ("never `db push
# --accept-data-loss`"): this job runs as the schema-owning identity, and a
# baked-in flag would turn the next `gcloud run jobs execute` into a silent
# data-loss event. When the job refuses: read its log for the listed drops,
# decide whether they are intended, back up first if they are, then perform the
# destructive step by hand from the management-subnet host as the migration
# identity (psql over verify-ca with a `gcloud sql generate-login-token` token,
# see docs/gcp/deployment-runbook.md) - or ship an additive schema first and the
# removal in a later, reviewed run. Do not add the flag to this image.
#
# Authentication: no password exists anywhere in the GCP installation (D4). The
# entrypoint mints a metadata-server token carrying only the SQL login scope,
# composes DATABASE_URL in memory and passes it to one child process; it refuses
# to start if DATABASE_URL, PGPASSWORD, a key file, a metadata override or a
# proxy variable is present. Env contract and validation mirror
# flow_like_gcp_data::postgres, see migrate.ts and README.md.
#
# Transport: Cloud SQL refuses cleartext (ENCRYPTED_ONLY) and the job always
# sends `sslmode=require`. Prisma's engine cannot verify a certificate chain
# without also verifying the hostname, and the per-instance Cloud SQL CA issues
# no address identity, so against the private IP that Terraform passes today the
# server certificate is not verified by the client; against the instance DNS
# name the pinned CA (GCP_POSTGRES_SERVER_CA) is enforced with a strict check.
# The full reasoning and the libpq-vs-Prisma parameter mismatch are in
# migrate.ts (`tlsPosture`).
#
# The checked-in schema declares `provider = "cockroachdb"`. Every Postgres
# installation of Flow-Like (docker-compose db-init, packages/api's own tools
# image) rewrites the datasource provider to `postgresql` with
# packages/api/scripts/make-postgres-prisma-mirror.sh before pushing; this image
# does the same at build time and asserts the result, so the runtime schema
# under /app/prisma/schema is already the Postgres one.
# =============================================================================

FROM oven/bun:1.3.8-debian AS builder

# Prisma's schema engine is a native binary linked against OpenSSL 3; the debian
# variant of the bun image is what packages/api's tooling and the Kubernetes
# migration image have proven it against.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY apps/backend/gcp/migration/package.json apps/backend/gcp/migration/bun.lock ./
# @prisma/engines downloads the schema-engine binary in its postinstall; bun
# runs it because the package is trusted (default list and package.json). It
# must happen here, at build time - the job's network path is the VPC and it
# has no route to binaries.prisma.sh at run time.
RUN --mount=type=cache,target=/root/.bun/install/cache,id=bun-cache-gcp-migration \
    bun install --frozen-lockfile --production

# The API's package.json is the pin the schema is authored against. The version
# installed here is exact (lockfile) and must satisfy that range, so a Prisma
# major bump in packages/api that forgets this image fails the build instead of
# the migration.
COPY packages/api/package.json ./api-package.json
RUN bun -e ' \
    const api = require("./api-package.json"); \
    const range = api.devDependencies?.prisma ?? api.dependencies?.prisma; \
    const installed = require("prisma/package.json").version; \
    if (!range || !Bun.semver.satisfies(installed, range)) { \
      console.error(`prisma ${installed} does not satisfy the packages/api pin ${range}`); \
      process.exit(1); \
    } \
    console.log(`prisma ${installed} satisfies the packages/api pin ${range}`);'

COPY packages/api/prisma/schema ./prisma/schema
COPY packages/api/scripts/make-postgres-prisma-mirror.sh /usr/local/bin/make-postgres-prisma-mirror.sh
COPY apps/backend/gcp/migration/prisma.config.ts apps/backend/gcp/migration/migrate.ts ./

# The throwaway loopback URL only satisfies prisma.config.ts's `env()` guard;
# `prisma format` / `prisma validate` never open a connection. The two greps make
# the rewrite an assertion rather than a hope, and running the schema engine
# with --version proves the native binary loads (libssl present, right target)
# before the image is ever pushed.
RUN chmod +x /usr/local/bin/make-postgres-prisma-mirror.sh \
    && DATABASE_URL=postgresql://build:build@127.0.0.1:5432/build make-postgres-prisma-mirror.sh \
    && rm -rf prisma/schema \
    && mv prisma-postgres-mirror/schema prisma/schema \
    && rmdir prisma-postgres-mirror \
    && grep -rEq 'provider *= *"postgresql"' prisma/schema/ \
    && ! grep -rEq 'provider *= *"cockroachdb"' prisma/schema/ \
    && DATABASE_URL=postgresql://build:build@127.0.0.1:5432/build bun x --bun prisma validate --schema=prisma/schema \
    && node_modules/@prisma/engines/schema-engine-* --version

FROM oven/bun:1.3.8-debian AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /app /app

USER 10001:10001
# CHECKPOINT_DISABLE stops the Prisma CLI phoning checkpoint.prisma.io for
# update checks: the job has no internet route, and a migration job should not
# be making outbound calls that are not the database. No GCP_POSTGRES_* default
# is baked in: the whole contract arrives from Terraform, exactly as it does
# for the api and queue-worker images.
ENV NODE_ENV=production \
    CHECKPOINT_DISABLE=1 \
    PRISMA_HIDE_UPDATE_MESSAGE=1

STOPSIGNAL SIGTERM
ENTRYPOINT ["bun", "run", "migrate.ts"]
