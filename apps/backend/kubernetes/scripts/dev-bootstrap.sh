#!/usr/bin/env bash
# Generate private development configuration without changing a cluster or .env.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
  echo 'Development bootstrap requires K3D_EXECUTION_MODE=trusted_shared. For isolated execution, use setup-config.sh directly.'
  exec "$SCRIPT_DIR/setup-config.sh" --help
fi
if [[ "${K3D_EXECUTION_MODE:-}" != trusted_shared ]]; then
  echo 'Set K3D_EXECUTION_MODE=trusted_shared explicitly for local development configuration.' >&2
  echo 'For isolated execution on Linux with runsc and Cilium, use setup-config.sh directly.' >&2
  exit 1
fi
exec "$SCRIPT_DIR/setup-config.sh" "$@"
