#!/bin/sh
set -eu

arv-fake-gv-camera-0.8 -i 127.0.0.1 -s FLOWGENICAM > /tmp/aravis.log 2>&1 &
camera_pid=$!
cleanup() {
    kill -INT "$camera_pid" 2>/dev/null || true
    wait "$camera_pid" 2>/dev/null || true
    cat /tmp/aravis.log
}
trap cleanup EXIT INT TERM

genicam-e2e --ignored --test-threads=1 --nocapture "$@"
