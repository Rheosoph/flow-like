# =============================================================================
# Database Migration Image for Kubernetes
# =============================================================================
# This image contains the Prisma schema and tools needed to run migrations
# against CockroachDB in the Kubernetes cluster.
# =============================================================================

FROM oven/bun:1.2-debian

# Install necessary tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install prisma globally
RUN bun add -g prisma@latest

WORKDIR /workspace

# Copy the prisma schema from packages/api, plus the guarded, idempotent SQL
# that must run before `db push` on an existing database (column type changes
# Prisma emits without the USING clause they need; CockroachDB and PostgreSQL)
COPY packages/api/prisma/schema ./prisma/schema
COPY packages/api/prisma/pre-push.ts ./prisma/pre-push.ts
COPY packages/api/prisma/pre-push ./prisma/pre-push
COPY packages/api/package.json ./package.json

# Install dependencies for prisma (including prisma which has the config module)
# and for prisma/pre-push.ts (pg, dotenv)
RUN bun install --production && bun add prisma @prisma/client pg dotenv

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

# Create migration script
COPY <<'EOF' /usr/local/bin/run-migration.sh
#!/bin/bash
set -e

echo "=== Flow-Like Database Migration ==="
echo "Waiting for database to be ready..."

# Wait for database to be accessible
max_retries=30
retry_count=0
until pg_isready -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" 2>/dev/null; do
    retry_count=$((retry_count + 1))
    if [ $retry_count -ge $max_retries ]; then
        echo "ERROR: Database not ready after $max_retries attempts"
        exit 1
    fi
    echo "Database not ready yet, attempt $retry_count/$max_retries..."
    sleep 2
done

echo "Database is ready!"
echo "Applying pre-push SQL..."
bun prisma/pre-push.ts

echo "Applying database schema..."

# Run prisma db push to create/update schema
if bunx prisma db push --schema=prisma/schema --accept-data-loss; then
    echo "✅ Database schema applied successfully!"
else
    echo "❌ Failed to apply database schema"
    exit 1
fi

echo "=== Migration Complete ==="
EOF

RUN chmod +x /usr/local/bin/run-migration.sh

CMD ["/usr/local/bin/run-migration.sh"]
