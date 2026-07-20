#!/usr/bin/env bash

set -euo pipefail

max_attempts="${FINALIZE_MAX_ATTEMPTS:-6}"
base_delay="${FINALIZE_BASE_DELAY_SECONDS:-15}"
max_delay="${FINALIZE_MAX_DELAY_SECONDS:-300}"
config_path="${TAURI_CONFIG_PATH:-apps/desktop/src-tauri/tauri.conf.json}"

require_integer() {
  local name="$1"
  local value="$2"

  if ! [[ "$value" =~ ^[0-9]+$ ]]; then
    echo "::error::$name must be a non-negative integer, got: $value"
    exit 1
  fi
}

require_integer FINALIZE_MAX_ATTEMPTS "$max_attempts"
require_integer FINALIZE_BASE_DELAY_SECONDS "$base_delay"
require_integer FINALIZE_MAX_DELAY_SECONDS "$max_delay"

if (( max_attempts < 1 || max_attempts > 10 )); then
  echo "::error::FINALIZE_MAX_ATTEMPTS must be between 1 and 10."
  exit 1
fi

if [ -z "${GITHUB_REPOSITORY:-}" ] || \
  ! [[ "$GITHUB_REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "::error::GITHUB_REPOSITORY must be an owner/repository pair."
  exit 1
fi

if [ -z "${GITHUB_RUN_ID:-}" ] || ! [[ "$GITHUB_RUN_ID" =~ ^[0-9]+$ ]]; then
  echo "::error::GITHUB_RUN_ID must be numeric."
  exit 1
fi

run_attempt="${GITHUB_RUN_ATTEMPT:-1}"
if ! [[ "$run_attempt" =~ ^[0-9]+$ ]]; then
  echo "::error::GITHUB_RUN_ATTEMPT must be numeric."
  exit 1
fi

for command_name in gh jq mktemp; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "::error::Required command is unavailable: $command_name"
    exit 1
  fi
done

app_version="${APP_VERSION:-}"
if [ -z "$app_version" ]; then
  if [ ! -f "$config_path" ] || \
    ! app_version="$(jq -er '.version | strings | select(length > 0)' "$config_path")"; then
    echo "::error::APP_VERSION is unset and no version could be read from $config_path."
    exit 1
  fi
fi

if ! [[ "$app_version" =~ ^[0-9A-Za-z.+-]+$ ]]; then
  echo "::error::APP_VERSION contains characters that are unsafe in a release tag."
  exit 1
fi

tag="beta-v${app_version}"
work_dir="$(mktemp -d)"

cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

retry_wait() {
  local attempt="$1"
  local message="$2"
  local delay

  delay=$((base_delay * (1 << (attempt - 1))))
  if (( delay > max_delay )); then
    delay="$max_delay"
  fi
  echo "$message in ${delay}s..."
  if (( delay > 0 )); then
    sleep "$delay"
  fi
}

sha256_digest() {
  local file="$1"
  local output
  local hash

  if command -v sha256sum >/dev/null 2>&1; then
    if ! output="$(sha256sum "$file")"; then
      return 1
    fi
  elif command -v shasum >/dev/null 2>&1; then
    if ! output="$(shasum -a 256 "$file")"; then
      return 1
    fi
  else
    echo "::error::Neither sha256sum nor shasum is available."
    return 1
  fi

  hash="${output%% *}"
  if ! [[ "$hash" =~ ^[0-9a-fA-F]{64}$ ]]; then
    return 1
  fi
  hash="$(printf '%s' "$hash" | tr '[:upper:]' '[:lower:]')"
  printf 'sha256:%s\n' "$hash"
}

release_file="$work_dir/release.json"
for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  release_pages="$release_file.pages"
  release_part="$release_file.part"
  rm -f "$release_pages" "$release_part"
  # GitHub's release-by-tag endpoint omits drafts. Search the authenticated
  # release list, matching tauri-action's draft lookup behavior.
  if gh api \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    --paginate \
    --slurp \
    "repos/${GITHUB_REPOSITORY}/releases?per_page=100" > "$release_pages"; then
    if jq -e --arg tag "$tag" '
      if type != "array" or any(.[]; type != "array") then
        error("invalid paginated release response")
      else
        [.[][] | select(.tag_name == $tag and .draft == true)]
        | if length == 1 and (.[0].id | type) == "number" and
            (.[0].created_at | type) == "string"
          then .[0]
          else error("draft release was missing or ambiguous")
          end
      end
    ' "$release_pages" > "$release_part" && \
      mv "$release_part" "$release_file"; then
      rm -f "$release_pages"
      break
    fi
  fi
  rm -f "$release_pages" "$release_part"

  if [ "$attempt" -eq "$max_attempts" ]; then
    echo "::error::Could not resolve release $tag."
    exit 1
  fi
  retry_wait "$attempt" "Retrying release lookup"
done

if ! release_id="$(jq -er '.id | tostring' "$release_file")" || \
  ! [[ "$release_id" =~ ^[0-9]+$ ]]; then
  echo "::error::The release response did not contain a numeric ID."
  exit 1
fi

# gh --paginate emits one array per page; --slurp makes those pages safe to
# validate and flatten even when the release grows beyond 100 assets.
snapshot_once() {
  local destination="$1"
  local pages="$destination.pages"
  local flat="$destination.flat"

  rm -f "$pages" "$flat"
  if ! gh api \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    --paginate \
    --slurp \
    "repos/${GITHUB_REPOSITORY}/releases/${release_id}/assets?per_page=100" \
    > "$pages"; then
    return 1
  fi
  if ! jq -e \
    'type == "array" and all(.[]; type == "array")' \
    "$pages" >/dev/null; then
    return 1
  fi
  if ! jq '[.[][]]' "$pages" > "$flat"; then
    return 1
  fi
  if ! mv "$flat" "$destination"; then
    return 1
  fi
  rm -f "$pages"
  return 0
}

snapshot_with_retry() {
  local destination="$1"
  local label="$2"
  local attempt

  for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    if snapshot_once "$destination"; then
      return 0
    fi
    if [ "$attempt" -lt "$max_attempts" ]; then
      retry_wait "$attempt" "$label"
    fi
  done
  return 1
}

assets_file="$work_dir/assets.json"
if ! snapshot_with_retry "$assets_file" "Retrying release asset listing"; then
  echo "::error::Could not list assets for release $tag."
  exit 1
fi

# Recognize only Tauri's direct (unzipped) updater signatures. Exact payload
# pairing prevents a partial or similarly named upload from entering metadata.
candidates_file="$work_dir/candidates.json"
if ! jq -e \
  --arg tag "$tag" '
  def updater_mapping:
    if test("_aarch64\\.app\\.tar\\.gz\\.sig$") then
      {base: "darwin-aarch64", bundle: "app"}
    elif test("_x64\\.app\\.tar\\.gz\\.sig$") then
      {base: "darwin-x86_64", bundle: "app"}
    elif test("_aarch64\\.AppImage\\.sig$") then
      {base: "linux-aarch64", bundle: "appimage"}
    elif test("_arm64\\.deb\\.sig$") then
      {base: "linux-aarch64", bundle: "deb"}
    elif test("\\.aarch64\\.rpm\\.sig$") then
      {base: "linux-aarch64", bundle: "rpm"}
    elif test("_amd64\\.AppImage\\.sig$") then
      {base: "linux-x86_64", bundle: "appimage"}
    elif test("_amd64\\.deb\\.sig$") then
      {base: "linux-x86_64", bundle: "deb"}
    elif test("\\.x86_64\\.rpm\\.sig$") then
      {base: "linux-x86_64", bundle: "rpm"}
    elif test("_x64_en-US\\.msi\\.sig$") then
      {base: "windows-x86_64", bundle: "msi"}
    elif test("_x64-setup\\.exe\\.sig$") then
      {base: "windows-x86_64", bundle: "nsis"}
    else null
    end;

  map(select(.state == "uploaded")) as $assets
  | [
      $assets[]
      | select(.name | type == "string")
      | . as $sig
      | ($sig.name | updater_mapping) as $mapping
      | select($mapping != null)
      | ($sig.name | sub("\\.sig$"; "")) as $payload_name
      | [$assets[] | select(.name == $payload_name)] as $payloads
      | if ($payloads | length) != 1 then
          error("missing or ambiguous payload for " + $sig.name)
        elif (($sig.id | type) != "number") or
             (($sig.digest // "") | type) != "string" or
             (($payloads[0].browser_download_url // "") | type) != "string" or
             (($payloads[0].browser_download_url // "") | length) == 0 then
          error("incomplete asset metadata for " + $sig.name)
        else
          $mapping + {
            sig_id: $sig.id,
            sig_name: $sig.name,
            sig_digest: ($sig.digest // ""),
            payload_name: $payload_name,
            url: ($payloads[0].browser_download_url
              | sub("/download/untagged-[^/]+/"; ("/download/" + $tag + "/")))
          }
        end
    ]
  | . as $records
  | ($records | group_by([.base, .bundle])) as $groups
  | if any($groups[]; length != 1) then
      error("duplicate updater signature for a platform bundle")
    else $records
    end
  ' "$assets_file" > "$candidates_file"; then
  echo "::error::Uploaded updater assets are incomplete or ambiguous."
  exit 1
fi

candidate_lines="$work_dir/candidate-lines"
if ! jq -c '.[]' "$candidates_file" > "$candidate_lines"; then
  echo "::error::Could not enumerate updater signatures."
  exit 1
fi

download_signature() {
  local asset_id="$1"
  local expected_digest="$2"
  local destination="$3"
  local attempt
  local actual_digest
  local part="$destination.part"

  if ! [[ "$expected_digest" =~ ^sha256:[0-9a-fA-F]{64}$ ]]; then
    echo "::error::Signature asset $asset_id has no verifiable SHA-256 digest."
    return 1
  fi
  expected_digest="$(printf '%s' "$expected_digest" | tr '[:upper:]' '[:lower:]')"

  for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    rm -f "$part"
    if gh api \
      -H "Accept: application/octet-stream" \
      -H "X-GitHub-Api-Version: 2022-11-28" \
      "repos/${GITHUB_REPOSITORY}/releases/assets/${asset_id}" > "$part"; then
      if [ -s "$part" ] && actual_digest="$(sha256_digest "$part")" && \
        [ "$actual_digest" = "$expected_digest" ]; then
        if ! mv "$part" "$destination"; then
          return 1
        fi
        return 0
      fi
    fi
    rm -f "$part"
    if [ "$attempt" -lt "$max_attempts" ]; then
      retry_wait "$attempt" "Retrying signature asset $asset_id"
    fi
  done
  return 1
}

enriched_lines="$work_dir/enriched-lines"
: > "$enriched_lines"
record_index=0
while IFS= read -r record; do
  record_index=$((record_index + 1))
  if ! sig_id="$(jq -er '.sig_id | tostring' <<< "$record")" || \
    ! sig_digest="$(jq -er '.sig_digest' <<< "$record")"; then
    echo "::error::Could not read signature metadata."
    exit 1
  fi
  signature_file="$work_dir/signature-${record_index}"
  if ! download_signature "$sig_id" "$sig_digest" "$signature_file"; then
    echo "::error::Could not download and verify signature asset $sig_id."
    exit 1
  fi
  # --rawfile retains the signature exactly, including trailing newlines.
  if ! jq -cn \
    --argjson record "$record" \
    --rawfile signature "$signature_file" \
    '$record + {signature: $signature}' >> "$enriched_lines"; then
    echo "::error::Could not encode signature asset $sig_id."
    exit 1
  fi
done < "$candidate_lines"

enriched_file="$work_dir/enriched.json"
if ! jq -s '.' "$enriched_lines" > "$enriched_file"; then
  echo "::error::Could not assemble updater records."
  exit 1
fi

# Match tauri-action's primary priorities: app archive on macOS, AppImage on
# Linux, and MSI with NSIS as the Windows fallback.
platforms_file="$work_dir/platforms.json"
if ! jq -e '
  def selected($base; $bundle):
    ([.[] | select(.base == $base and .bundle == $bundle)][0] // null);
  def primary($base):
    if ($base | startswith("darwin-")) then selected($base; "app")
    elif ($base | startswith("linux-")) then selected($base; "appimage")
    elif $base == "windows-x86_64" then
      (selected($base; "msi") // selected($base; "nsis"))
    else null
    end;
  def value($record):
    {signature: $record.signature, url: $record.url};

  . as $records
  | ["darwin-aarch64", "darwin-x86_64", "linux-aarch64",
      "linux-x86_64", "windows-x86_64"] as $required
  | ([$records[] | {
      key: (.base + "-" + .bundle),
      value: value(.)
    }]) as $bundles
  | ([$required[] as $base
      | (primary($base)) as $record
      | if $record == null then error("missing primary updater for " + $base)
        else {key: $base, value: value($record)}
        end]) as $primaries
  | ($bundles + $primaries | from_entries)
  ' "$enriched_file" > "$platforms_file"; then
  echo "::error::All five primary updater platforms are required."
  exit 1
fi

latest_file="$work_dir/latest.json"
if ! jq -S \
  --arg version "$app_version" \
  --slurpfile platforms "$platforms_file" '
    {
      version: $version,
      notes: (.body // ""),
      pub_date: .created_at,
      platforms: $platforms[0]
    }
  ' "$release_file" > "$latest_file"; then
  echo "::error::Could not generate latest.json."
  exit 1
fi

if ! expected_digest="$(sha256_digest "$latest_file")"; then
  echo "::error::Could not hash generated latest.json."
  exit 1
fi

temp_name="latest.json.${GITHUB_RUN_ID}.${run_attempt}.tmp"
staged_file="$work_dir/$temp_name"

asset_state=""
asset_id=""
asset_digest=""

find_named_asset() {
  local snapshot="$1"
  local name="$2"
  local count
  local fields

  asset_state=""
  asset_id=""
  asset_digest=""

  if ! count="$(jq -er --arg name "$name" \
    '[.[] | select(.name == $name)] | length' "$snapshot")"; then
    return 2
  fi
  if [ "$count" -eq 0 ]; then
    return 1
  fi
  if [ "$count" -ne 1 ]; then
    return 2
  fi
  if ! fields="$(jq -er --arg name "$name" '
    [.[] | select(.name == $name)][0]
    | [(.state // ""), ((.id // "") | tostring), (.digest // "")]
    | @tsv
  ' "$snapshot")"; then
    return 2
  fi
  if ! IFS=$'\t' read -r asset_state asset_id asset_digest <<< "$fields" || \
    [ -z "$asset_state" ] || ! [[ "$asset_id" =~ ^[0-9]+$ ]]; then
    return 2
  fi
  return 0
}

delete_asset() {
  local id="$1"
  gh api \
    --method DELETE \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "repos/${GITHUB_REPOSITORY}/releases/assets/${id}" \
    --silent
}

# A rerun commonly finds metadata already finalized by an earlier attempt.
# Reuse the digest-verified canonical asset without making any remote mutation.
if find_named_asset "$assets_file" "latest.json" && \
  [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
  if find_named_asset "$assets_file" "$temp_name"; then
    if ! delete_asset "$asset_id"; then
      echo "::warning::Final metadata is correct, but temporary asset $temp_name remains."
    fi
  fi
  echo "Verified existing latest.json for $tag with five updater platforms."
  exit 0
fi

if ! ln "$latest_file" "$staged_file"; then
  echo "::error::Could not stage latest.json for upload."
  exit 1
fi

stage_snapshot="$work_dir/stage-snapshot.json"
deferred_stage_id=""
staged_id=""

for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  recheck_now=0
  if snapshot_once "$stage_snapshot"; then
    if find_named_asset "$stage_snapshot" "$temp_name"; then
      if [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
        staged_id="$asset_id"
        break
      fi

      if [ "$asset_state" = "starter" ] || \
        { [ "$asset_state" = "uploaded" ] && [ -z "$asset_digest" ]; }; then
        if [ "$deferred_stage_id" != "$asset_id" ]; then
          deferred_stage_id="$asset_id"
        else
          if delete_asset "$asset_id"; then
            recheck_now=1
          else
            echo "::warning::Could not remove incomplete staged metadata asset $asset_id."
            recheck_now=1
          fi
        fi
      else
        if delete_asset "$asset_id"; then
          recheck_now=1
        else
          echo "::warning::Could not remove stale staged metadata asset $asset_id."
          recheck_now=1
        fi
      fi
    else
      find_status=$?
      if [ "$find_status" -eq 1 ]; then
        if ! gh release upload "$tag" "$staged_file" --repo "$GITHUB_REPOSITORY"; then
          echo "::warning::Staged latest.json upload failed; its remote state will be reconciled."
        fi
        recheck_now=1
      else
        echo "::warning::Could not interpret the staged asset listing."
      fi
    fi
  else
    echo "::warning::Could not inspect the staged metadata asset."
  fi

  # Reconcile a mutation once immediately. If it has not settled, preserve the
  # exponential backoff before spending another numbered retry attempt.
  if [ "$recheck_now" -eq 1 ] && snapshot_once "$stage_snapshot"; then
    if find_named_asset "$stage_snapshot" "$temp_name" && \
      [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
      staged_id="$asset_id"
      break
    fi
  fi

  if [ "$attempt" -lt "$max_attempts" ]; then
    retry_wait "$attempt" "Retrying staged latest.json verification"
  fi
done

# Every mutation gets a final reconciliation read, including one made during
# the last retry round or one whose client response was ambiguous.
if ! [[ "$staged_id" =~ ^[0-9]+$ ]] && snapshot_once "$stage_snapshot"; then
  if find_named_asset "$stage_snapshot" "$temp_name"; then
    if [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
      staged_id="$asset_id"
    fi
  else
    final_stage_status=$?
    # A confirmed or ambiguous DELETE in the last round can leave the name
    # safely vacant. Give that transition one upload and verification read.
    if [ "$final_stage_status" -eq 1 ]; then
      if ! gh release upload "$tag" "$staged_file" --repo "$GITHUB_REPOSITORY"; then
        echo "::warning::Final staged upload response was ambiguous; verifying it."
      fi
      if snapshot_once "$stage_snapshot" && \
        find_named_asset "$stage_snapshot" "$temp_name" && \
        [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
        staged_id="$asset_id"
      fi
    fi
  fi
fi

if ! [[ "$staged_id" =~ ^[0-9]+$ ]]; then
  echo "::error::Could not upload and SHA-256 verify staged latest.json."
  exit 1
fi

# The candidate is safely remote before the old name is removed. Each loop
# re-lists state, so failed DELETE/PATCH calls are reconciled before retrying.
replace_snapshot="$work_dir/replace-snapshot.json"
final_id=""
for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  recheck_now=0
  if snapshot_once "$replace_snapshot"; then
    if find_named_asset "$replace_snapshot" "latest.json"; then
      latest_status=0
    else
      latest_status=$?
    fi

    if [ "$latest_status" -eq 0 ]; then
      if [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
        final_id="$asset_id"
        break
      fi
      if delete_asset "$asset_id"; then
        # The verified candidate is already remote. Rename it immediately
        # after a confirmed delete so the canonical name is absent briefly.
        if ! gh api \
          --method PATCH \
          -H "X-GitHub-Api-Version: 2022-11-28" \
          "repos/${GITHUB_REPOSITORY}/releases/assets/${staged_id}" \
          -f name="latest.json" >/dev/null; then
          echo "::warning::PATCH rename was ambiguous; rechecking both asset names."
        fi
        recheck_now=1
      else
        echo "::warning::DELETE for the old latest.json was ambiguous; rechecking it."
        recheck_now=1
      fi
    elif [ "$latest_status" -eq 1 ]; then
      if find_named_asset "$replace_snapshot" "$temp_name"; then
        temp_status=0
      else
        temp_status=$?
      fi
      if [ "$temp_status" -eq 0 ] && [ "$asset_id" = "$staged_id" ] && \
        [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
        if ! gh api \
          --method PATCH \
          -H "X-GitHub-Api-Version: 2022-11-28" \
          "repos/${GITHUB_REPOSITORY}/releases/assets/${staged_id}" \
          -f name="latest.json" >/dev/null; then
          echo "::warning::PATCH rename was ambiguous; rechecking both asset names."
        fi
        recheck_now=1
      else
        echo "::warning::Verified staged asset is temporarily absent from the listing; rechecking."
      fi
    else
      echo "::warning::Could not interpret latest.json asset state."
    fi
  else
    echo "::warning::Could not inspect latest.json replacement state."
  fi

  # A successful or ambiguous mutation gets one immediate read. An unsettled
  # state still backs off before consuming the next retry attempt.
  if [ "$recheck_now" -eq 1 ] && snapshot_once "$replace_snapshot"; then
    if find_named_asset "$replace_snapshot" "latest.json" && \
      [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
      final_id="$asset_id"
      break
    fi
  fi

  if [ "$attempt" -lt "$max_attempts" ]; then
    retry_wait "$attempt" "Retrying latest.json replacement"
  fi
done

# Reconcile the last DELETE/PATCH even when it happened on the final round. If
# an ambiguous DELETE removed the old name, complete the already-staged rename
# and verify its result once more.
if ! [[ "$final_id" =~ ^[0-9]+$ ]] && snapshot_once "$replace_snapshot"; then
  if find_named_asset "$replace_snapshot" "latest.json"; then
    if [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
      final_id="$asset_id"
    fi
  else
    final_latest_status=$?
    if [ "$final_latest_status" -eq 1 ] && \
      find_named_asset "$replace_snapshot" "$temp_name" && \
      [ "$asset_id" = "$staged_id" ] && \
      [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
      if ! gh api \
        --method PATCH \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "repos/${GITHUB_REPOSITORY}/releases/assets/${staged_id}" \
        -f name="latest.json" >/dev/null; then
        echo "::warning::Final PATCH rename response was ambiguous; verifying it."
      fi
      if snapshot_once "$replace_snapshot" && \
        find_named_asset "$replace_snapshot" "latest.json" && \
        [ "$asset_state" = "uploaded" ] && [ "$asset_digest" = "$expected_digest" ]; then
        final_id="$asset_id"
      fi
    fi
  fi
fi

if ! [[ "$final_id" =~ ^[0-9]+$ ]]; then
  echo "::error::Could not safely replace and verify latest.json."
  exit 1
fi

# A rerun can find an already-correct latest.json while its verified temp copy
# still exists. It is harmless, but remove it when possible.
if [ "$final_id" != "$staged_id" ] && snapshot_once "$replace_snapshot"; then
  if find_named_asset "$replace_snapshot" "$temp_name"; then
    if ! delete_asset "$asset_id"; then
      echo "::warning::Final metadata is correct, but temporary asset $temp_name remains."
    fi
  fi
fi

echo "Generated, uploaded, and verified latest.json for $tag with five updater platforms."
