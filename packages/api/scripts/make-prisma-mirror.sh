#!/usr/bin/env bash
# Derive a provider-specific copy of prisma/schema for engines Prisma drives as "postgresql".
#
#   make-prisma-mirror.sh --target postgresql   -> prisma-postgres-mirror/schema
#   make-prisma-mirror.sh --target dsql         -> prisma-dsql-mirror/schema
#
# Both targets only swap the datasource provider. The dsql target additionally refuses
# anything Aurora DSQL cannot create (enums, scalar arrays, GIN indexes, native type
# attributes other than Date/Timestamp/Timestamptz) so a regression in the schema of record fails here
# instead of at deploy time.
set -euo pipefail

TARGET=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="${2:-}"; shift 2 ;;
    --target=*) TARGET="${1#--target=}"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$TARGET" in
  postgresql) DST_PARENT="prisma-postgres-mirror" ;;
  dsql) DST_PARENT="prisma-dsql-mirror" ;;
  *) echo "Usage: $0 --target postgresql|dsql" >&2; exit 2 ;;
esac

SRC_ROOT="prisma/schema"
DST_ROOT="$DST_PARENT/schema"

if [ ! -d "$SRC_ROOT" ]; then
  echo "No '$SRC_ROOT' directory found. Aborting." >&2
  exit 1
fi

rm -rf "$DST_PARENT"
mkdir -p "$DST_ROOT"
cp -R "$SRC_ROOT"/. "$DST_ROOT"/

while IFS= read -r -d '' file; do
  awk '
    BEGIN { in_ds = 0 }
    {
      if ($0 ~ /^[[:space:]]*datasource[[:space:]]+[A-Za-z0-9_]+[[:space:]]*\{[[:space:]]*$/) {
        in_ds = 1
      }
      if (in_ds && $0 ~ /provider[[:space:]]*=/) {
        gsub(/provider[[:space:]]*=[[:space:]]*"cockroachdb"/, "provider = \"postgresql\"")
      }
      print
      if (in_ds && $0 ~ /^[[:space:]]*\}[[:space:]]*$/) {
        in_ds = 0
      }
    }
  ' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
done < <(find "$DST_ROOT" -type f -name "*.prisma" -print0)

if [ "$TARGET" = "dsql" ]; then
  violations=0
  report() {
    echo "DSQL mirror violation ($1):" >&2
    echo "$2" | sed 's/^/  /' >&2
    violations=1
  }
  hits=$(grep -rnE '^[[:space:]]*enum[[:space:]]+[A-Za-z0-9_]+[[:space:]]*\{' "$DST_ROOT" || true)
  [ -n "$hits" ] && report "enum blocks (no CREATE TYPE on DSQL)" "$hits"
  hits=$(grep -rnE '^[[:space:]]*[A-Za-z0-9_]+[[:space:]]+(String|Int|BigInt|Float|Decimal|Boolean|DateTime|Json|Bytes)\[\]' "$DST_ROOT" || true)
  [ -n "$hits" ] && report "scalar list columns (no array types on DSQL)" "$hits"
  hits=$(grep -rnE 'type:[[:space:]]*Gin' "$DST_ROOT" || true)
  [ -n "$hits" ] && report "GIN indexes (unsupported on DSQL)" "$hits"
  hits=$(grep -rnE '@db\.' "$DST_ROOT" | grep -vE '@db\.(Date|Timestamptz?)\b' || true)
  [ -n "$hits" ] && report "native type attributes other than @db.Date/@db.Timestamp/@db.Timestamptz" "$hits"
  if [ "$violations" -ne 0 ]; then
    rm -rf "$DST_PARENT"
    exit 1
  fi
fi

if command -v bunx >/dev/null 2>&1; then
  bunx prisma format   --schema="$DST_ROOT" || true
  bunx prisma validate --schema="$DST_ROOT" || true
fi

echo "$TARGET mirror created at '$DST_ROOT'."
