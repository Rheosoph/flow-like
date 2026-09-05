#!/bin/sh
set -eu
umask 077
# Passwords are generated as hexadecimal strings. Refuse ACL syntax injection.
for value in "$REDIS_API_PASSWORD" "$REDIS_RUNTIME_PASSWORD" "$REDIS_SIGNALING_PASSWORD" "$REDIS_SINK_PASSWORD" "$REDIS_METRICS_PASSWORD"; do
  case "$value" in ''|*[!a-fA-F0-9]*) echo 'Redis passwords must be hexadecimal' >&2; exit 1;; esac
  [ "${#value}" -ge 64 ] || { echo 'Redis passwords require 32 random bytes' >&2; exit 1; }
done
cat > /tmp/users.acl <<ACL
user default off
user api on >$REDIS_API_PASSWORD ~* &* +@all -@dangerous +client|setname +client|setinfo
user runtime on >$REDIS_RUNTIME_PASSWORD ~exec:* ~execution:* &* +time +@read +@write +@list +@stream +@scripting +ping +client|setname +client|setinfo -@dangerous
user signaling on >$REDIS_SIGNALING_PASSWORD &signal:* +ping +publish +subscribe +unsubscribe +psubscribe +punsubscribe +client|setname +client|setinfo
user sink on >$REDIS_SINK_PASSWORD ~sink:* +@read +@write +@scripting +ping +client|setname +client|setinfo -@dangerous +keys
user metrics on >$REDIS_METRICS_PASSWORD +ping +info +client|list +slowlog|get +slowlog|len +latency|latest +config|get +memory|stats
ACL
exec redis-server --appendonly yes --appendfsync everysec --aclfile /tmp/users.acl --maxmemory "$REDIS_MAXMEMORY" --maxmemory-policy noeviction --protected-mode yes
