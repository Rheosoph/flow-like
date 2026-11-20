#!/bin/sh
# Cargo runner on macOS. The linker signs every build ad hoc, and the keychain binds
# "Always Allow" to that one build's hash, so each rebuild of the desktop app asks again
# for every item it stores. Signed with a stable identity under the bundle id, rebuilds
# keep the approvals and share them with the released app. Cargo copies the unsigned
# binary out of deps/ on every run, so every run signs it again (a few seconds).
set -u

binary=${1:?usage: macos-cargo-runner.sh <binary> [args...]}

signing_identity() {
	if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
		printf '%s\n' "$APPLE_SIGNING_IDENTITY"
		return
	fi
	security find-identity -v -p codesigning 2>/dev/null | awk '
		/"Developer ID Application:/ && developer_id == "" { developer_id = $2 }
		/"Apple Development:/ && development == "" { development = $2 }
		END { print (developer_id != "" ? developer_id : development) }'
}

if [ "$(basename "$binary")" = flow-like-desktop ] &&
	codesign -dv "$binary" 2>&1 | grep -q '^TeamIdentifier=not set$'; then
	identity=$(signing_identity)
	if [ -n "$identity" ] &&
		! output=$(codesign --force --sign "$identity" --identifier com.flow-like.app --timestamp=none "$binary" 2>&1); then
		printf '%s\nmacos-cargo-runner: signing %s with %s failed; the keychain will ask again for this build\n' \
			"$output" "$binary" "$identity" >&2
	fi
fi

exec "$@"
