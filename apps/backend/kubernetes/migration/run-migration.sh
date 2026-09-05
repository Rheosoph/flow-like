#!/usr/bin/env bash
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL must be set}"
for attempt in $(seq 1 60); do
  if pg_isready -d "$DATABASE_URL" >/dev/null 2>&1 &&
     PGCONNECT_TIMEOUT=5 psql -X -d "$DATABASE_URL" -Atqc "SELECT 1" >/dev/null 2>&1; then
    break
  fi
  if [ "$attempt" -eq 60 ]; then
    echo "Database did not become available" >&2
    exit 1
  fi
  sleep 2
done

case "${DATABASE_PROVIDER:-postgresql}" in
  postgresql)
    make-prisma-mirror.sh --target postgresql
    schema=prisma-postgres-mirror/schema
    ;;
  cockroachdb) schema=prisma/schema ;;
  *) echo "DATABASE_PROVIDER must be postgresql or cockroachdb" >&2; exit 1 ;;
esac

bun prisma/pre-push.ts
bunx prisma db push --schema="$schema" --accept-data-loss
echo "Database schema applied"
