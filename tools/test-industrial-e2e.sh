#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
MODE="${1:---all}"
case "$MODE" in
  --all|--adapters) SERVICES=(nats redis rabbitmq redpanda modbus opcua mosquitto cip ads hart iolink-contract zenoh zenoh-peer sparkplug-peer) ;;
  --mqtt) SERVICES=(mosquitto) ;;
  --devices) SERVICES=() ;;
  *) echo "Usage: $0 [--all|--adapters|--mqtt|--devices]" >&2; exit 2 ;;
esac

# Build native fixtures even when the active Buildx builder defaults to a remote
# architecture. The IO-Link contract image declares its own amd64 requirement.
export DOCKER_DEFAULT_PLATFORM="${DOCKER_DEFAULT_PLATFORM:-linux/$(docker version --format '{{.Server.Arch}}')}"

TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/flow-industrial-e2e.XXXXXX")"
PROJECT="flow-industrial-e2e-$$-$RANDOM"
COMPOSE=(docker compose -p "$PROJECT" -f .github/fixtures/industrial-brokers.yml)
cleanup() {
  local status=$?
  trap - EXIT
  if (( status != 0 )); then
    "${COMPOSE[@]}" logs --no-color --tail=100 >&2 || true
  fi
  "${COMPOSE[@]}" down --rmi local --volumes --remove-orphans || status=1
  rm -rf "$TEST_DIR"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if (( ${#SERVICES[@]} > 0 )); then
  "${COMPOSE[@]}" up -d --build --wait --wait-timeout 240 "${SERVICES[@]}"
fi
export RUSTC_WRAPPER=""
run_ignored_library_tests() {
  local package="$1" filter="$2"
  local listing="$TEST_DIR/$package-tests.txt"
  cargo test --locked -p "$package" --features execute --lib "$filter" -- --ignored --list > "$listing"
  if ! grep -q ': test$' "$listing"; then
    echo "No ignored $package tests match $filter" >&2
    return 1
  fi
  cargo test --locked -p "$package" --features execute --lib "$filter" -- --ignored --test-threads=1
}
if [[ "$MODE" == --all || "$MODE" == --adapters ]]; then
  export FLOW_LIKE_NATS_URL=nats://127.0.0.1:18422
  export FLOW_LIKE_REDIS_URL=redis://127.0.0.1:16379
  export FLOW_LIKE_AMQP_URL=amqp://flowtest:flowtest-tests-only@127.0.0.1:15672/%2f
  export FLOW_LIKE_KAFKA_BROKERS=127.0.0.1:19093
  export FLOW_LIKE_MODBUS_E2E_ADDR=127.0.0.1:15020
  export FLOW_LIKE_CIP_E2E_ADDR=127.0.0.1:44819
  export FLOW_LIKE_ADS_E2E_ADDR=127.0.0.1:48899
  export FLOW_LIKE_HART_E2E_ADDR=127.0.0.1:15094
  export FLOW_LIKE_IOLINK_CONTRACT_URL=http://127.0.0.1:18082
  export FLOW_LIKE_ZENOH_ROUTER=tcp/127.0.0.1:17447
  export FLOW_LIKE_ZENOH_ORACLE_URL=http://127.0.0.1:18081
  export FLOW_LIKE_SPARKPLUG_MQTT_HOST=127.0.0.1
  export FLOW_LIKE_SPARKPLUG_MQTT_PORT=18883
  export FLOW_LIKE_SPARKPLUG_ORACLE_URL=http://127.0.0.1:18089
  export INDUSTRIAL_OPCUA_ENDPOINT=opc.tcp://127.0.0.1:14840/flow-like/
  export INDUSTRIAL_OPCUA_CERT_DIR="$TEST_DIR/opcua"
  mkdir -p "$INDUSTRIAL_OPCUA_CERT_DIR"
  "${COMPOSE[@]}" cp opcua:/fixture-pki/. "$INDUSTRIAL_OPCUA_CERT_DIR"
  run_ignored_library_tests flow-like-industrial broker_
  TEST_SUITES=(modbus_e2e opcua_e2e cip_e2e ads_e2e hart_e2e iolink_contract_e2e zenoh_e2e sparkplug_e2e)
  for suite in "${TEST_SUITES[@]}"; do
    cargo test --locked -p flow-like-industrial --features execute --test "$suite" -- --ignored --list > "$TEST_DIR/$suite.txt"
    if ! grep -q ': test$' "$TEST_DIR/$suite.txt"; then
      echo "No ignored tests found in $suite" >&2
      exit 1
    fi
    cargo test --locked -p flow-like-industrial --features execute --test "$suite" -- --ignored --test-threads=1
  done
fi
if [[ "$MODE" == --all || "$MODE" == --mqtt ]]; then
  export FLOW_LIKE_MQTT_HOST=127.0.0.1
  export FLOW_LIKE_MQTT_PORT=18883
  run_ignored_library_tests flow-like-catalog-web mosquitto_
fi
if [[ "$MODE" == --all || "$MODE" == --devices ]]; then
  # These fixtures run the exact adapter source inside Linux beside the peer.
  # GigE Vision needs a reachable UDP return address that Docker Desktop NAT
  # cannot advertise from the macOS host.
  "${COMPOSE[@]}" run --build --rm genicam
  "${COMPOSE[@]}" run --build --rm iroh
  "${COMPOSE[@]}" run --build --rm ethercat
  "${COMPOSE[@]}" run --build --rm modbus-rtu
fi
