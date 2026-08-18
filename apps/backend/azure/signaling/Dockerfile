# syntax=docker/dockerfile:1.7
FROM oven/bun:1.3.8-alpine

WORKDIR /app

COPY apps/backend/signaling/package.json apps/backend/signaling/bun.lock ./
RUN bun install --frozen-lockfile --production

COPY --chown=bun:bun apps/backend/signaling/server.ts apps/backend/signaling/redis.ts apps/backend/signaling/auth.ts apps/backend/signaling/limits.ts ./

# Endpoint and identity values are deployment-specific and intentionally are
# not baked into the image. Azure mode permits managed identity only.
ENV NODE_ENV=production \
    PORT=4444 \
    REDIS_AUTH_MODE=azure-entra \
    AZURE_TOKEN_CREDENTIALS=ManagedIdentityCredential \
    REALTIME_ALLOW_INSECURE_LOCAL_DEV=false

EXPOSE 4444
USER bun

ENTRYPOINT ["bun", "run", "server.ts"]
