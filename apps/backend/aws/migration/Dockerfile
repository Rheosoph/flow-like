# syntax=docker/dockerfile:1.7
#
# AWS migration job: applies packages/api/prisma/migrations-dsql to the Aurora
# DSQL cluster as the `admin` role (migrate.ts mints the IAM token with
# @aws-sdk/dsql-signer; no password exists on this cloud). Meant to run as a
# one-off ECS Fargate task with a task role that holds dsql:DbConnectAdmin on
# the cluster ARN. One execution, one apply, exit code = migrate.ts's.
#
# The job does NOT run `prisma migrate deploy`: the schema engine Prisma 7.3.0
# ships sends a whole migration.sql as one batch, which PostgreSQL runs in one
# implicit transaction and DSQL rejects (one DDL per transaction). migrate.ts
# applies every statement on its own, waits for the async index/validation jobs
# in sys.jobs, records the migration in _prisma_migrations exactly as Prisma
# would, grants the runtime role, and finally runs `prisma migrate status` as an
# independent check that the recorded history matches the files. Prisma is in
# the image only for that check and for validating the schema at build time.
#
# The tracked schema declares `provider = "cockroachdb"`; the image derives the
# DSQL mirror with packages/api/scripts/make-prisma-mirror.sh --target dsql,
# which swaps the provider and fails the build on anything DSQL cannot create.

FROM oven/bun:1.3.8-alpine

# openssl provides libssl3/libcrypto3 for the schema engine (and picks the
# linux-musl-openssl-3.0.x engine at install), ca-certificates is what lets
# both node-postgres and Prisma verify the cluster's public-CA certificate,
# bash runs the mirror script. Runtime user is the same uid 10001 as the Rust
# images; it gets a home because bun/bunx keep their caches under $HOME.
RUN apk add --no-cache bash ca-certificates openssl \
    && addgroup -g 10001 flowlike \
    && adduser -u 10001 -G flowlike -s /sbin/nologin -D flowlike

WORKDIR /app

COPY apps/backend/aws/migration/package.json apps/backend/aws/migration/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache,id=bun-cache-aws-migration \
    bun install --frozen-lockfile --production

COPY packages/api/prisma/schema ./prisma/schema
COPY packages/api/scripts/make-prisma-mirror.sh ./make-prisma-mirror.sh
RUN bash ./make-prisma-mirror.sh --target dsql \
    && rm -rf prisma/schema \
    && mv prisma-dsql-mirror/schema prisma/schema \
    && rmdir prisma-dsql-mirror \
    && rm make-prisma-mirror.sh \
    && awk '/^datasource[[:space:]]/,/^}/' prisma/schema/*.prisma | grep -q 'provider = "postgresql"' \
    && ! awk '/^datasource[[:space:]]/,/^}/' prisma/schema/*.prisma | grep -q '"cockroachdb"'

COPY packages/api/prisma/migrations-dsql ./prisma/migrations-dsql
COPY apps/backend/aws/migration/prisma.dsql.config.ts apps/backend/aws/migration/migrate.ts apps/backend/aws/migration/shared.ts ./

ENV NODE_ENV=production \
    CHECKPOINT_DISABLE=1 \
    PRISMA_HIDE_UPDATE_MESSAGE=1 \
    PRISMA_SCHEMA_DISABLE_ADVISORY_LOCK=1

# Fail the build, not the job: the schema engine must start (engine download,
# libssl), the mirrored schema must validate, and the migrations directory must
# be the committed Prisma layout.
RUN node_modules/@prisma/engines/schema-engine-* --version \
    && bun -e 'await import("./migrate.ts")' \
    && bunx --bun prisma --version \
    && bunx --bun prisma validate --config prisma.dsql.config.ts \
    && test -f prisma/migrations-dsql/migration_lock.toml \
    && ls -d prisma/migrations-dsql/*/migration.sql

ENV HOME=/home/flowlike
USER 10001:10001

STOPSIGNAL SIGTERM
ENTRYPOINT ["bun", "run", "migrate.ts"]
