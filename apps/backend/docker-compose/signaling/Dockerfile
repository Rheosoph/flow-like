# syntax=docker/dockerfile:1.7
FROM oven/bun:1.3.8-alpine

WORKDIR /app

COPY apps/backend/signaling/package.json apps/backend/signaling/bun.lock ./

RUN bun install --frozen-lockfile --production

COPY --chown=bun:bun apps/backend/signaling/server.ts apps/backend/signaling/redis.ts apps/backend/signaling/auth.ts apps/backend/signaling/limits.ts ./

ENV NODE_ENV=production \
    PORT=4444 \
    REDIS_AUTH_MODE=local \
    REDIS_URL=redis://redis:6379
EXPOSE 4444

USER bun
CMD ["bun", "run", "server.ts"]
