# syntax=docker/dockerfile:1.7
#
# Azure migration job: `prisma db push` of packages/api/prisma/schema against the
# Flexible Server database, authenticated as the migration user-assigned managed
# identity (migrate.ts mints the token; no password exists on this cloud). Bun
# rather than Rust because the schema is Prisma's; the only native code is the
# Prisma schema engine that @prisma/engines fetches for this platform at install.
# Manual trigger only, one execution per invocation, exit code = Prisma's.
#
# --accept-data-loss is deliberately absent, and must stay absent.
#   `db push` diffs the schema against the live database. When the diff would
#   destroy or narrow anything (drop a table or column, shrink a type, drop an
#   enum value, ...), Prisma prints the exact warnings and, because this job has
#   no TTY and never passes the flag, refuses with "Use the --accept-data-loss
#   flag ..." and exits 1 - nothing was executed and the execution shows as
#   failed. (Steps Prisma cannot execute at all, such as a new required column
#   without a default on a populated table, are listed and refused the same
#   way.) That is the posture both runbooks mandate ("never `db push
#   --accept-data-loss`"): a destructive change never runs from a job with no
#   human in front of it. What the operator does then: read the warnings in the
#   execution log, review the intended schema change against the data it
#   touches, and - if it really is intended - apply the destructive step by
#   hand from the management host with a short-lived migration-identity token
#   (docs/azure/deployment-runbook.md), then re-run this job so it converges on
#   the remaining additive changes. Do not put the flag here and do not smuggle
#   it in through the job's args or env.
#
# The tracked schema declares `provider = "cockroachdb"` (the Kubernetes target
# is CockroachDB). Flexible Server is plain PostgreSQL, so the datasource
# provider is rewritten to "postgresql" at build time - the same edit
# packages/api/scripts/make-postgres-prisma-mirror.sh applies for every plain-
# PostgreSQL target - and the result is validated at build time.
#
# TLS: migrate.ts hands Prisma `sslmode=require&sslaccept=strict`, which is how
# Prisma's engine spells verify-full (it does not implement libpq's verify-*
# modes and would silently skip certificate checks on them). Verification uses
# the system CA store, so ca-certificates is installed here.

FROM oven/bun:1.3.8-alpine

# openssl provides libssl3/libcrypto3, which the schema engine links against and
# which @prisma/get-platform inspects to pick the linux-musl-openssl-3.0.x
# engine - hence before `bun install`. ca-certificates is what makes
# sslaccept=strict able to verify the Flexible Server chain. The runtime user is
# the same uid 10001 as the Rust images; it gets a home because bun and bunx
# keep their caches under $HOME (nothing under /app is writable to it).
RUN apk add --no-cache ca-certificates openssl \
    && addgroup -g 10001 flowlike \
    && adduser -u 10001 -G flowlike -s /sbin/nologin -D flowlike

WORKDIR /app

COPY apps/backend/azure/migration/package.json apps/backend/azure/migration/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache,id=bun-cache-azure-migration \
    bun install --frozen-lockfile --production

COPY packages/api/prisma/schema ./prisma/schema
# Rewrite the datasource provider inside datasource blocks only (generator
# blocks and model fields named `provider` are untouched), then fail the build
# if no datasource ended up on postgresql or one still says cockroachdb.
RUN sed -i '/^datasource[[:space:]]/,/^}/ s/^\([[:space:]]*provider[[:space:]]*=[[:space:]]*\)"cockroachdb"/\1"postgresql"/' prisma/schema/*.prisma \
    && awk '/^datasource[[:space:]]/,/^}/' prisma/schema/*.prisma | grep -q 'provider = "postgresql"' \
    && ! awk '/^datasource[[:space:]]/,/^}/' prisma/schema/*.prisma | grep -q '"cockroachdb"'

# Column type changes `db push` cannot emit correctly on an existing database
# (enum -> TEXT, array -> JSONB need a USING clause). migrate.ts runs the
# runner, idempotent and guarded per statement, with the same token before it
# spawns Prisma; pg and dotenv in package.json are its imports.
COPY packages/api/prisma/pre-push.ts ./prisma/pre-push.ts
COPY packages/api/prisma/pre-push ./prisma/pre-push

COPY apps/backend/azure/migration/prisma.config.ts apps/backend/azure/migration/migrate.ts ./

ENV NODE_ENV=production \
    CHECKPOINT_DISABLE=1 \
    PRISMA_HIDE_UPDATE_MESSAGE=1

# Fail the build, not the job: executing the schema engine directly surfaces a
# missing engine download or libssl here, `prisma --version` records the
# resolved versions in the build log, and `validate` proves the rewritten
# multi-file schema parses for the postgresql provider.
RUN node_modules/@prisma/engines/schema-engine-* --version \
    && bunx --bun prisma --version \
    && bunx --bun prisma validate --schema=prisma/schema

# Set only now: the build steps above ran as root and must not seed this home
# with root-owned bun/bunx cache entries the runtime user cannot replace.
ENV HOME=/home/flowlike
USER 10001:10001

STOPSIGNAL SIGTERM
ENTRYPOINT ["bun", "run", "migrate.ts"]
