# syntax=docker/dockerfile:1.4
FROM oven/bun:1.2-alpine

WORKDIR /app

COPY apps/backend/signaling/package.json apps/backend/signaling/bun.lock ./

RUN bun install --frozen-lockfile

COPY apps/backend/signaling/server.ts ./

ENV PORT=4444
ENV REDIS_URL=redis://redis:6379
EXPOSE 4444

CMD ["bun", "run", "server.ts"]
