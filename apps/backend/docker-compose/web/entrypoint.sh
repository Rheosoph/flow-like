#!/bin/sh
set -eu

# The filesystem can be read-only; the deployment must provide a writable /tmp.
python3 -B /usr/local/bin/flow-like-web-runtime-config.py \
    --nginx-output /tmp/flow-like-web/hosted-proxy.conf
exec "$@"
