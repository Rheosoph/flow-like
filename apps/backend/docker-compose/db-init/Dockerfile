# syntax=docker/dockerfile:1.7
FROM oven/bun:1.3.8-debian
USER root
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
COPY apps/backend/docker-compose/db-init/package.json apps/backend/docker-compose/db-init/bun.lock ./
RUN bun install --frozen-lockfile --production
# Only schema and migration code enter this image. No service .env files,
# signing keys, API sources, or development dependencies are copied.
COPY packages/api/prisma/schema ./prisma/schema
COPY packages/api/prisma/pre-push.ts ./prisma/pre-push.ts
COPY packages/api/prisma/pre-push ./prisma/pre-push
COPY packages/api/scripts/make-prisma-mirror.sh /usr/local/bin/make-prisma-mirror.sh
COPY apps/backend/docker-compose/db-init/prisma.config.ts ./prisma.config.ts
COPY apps/backend/docker-compose/db-init/run-migration.ts ./run-migration.ts
RUN chmod +x /usr/local/bin/make-prisma-mirror.sh \
    && PATH=/usr/bin:/bin /usr/local/bin/make-prisma-mirror.sh --target postgresql \
    && DATABASE_URL=postgresql://build:build@localhost/build bun node_modules/prisma/build/index.js validate --schema=prisma-postgres-mirror/schema
USER bun
CMD ["bun", "/workspace/run-migration.ts"]
