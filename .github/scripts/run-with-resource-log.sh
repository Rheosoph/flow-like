#!/usr/bin/env bash

set -euo pipefail

# Runs a command while sampling memory, swap and disk headroom. A runner that
# exhausts either is shut down before `if: failure()` steps can run, so the
# last samples in the step log are the only evidence of which one ran out.

interval="${RESOURCE_LOG_INTERVAL_SECONDS:-120}"

sample() {
  local mem_available swap_free disk_free largest
  mem_available=$(awk '/^MemAvailable:/ { print int($2 / 1024) }' /proc/meminfo)
  swap_free=$(awk '/^SwapFree:/ { print int($2 / 1024) }' /proc/meminfo)
  disk_free=$(df -BM --output=avail "${GITHUB_WORKSPACE:-.}" | awk 'END { print $1 }')
  # awk reads all of ps's output; `head` would close the pipe early and fail it.
  largest=$(ps -eo rss=,args= --sort=-rss | awk 'NR <= 3 {
    name = $2
    for (i = 3; i < NF; i++) if ($i == "--crate-name") name = $(i + 1)
    sub(".*/", "", name)
    printf "%s%s %d MiB", (NR > 1 ? ", " : ""), name, $1 / 1024
  }')
  echo "[resources] $(date -u +%H:%M:%S) memory available ${mem_available} MiB, swap free ${swap_free} MiB, disk free ${disk_free}; largest: ${largest}"
}

sample || true
(while sleep "$interval" >/dev/null 2>&1; do sample || true; done) &
sampler=$!
trap 'kill "$sampler" 2>/dev/null || true' EXIT

"$@"
