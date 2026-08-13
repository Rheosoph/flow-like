#!/usr/bin/env bash

set -uo pipefail

max_attempts="${FLOW_LIKE_CODESIGN_MAX_ATTEMPTS:-3}"
timeout_seconds="${FLOW_LIKE_CODESIGN_TIMEOUT_SECONDS:-300}"
retry_delay_seconds="${FLOW_LIKE_CODESIGN_RETRY_DELAY_SECONDS:-10}"

require_integer() {
  local name="$1"
  local value="$2"

  if ! [[ "$value" =~ ^[0-9]+$ ]]; then
    echo "::error::$name must be a non-negative integer, got: $value"
    exit 2
  fi
}

require_integer FLOW_LIKE_CODESIGN_MAX_ATTEMPTS "$max_attempts"
require_integer FLOW_LIKE_CODESIGN_TIMEOUT_SECONDS "$timeout_seconds"
require_integer FLOW_LIKE_CODESIGN_RETRY_DELAY_SECONDS "$retry_delay_seconds"

if (( max_attempts < 1 || max_attempts > 10 )); then
  echo "::error::FLOW_LIKE_CODESIGN_MAX_ATTEMPTS must be between 1 and 10."
  exit 2
fi
if (( timeout_seconds < 30 || timeout_seconds > 900 )); then
  echo "::error::FLOW_LIKE_CODESIGN_TIMEOUT_SECONDS must be between 30 and 900."
  exit 2
fi
if (( retry_delay_seconds > 60 )); then
  echo "::error::FLOW_LIKE_CODESIGN_RETRY_DELAY_SECONDS must not exceed 60."
  exit 2
fi

for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  /usr/bin/perl -e 'alarm shift; exec @ARGV; die "could not execute codesign: $!\n"' \
    "$timeout_seconds" /usr/bin/codesign "$@"
  result_code=$?

  if (( result_code == 0 )); then
    exit 0
  fi
  if (( result_code != 142 )); then
    exit "$result_code"
  fi
  if (( attempt == max_attempts )); then
    echo "::error::codesign timed out after ${timeout_seconds}s on all ${max_attempts} attempts."
    exit 124
  fi

  echo "::warning::codesign timed out after ${timeout_seconds}s (attempt ${attempt}/${max_attempts}); retrying in ${retry_delay_seconds}s."
  sleep "$retry_delay_seconds"
done

