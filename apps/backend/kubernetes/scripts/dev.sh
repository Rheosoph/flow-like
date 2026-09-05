#!/usr/bin/env bash
# Compatibility entry point for the current k3d development workflow.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/k3d-setup.sh" "$@"
