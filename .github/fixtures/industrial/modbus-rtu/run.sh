#!/bin/sh
set -eu

export FLOW_LIKE_MODBUS_RTU_E2E_PORT=/tmp/modbus-client
export FLOW_LIKE_MODBUS_RTU_E2E_STATE=/tmp/modbus-rtu-state
mkdir "$FLOW_LIKE_MODBUS_RTU_E2E_STATE"
socat PTY,raw,echo=0,link=/tmp/modbus-client PTY,raw,echo=0,link=/tmp/modbus-server &
serial_pid=$!
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
    kill "$serial_pid" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
timeout 10 sh -c 'until [ -e /tmp/modbus-client ] && [ -e /tmp/modbus-server ]; do sleep 0.05; done'
python /fixture/server.py &
server_pid=$!
timeout 10 sh -c 'until [ -f "$FLOW_LIKE_MODBUS_RTU_E2E_STATE/ready" ]; do sleep 0.05; done'
test "$(modbus-rtu-e2e --ignored --list | grep -c ': test$')" -eq 3
timeout 60 modbus-rtu-e2e --ignored --test-threads=1 --nocapture
