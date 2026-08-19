#!/usr/bin/env bash
# Runs i18next-cli from the repository root.
#
# The CLI is installed into tools/i18n rather than the workspace: the root
# package.json pins `glob` and `minimatch` to old majors for security, and
# i18next-cli needs glob 13 / minimatch 10. Resolution walks up from the CLI's
# own directory, so the nested install wins while the cwd stays at the repo
# root — which is what `i18next.config.ts` resolves its source globs against.
set -euo pipefail

TOOL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TOOL_DIR/../.." && pwd)"
BIN="$TOOL_DIR/node_modules/.bin/i18next-cli"

if [ ! -x "$BIN" ]; then
	echo "==> Installing i18next-cli into tools/i18n"
	bun install --cwd "$TOOL_DIR" >/dev/null
fi

cd "$REPO_ROOT"
exec "$BIN" "$@"
