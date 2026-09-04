#!/usr/bin/env bash
set -euo pipefail

# Code outside the portability layer may only consult DbDialect predicates
# (effective_isolation, bounded_transactions, optimistic_concurrency,
# has_pg_stat_catalog, supports_set_config_timeouts, ...). Branching on the
# engine itself creates "works on CockroachDB, fails on DSQL" paths.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

# POSIX ERE: no \b, so the identifier boundary is spelled out.
PATTERN='is_dsql\(|DbDialect::Dsql([^A-Za-z0-9_]|$)'

# The layer itself, the probes that must speak to a specific catalog, the
# backfills that refuse bounded engines, and the mains that build a pool
# whose engine they already know.
ALLOWED=(
    packages/db/src
    packages/api/src/db
    packages/api/src/routes/admin/resources/database.rs
    packages/api/src/cache/postgres.rs
    packages/api/src/db_backfills.rs
    apps/backend/aws/api/src/main.rs
    apps/backend/aws/file-tracker/src/main.rs
)

exclude_args=()
for prefix in "${ALLOWED[@]}"; do
    exclude_args+=(":(exclude)$prefix" ":(exclude)$prefix/*")
done

hits="$(git grep --untracked -nE "$PATTERN" -- 'packages/**/*.rs' 'apps/backend/**/*.rs' 'libs/**/*.rs' "${exclude_args[@]}" || true)"
if [[ -n "$hits" ]]; then
    echo "$hits"
    echo "error: branch on DbDialect predicates, not on the engine (see packages/db/src/dialect.rs)" >&2
    exit 1
fi
