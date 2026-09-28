#!/usr/bin/env bash
# Starts a built signaling image and requires it to report ready. Bun resolves
# static imports at startup, so an image missing a module exits here instead
# of crash-looping after publication.
set -euo pipefail

image="${1:?usage: image-smoke.sh <image> [platform]}"
platform="${2:-}"

public_key="$(openssl ecparam -name prime256v1 -genkey -noout | openssl ec -pubout 2>/dev/null | base64 | tr -d '\n')"
container="$(docker run -d ${platform:+--platform "$platform"} -p 127.0.0.1::4444 \
  -e PORT=4444 \
  -e REALTIME_FANOUT_MODE=local \
  -e REALTIME_ALLOWED_ORIGINS=https://signaling-smoke.invalid \
  -e BACKEND_PUB="$public_key" \
  "$image")"
trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT

for _ in $(seq 1 60); do
  if [ "$(docker inspect -f '{{.State.Running}}' "$container")" != "true" ]; then
    break
  fi
  port="$(docker port "$container" 4444/tcp | head -n1 | sed 's/.*://')"
  if [ -n "$port" ] && curl -fsS "http://127.0.0.1:${port}/ready" >/dev/null 2>&1; then
    echo "Signaling image ${image} is ready."
    exit 0
  fi
  sleep 1
done

docker logs "$container" >&2 || true
echo "Signaling image ${image} did not become ready." >&2
exit 1
