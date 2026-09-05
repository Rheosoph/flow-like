# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM oven/bun:1.3.8-debian AS builder

WORKDIR /app

# Include the complete frozen workspace dependency graph.
COPY . .
RUN bun install --frozen-lockfile

# Build arguments for environment variables
ARG NEXT_PUBLIC_API_URL=http://localhost:8080
ARG NEXT_PUBLIC_REDIRECT_URL=http://localhost:3001/callback
ARG NEXT_PUBLIC_REDIRECT_LOGOUT_URL=http://localhost:3001/

# Set environment variables for build
ENV NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL}
ENV NEXT_PUBLIC_REDIRECT_URL=${NEXT_PUBLIC_REDIRECT_URL}
ENV NEXT_PUBLIC_REDIRECT_LOGOUT_URL=${NEXT_PUBLIC_REDIRECT_LOGOUT_URL}
ENV NEXT_TELEMETRY_DISABLED=1

# Build the static export
WORKDIR /app/apps/web
RUN bun run build

# ============================================================
# Stage 2: Runtime (nginx to serve static files)
# ============================================================
FROM nginx:1.30.4-alpine@sha256:dc5069ad14f19660b141b21236140b91656bf89bbc3e2417c70ae650cd66104c

# Copy custom nginx configuration
COPY apps/backend/kubernetes/web/nginx-main.conf /etc/nginx/nginx.conf
COPY apps/backend/kubernetes/web/nginx.conf /etc/nginx/conf.d/default.conf

# Copy static files from builder
COPY --from=builder /app/apps/web/out /usr/share/nginx/html

# Create non-root user
RUN adduser -D -u 1000 flowlike && \
    chown -R flowlike:flowlike /usr/share/nginx/html && \
    chown -R flowlike:flowlike /var/cache/nginx && \
    chown -R flowlike:flowlike /var/log/nginx && \
    touch /var/run/nginx.pid && \
    chown flowlike:flowlike /var/run/nginx.pid

USER flowlike

EXPOSE 3001

ENTRYPOINT ["nginx", "-g", "daemon off;"]
