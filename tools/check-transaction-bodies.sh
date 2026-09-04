#!/usr/bin/env bash
set -euo pipefail

# A retried transaction body runs from scratch on every attempt and must not
# await anything but its own transaction: no storage or network calls, no
# signing, no sleeps, no audit branches. This scans every
# `state.transaction(` / `state.transaction_with(` / `retry_transaction(` call
# up to its matching `)` and rejects bodies that mention a forbidden
# identifier.
#
# Usage: tools/check-transaction-bodies.sh [FILE ...]
# Without arguments every tracked Rust file under packages/, apps/backend/ and
# libs/ is scanned.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

# Bracket classes instead of backslash escapes: awk -v rewrites escapes.
FORBIDDEN='master_credentials|to_store[(]|[.]sign[(]|delete_stream|get_state_store|reqwest|sleep[(]|Client::|audit_branch!'
# `state.transaction(` on one line, or `.transaction(` opening a line whose
# previous line ends in `state` (how rustfmt breaks the chain).
START_INLINE='(state[.]transaction(_with)?[(]|[.]transaction_with[(]|retry_transaction(::<[^>]*>)?[(])'
START_CONTINUED='^[[:space:]]*[.]transaction(_with)?[(]'
STATE_TAIL='state[[:space:]]*$'

# The wrapper itself and its tests are the only places allowed to spell out
# what a body may not do.
ALLOWED=(
    packages/db/src
    packages/api/src/db
    packages/api/src/state.rs
    # Live-cluster test of the wrapper itself: two writers deliberately overlap
    # inside their transactions to force the 40001 the retry policy absorbs.
    packages/api/tests/dsql_live.rs
)

is_allowed() {
    local file="$1"
    for prefix in "${ALLOWED[@]}"; do
        case "$file" in "$prefix"*) return 0 ;; esac
    done
    return 1
}

scan_file() {
    awk -v inline="$START_INLINE" -v continued="$START_CONTINUED" \
        -v state_tail="$STATE_TAIL" -v forbidden="$FORBIDDEN" '
        BEGIN { depth = 0; previous = "" }
        {
            line = $0
            if (depth == 0) {
                if (match(line, inline)) {
                    opened = 1
                } else if (previous ~ state_tail && match(line, continued)) {
                    opened = 1
                } else {
                    opened = 0
                }
                previous = line
                if (!opened) next
                start_line = NR
                call = substr(line, RSTART, RLENGTH)
                sub(/^[[:space:]]+/, "", call)
                # Count from the opening parenthesis, past any turbofish.
                line = substr(line, RSTART + RLENGTH - 1)
                buffer = ""
            } else {
                previous = line
            }
            n = length(line)
            for (i = 1; i <= n; i++) {
                ch = substr(line, i, 1)
                if (ch == "(") depth++
                else if (ch == ")") {
                    depth--
                    if (depth == 0) {
                        body = buffer substr(line, 1, i)
                        if (body ~ forbidden) print FILENAME ":" start_line ": " call
                        buffer = ""
                        break
                    }
                }
            }
            if (depth > 0) buffer = buffer line "\n"
        }
    ' "$1"
}

files=()
if [[ $# -gt 0 ]]; then
    files=("$@")
else
    while IFS= read -r file; do
        files+=("$file")
    done < <(git ls-files --cached --others --exclude-standard 'packages/**/*.rs' 'apps/backend/**/*.rs' 'libs/**/*.rs')
fi

status=0
for file in "${files[@]}"; do
    if [[ ! -f "$file" ]] || is_allowed "$file"; then
        continue
    fi
    violations="$(scan_file "$file")"
    if [[ -n "$violations" ]]; then
        echo "$violations"
        status=1
    fi
done

if [[ $status -ne 0 ]]; then
    echo "error: retried transaction bodies must only await the transaction (see packages/db/src/retry.rs)" >&2
fi
exit $status
