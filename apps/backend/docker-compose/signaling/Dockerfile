# syntax=docker/dockerfile:1.7
FROM oven/bun:1.3.8-alpine

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

WORKDIR /app

COPY apps/backend/signaling/package.json apps/backend/signaling/bun.lock ./

RUN bun install --frozen-lockfile --production

# Every top-level module ships; tests live in tests/ and stay out of the image.
COPY --chown=bun:bun apps/backend/signaling/*.ts ./
# Bun resolves static imports only at startup, so a missing module must fail here.
RUN bun build ./server.ts --target=bun --packages=external --outfile=/tmp/signaling-resolve-check.js && \
    rm /tmp/signaling-resolve-check.js

ENV NODE_ENV=production \
    PORT=4444 \
    REDIS_AUTH_MODE=local \
    REDIS_URL=redis://redis:6379
EXPOSE 4444

USER bun
CMD ["bun", "run", "server.ts"]
