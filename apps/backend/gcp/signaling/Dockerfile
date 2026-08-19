# syntax=docker/dockerfile:1.7
# GCP signaling image. The GCP root pins the Cloud Run signaling service to
# exactly one instance (min = max = 1, asserted by a precondition together with
# REALTIME_FANOUT_MODE=local), so there is no second replica to fan messages
# out to and no Redis (Memorystore) is provisioned. Fan-out stays in-process:
# in `local` mode server.ts constructs no Redis client and dials nothing, so
# none of the REDIS_* / AZURE_* settings the Azure image carries apply here.
# Raising the instance count requires provisioning Redis and switching the
# fan-out mode in the same change; the root's invariant refuses one without
# the other.
FROM oven/bun:1.3.8-alpine

WORKDIR /app

COPY apps/backend/signaling/package.json apps/backend/signaling/bun.lock ./
RUN bun install --frozen-lockfile --production

COPY --chown=bun:bun apps/backend/signaling/server.ts apps/backend/signaling/redis.ts apps/backend/signaling/auth.ts apps/backend/signaling/limits.ts ./

# Endpoint and identity values are deployment-specific and intentionally are
# not baked into the image. Cloud Run overrides PORT with the service's
# container port; it is set here so the image is runnable as-is and so the
# value is visible in `docker inspect` rather than only in Terraform.
ENV NODE_ENV=production \
    PORT=4444 \
    REALTIME_FANOUT_MODE=local \
    REALTIME_ALLOW_INSECURE_LOCAL_DEV=false

EXPOSE 4444
USER bun

ENTRYPOINT ["bun", "run", "server.ts"]
