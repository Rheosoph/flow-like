#!/bin/sh
set -eu

# Both raw Ethernet endpoints stay in this container's private network namespace.
ip link add flow-master type veth peer name flow-peer
ip link set flow-master up
ip link set flow-peer up
trap 'ip link delete flow-master 2>/dev/null || true' EXIT
export FLOW_LIKE_ETHERCAT_INTERFACE=flow-master
export FLOW_LIKE_ETHERCAT_PEER_INTERFACE=flow-peer
export FLOW_LIKE_ETHERCAT_PEER_BIN=/usr/local/bin/ethercat-peer
export FLOW_LIKE_ETHERCAT_PEER_CONFIG=/fixture/device.json
test "$(ethercat-e2e --ignored --list | grep -c ': test$')" -eq 3
ethercat-e2e --ignored --test-threads=1 --nocapture
