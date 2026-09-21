# =============================================================================
# Database Migration Image for Kubernetes
# =============================================================================
# This image contains the Prisma schema and tools needed to run migrations
# against PostgreSQL or CockroachDB in the Kubernetes cluster.
# =============================================================================

FROM oven/bun:1.3.8-debian

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

# Install necessary tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Share the reviewed Prisma, pg, and dotenv versions with Compose migrations.
COPY apps/backend/docker-compose/db-init/package.json apps/backend/docker-compose/db-init/bun.lock ./
RUN bun install --frozen-lockfile --production

# Copy the prisma schema from packages/api, plus the guarded, idempotent SQL
# that must run before `db push` on an existing database (column type changes
# Prisma emits without the USING clause they need; CockroachDB and PostgreSQL)
COPY packages/api/prisma/schema ./prisma/schema
COPY packages/api/prisma/pre-push.ts ./prisma/pre-push.ts
COPY packages/api/prisma/pre-push ./prisma/pre-push
COPY packages/api/scripts/make-prisma-mirror.sh /usr/local/bin/make-prisma-mirror.sh
RUN chmod +x /usr/local/bin/make-prisma-mirror.sh

# Create a minimal prisma.config.ts for migrations
RUN cat > ./prisma.config.ts <<'CONFIGEOF'
import { defineConfig } from "prisma/config";

export default defineConfig({
  schema: "prisma/schema/",
  datasource: {
    url: process.env.DATABASE_URL || "",
  },
});
CONFIGEOF

COPY apps/backend/kubernetes/migration/run-migration.sh /usr/local/bin/run-migration.sh
COPY apps/backend/shared/audit_database_roles.ts /workspace/audit_database_roles.ts
COPY apps/backend/kubernetes/migration/wait-for-migration.ts /usr/local/bin/wait-for-migration.ts

RUN chmod +x /usr/local/bin/run-migration.sh
RUN bash -n /usr/local/bin/run-migration.sh \
    && bun -e 'await import("/usr/local/bin/wait-for-migration.ts")' \
    && node_modules/@prisma/engines/schema-engine-* --version \
    && DATABASE_URL=postgresql://build:build@127.0.0.1/build bun --no-install node_modules/prisma/build/index.js validate --schema=prisma/schema

CMD ["/usr/local/bin/run-migration.sh"]
