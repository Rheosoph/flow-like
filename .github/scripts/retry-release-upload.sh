#!/usr/bin/env bash

set -euo pipefail

bundle_dir="${RELEASE_BUNDLE_DIR:-target/release/bundle}"
max_attempts="${RELEASE_UPLOAD_MAX_ATTEMPTS:-6}"
base_delay="${RELEASE_UPLOAD_BASE_DELAY_SECONDS:-30}"
max_delay="${RELEASE_UPLOAD_MAX_DELAY_SECONDS:-300}"
jitter_max="${RELEASE_UPLOAD_JITTER_SECONDS:-30}"
initial_stagger_max="${RELEASE_UPLOAD_INITIAL_STAGGER_SECONDS:-30}"
poll_delay="${RELEASE_UPLOAD_POLL_DELAY_SECONDS:-5}"

require_positive_integer() {
  local name="$1"
  local value="$2"

  if ! [[ "$value" =~ ^[0-9]+$ ]]; then
    echo "::error::$name must be a non-negative integer, got: $value"
    exit 1
  fi
}

require_positive_integer RELEASE_UPLOAD_MAX_ATTEMPTS "$max_attempts"
require_positive_integer RELEASE_UPLOAD_BASE_DELAY_SECONDS "$base_delay"
require_positive_integer RELEASE_UPLOAD_MAX_DELAY_SECONDS "$max_delay"
require_positive_integer RELEASE_UPLOAD_JITTER_SECONDS "$jitter_max"
require_positive_integer RELEASE_UPLOAD_INITIAL_STAGGER_SECONDS "$initial_stagger_max"
require_positive_integer RELEASE_UPLOAD_POLL_DELAY_SECONDS "$poll_delay"

if (( max_attempts < 1 || max_attempts > 10 )); then
  echo "::error::RELEASE_UPLOAD_MAX_ATTEMPTS must be between 1 and 10."
  exit 1
fi

if [ -z "${GITHUB_REPOSITORY:-}" ]; then
  echo "::error::GITHUB_REPOSITORY is required."
  exit 1
fi

# These outputs are written after Tauri has built and discovered its artifacts,
# but before it starts uploading. An empty output therefore indicates a real
# build/discovery failure, which must not be masked by the upload fallback.
if [ -z "${TAURI_ARTIFACT_PATHS:-}" ] || \
  ! jq -e 'type == "array" and length > 0' <<< "$TAURI_ARTIFACT_PATHS" >/dev/null; then
  echo "::error::Tauri did not report any built artifacts; refusing to run the upload-only fallback."
  exit 1
fi

if [ -z "${TAURI_APP_VERSION:-}" ]; then
  echo "::error::Tauri did not report the application version."
  exit 1
fi

tag="beta-v${TAURI_APP_VERSION}"

if [ ! -d "$bundle_dir" ]; then
  echo "::error::No release bundle directory found at $bundle_dir."
  exit 1
fi

work_dir="$(mktemp -d)"
artifact_paths="$work_dir/artifact-paths"
artifact_manifest="$work_dir/artifact-manifest"
starter_markers="$work_dir/starter-markers"
reported_paths="$work_dir/reported-paths"
upload_staging="$bundle_dir/.release-upload-${GITHUB_RUN_ID:-$$}-${RANDOM}"

cleanup() {
  rm -rf "$work_dir" "$upload_staging"
}
trap cleanup EXIT
mkdir -p "$starter_markers" "$upload_staging"

if ! jq -er '
  if all(.[]; type == "string" and length > 0) then .[]
  else error("artifact paths must be non-empty strings")
  end
' \
  <<< "$TAURI_ARTIFACT_PATHS" > "$reported_paths"; then
  echo "::error::Could not parse Tauri's reported artifact paths."
  exit 1
fi

# Use only paths reported by this action invocation. On macOS the action emits
# the .app path before packaging it, so resolve that one path to the archive it
# creates immediately afterwards.
while IFS= read -r reported_path; do
  file="$reported_path"
  if command -v cygpath >/dev/null 2>&1; then
    file="$(cygpath -u "$file")"
  fi

  if [ -f "$file" ]; then
    printf '%s\n' "$file" >> "$artifact_paths"
  elif [[ "$file" == *.app ]] && [ -d "$file" ] && [ -f "$file.tar.gz" ]; then
    printf '%s\n' "$file.tar.gz" >> "$artifact_paths"
  else
    echo "::error::Tauri-reported artifact is missing: $reported_path"
    exit 1
  fi
done < "$reported_paths"

sort -u -o "$artifact_paths" "$artifact_paths"

if [ ! -s "$artifact_paths" ]; then
  echo "::error::No release artifacts found under $bundle_dir."
  find "$bundle_dir" -type f | sed 's/^/  /'
  exit 1
fi

sha256_digest() {
  local file="$1"
  local output
  local hash

  if command -v sha256sum >/dev/null 2>&1; then
    if ! output="$(sha256sum "$file")"; then
      echo "::error::Could not hash artifact: $file"
      return 1
    fi
  elif command -v shasum >/dev/null 2>&1; then
    if ! output="$(shasum -a 256 "$file")"; then
      echo "::error::Could not hash artifact: $file"
      return 1
    fi
  else
    echo "::error::Neither sha256sum nor shasum is available."
    return 1
  fi

  hash="${output%% *}"
  if ! [[ "$hash" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "::error::Hash command returned an invalid SHA-256 for: $file"
    return 1
  fi
  hash="$(printf '%s' "$hash" | tr '[:upper:]' '[:lower:]')"
  printf 'sha256:%s\n' "$hash"
}

sanitize_asset_name() {
  local original="$1"
  local value="$1"

  # The generated artifact names are ASCII. Mirror gh's filename sanitization
  # so, for example, "Flow Like.msi" matches GitHub's "Flow.Like.msi".
  if [[ "$value" == .* ]]; then
    value="default${value}"
  fi
  if ! value="$(LC_ALL=C sed -E \
    -e 's/[^a-zA-Z0-9_+@-]+/./g' \
    -e 's/\.{2,}/./g' \
    -e 's/^\.//' \
    -e 's/\.$//' <<< "$value")"; then
    return 1
  fi
  if [[ "$original" != "$value" && "$value" != *.* ]]; then
    value="default.${value}"
  fi

  printf '%s\n' "$value"
}

tauri_asset_name() {
  local filename="$1"
  local arch

  # tauri-action adds the runner architecture to macOS updater archives even
  # though the local archive basename has no architecture suffix.
  case "$filename" in
    *.app.tar.gz.sig)
      if [ "${TAURI_RUNNER_ARCH:-}" = "ARM64" ]; then
        arch="aarch64"
      else
        arch="x64"
      fi
      printf '%s_%s.app.tar.gz.sig\n' "${filename%.app.tar.gz.sig}" "$arch"
      ;;
    *.app.tar.gz)
      if [ "${TAURI_RUNNER_ARCH:-}" = "ARM64" ]; then
        arch="aarch64"
      else
        arch="x64"
      fi
      printf '%s_%s.app.tar.gz\n' "${filename%.app.tar.gz}" "$arch"
      ;;
    *)
      printf '%s\n' "$filename"
      ;;
  esac
}

while IFS= read -r file; do
  filename="$(basename "$file")"
  asset_name="$(tauri_asset_name "$filename")"
  if ! remote_name="$(sanitize_asset_name "$asset_name")"; then
    echo "::error::Could not normalize release asset name: $asset_name"
    exit 1
  fi
  if ! digest="$(sha256_digest "$file")"; then
    exit 1
  fi

  upload_file="$file"
  if [ "$asset_name" != "$filename" ]; then
    upload_file="$upload_staging/$asset_name"
    if ! ln "$file" "$upload_file"; then
      echo "::error::Could not stage renamed upload asset: $asset_name"
      exit 1
    fi
  fi

  printf '%s\t%s\t%s\n' "$digest" "$remote_name" "$upload_file" >> "$artifact_manifest"
done < "$artifact_paths"

if (( initial_stagger_max > 0 )); then
  sleep $((RANDOM % (initial_stagger_max + 1)))
fi

retry_wait() {
  local attempt="$1"
  local message="$2"
  local delay
  local jitter=0
  local wait_seconds

  delay=$((base_delay * (1 << (attempt - 1))))
  if (( delay > max_delay )); then
    delay="$max_delay"
  fi
  if (( jitter_max > 0 )); then
    jitter=$((RANDOM % (jitter_max + 1)))
  fi
  wait_seconds=$((delay + jitter))
  echo "$message in ${wait_seconds}s..."
  if (( wait_seconds > 0 )); then
    sleep "$wait_seconds"
  fi
}

release_id="${TAURI_RELEASE_ID:-}"
if ! [[ "$release_id" =~ ^[0-9]+$ ]]; then
  for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    candidate=""
    if candidate="$(gh release view "$tag" \
      --repo "$GITHUB_REPOSITORY" \
      --json databaseId \
      --jq '.databaseId')" && [[ "$candidate" =~ ^[0-9]+$ ]]; then
      release_id="$candidate"
      break
    fi

    if ! gh release create "$tag" \
      --repo "$GITHUB_REPOSITORY" \
      --draft \
      --prerelease \
      --title "Flow Like - Beta v${TAURI_APP_VERSION}"; then
      : # The release may have been created concurrently or the API may be unavailable.
    fi

    if candidate="$(gh release view "$tag" \
      --repo "$GITHUB_REPOSITORY" \
      --json databaseId \
      --jq '.databaseId')" && [[ "$candidate" =~ ^[0-9]+$ ]]; then
      release_id="$candidate"
      break
    fi

    if [ "$attempt" -lt "$max_attempts" ]; then
      retry_wait "$attempt" "Retrying release lookup"
    fi
  done
fi

if ! [[ "$release_id" =~ ^[0-9]+$ ]]; then
  echo "::error::Could not resolve the numeric release ID for $tag."
  exit 1
fi

remote_state=""
remote_id=""
remote_digest=""

fetch_remote_asset() {
  local name="$1"
  local assets
  local record

  remote_state=""
  remote_id=""
  remote_digest=""

  if ! assets="$(gh api --method GET \
    --paginate \
    --slurp \
    "repos/${GITHUB_REPOSITORY}/releases/${release_id}/assets?per_page=100")"; then
    return 1
  fi

  if ! record="$(jq -er --arg name "$name" '
    if type != "array" or any(.[]; type != "array") then
      error("paginated release assets response is invalid")
    else
      [.[][] | select(.name == $name)] as $matches
      | if ($matches | length) > 1 then
          error("release asset name is ambiguous")
        else
          ($matches[0] // {}) as $asset
          | [
              ($asset.state // "missing"),
              (($asset.id // "") | tostring),
              ($asset.digest // "")
            ]
          | join("|")
        end
    end
  ' <<< "$assets")"; then
    return 1
  fi

  if ! IFS='|' read -r remote_state remote_id remote_digest <<< "$record" || \
    [ -z "$remote_state" ]; then
    return 1
  fi
}

# Returns 0 when the exact asset is already uploaded, 1 when it is safe to
# upload, and 2 when a transient API failure requires a later retry.
prepare_remote_asset() {
  local name="$1"
  local expected_digest="$2"

  if ! fetch_remote_asset "$name"; then
    echo "::warning::Could not inspect remote asset $name; deferring it to the next retry round."
    return 2
  fi

  case "$remote_state" in
    missing)
      return 1
      ;;
    uploaded)
      if [ "$remote_digest" = "$expected_digest" ]; then
        return 0
      fi
      echo "::warning::Replacing $name because its remote digest does not match the built artifact."
      ;;
    starter)
      if [ -z "$remote_id" ]; then
        echo "::warning::Incomplete asset $name has no ID; deferring it."
        return 2
      fi
      if [ ! -f "$starter_markers/$remote_id" ]; then
        # It may still be completing after an ambiguous 5xx response. Give it
        # one full backoff round before treating it as abandoned.
        : > "$starter_markers/$remote_id"
        echo "::warning::Incomplete asset $name may still be settling; deferring cleanup."
        return 2
      fi
      echo "::warning::Removing incomplete starter asset $name before retrying."
      ;;
    *)
      echo "::warning::Remote asset $name has unexpected state '$remote_state'; deferring it."
      return 2
      ;;
  esac

  if [ -z "$remote_id" ] || \
    ! gh api --method DELETE \
      "repos/${GITHUB_REPOSITORY}/releases/assets/${remote_id}" \
      --silent; then
    echo "::warning::Could not remove stale remote asset $name; deferring it."
    return 2
  fi

  return 1
}

upload_and_verify_asset() {
  local file="$1"
  local name="$2"
  local expected_digest="$3"
  local prepare_status
  local upload_succeeded=0
  local poll

  if prepare_remote_asset "$name" "$expected_digest"; then
    echo "  Verified existing upload: $name"
    return 0
  else
    prepare_status=$?
  fi

  if [ "$prepare_status" -ne 1 ]; then
    return 1
  fi

  echo "  Uploading $name..."
  if gh release upload "$tag" "$file" --repo "$GITHUB_REPOSITORY"; then
    upload_succeeded=1
  else
    echo "::warning::Upload command failed for $name; checking whether GitHub completed it asynchronously."
  fi

  # A failed request can leave an asset that transitions from starter to
  # uploaded moments later. Poll before a future round is allowed to delete it.
  for poll in 1 2 3; do
    if [ "$poll" -gt 1 ]; then
      sleep "$poll_delay"
    fi
    if fetch_remote_asset "$name" && \
      [ "$remote_state" = "uploaded" ] && \
      [ "$remote_digest" = "$expected_digest" ]; then
      echo "  Uploaded and verified: $name"
      return 0
    fi
  done

  if [ "$remote_state" = "starter" ] && [ -n "$remote_id" ]; then
    : > "$starter_markers/$remote_id"
  fi

  if [ "$upload_succeeded" -eq 1 ]; then
    echo "::warning::GitHub reported a successful upload for $name, but its digest could not yet be verified."
  fi
  return 1
}

pending="$artifact_manifest"

for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  next_pending="$work_dir/pending-${attempt}"
  : > "$next_pending"
  echo "Release upload round $attempt of $max_attempts"

  while IFS=$'\t' read -r expected_digest remote_name file; do
    if upload_and_verify_asset "$file" "$remote_name" "$expected_digest"; then
      continue
    fi
    printf '%s\t%s\t%s\n' "$expected_digest" "$remote_name" "$file" >> "$next_pending"
  done < "$pending"

  if [ ! -s "$next_pending" ]; then
    echo "All release assets were uploaded and verified."
    exit 0
  fi

  pending="$next_pending"
  if [ "$attempt" -eq "$max_attempts" ]; then
    break
  fi

  retry_wait "$attempt" "Retrying remaining assets"
done

# Reconcile one last time after a failed upload response. GitHub can finish an
# asset transition immediately after the final in-round poll.
final_pending="$work_dir/pending-final"
: > "$final_pending"
if (( poll_delay > 0 )); then
  sleep "$poll_delay"
fi
while IFS=$'\t' read -r expected_digest remote_name file; do
  if fetch_remote_asset "$remote_name" && \
    [ "$remote_state" = "uploaded" ] && \
    [ "$remote_digest" = "$expected_digest" ]; then
    echo "  Uploaded and verified during final reconciliation: $remote_name"
    continue
  fi
  printf '%s\t%s\t%s\n' "$expected_digest" "$remote_name" "$file" >> "$final_pending"
done < "$pending"
pending="$final_pending"

if [ ! -s "$pending" ]; then
  echo "All release assets were uploaded and verified."
  exit 0
fi

while IFS=$'\t' read -r _ remote_name _; do
  echo "::error::Failed to upload and verify $remote_name after $max_attempts rounds."
done < "$pending"

exit 1
