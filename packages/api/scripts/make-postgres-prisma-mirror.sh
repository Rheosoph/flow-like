#!/usr/bin/env bash
# Compatibility shim: the mirror is now produced by make-prisma-mirror.sh --target postgresql.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -x "$here/make-prisma-mirror.sh" ]; then
  exec "$here/make-prisma-mirror.sh" --target postgresql
fi
if command -v make-prisma-mirror.sh >/dev/null 2>&1; then
  exec make-prisma-mirror.sh --target postgresql
fi
echo "make-prisma-mirror.sh not found next to $0 or on PATH" >&2
exit 1
