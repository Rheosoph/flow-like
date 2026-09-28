#!/bin/sh
set -eu
: "${MAIL_POSTFIX_HOSTNAME:?Set the public MX hostname}"
: "${INBOUND_MAIL_DOMAIN:?Set the receiving domain}"
case "$MAIL_POSTFIX_HOSTNAME:$INBOUND_MAIL_DOMAIN" in
  *[!a-zA-Z0-9.:-]*) echo 'Invalid mail hostname or domain' >&2; exit 1 ;;
esac
MAIL_MAX_MESSAGE_BYTES="${MAIL_MAX_MESSAGE_BYTES:-10485760}"
case "$MAIL_MAX_MESSAGE_BYTES" in
  ''|*[!0-9]*) echo 'Invalid message size limit' >&2; exit 1 ;;
esac
test -r /run/secrets/mail_tls_certificate
test -r /run/secrets/mail_tls_key
postconf -e "myhostname=$MAIL_POSTFIX_HOSTNAME"
postconf -e "relay_domains=$INBOUND_MAIL_DOMAIN"
postconf -e "message_size_limit=$MAIL_MAX_MESSAGE_BYTES"
postfix set-permissions
postfix check
exec postfix start-fg

