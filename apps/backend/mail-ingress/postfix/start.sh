#!/bin/sh
set -eu
: "${MAIL_POSTFIX_HOSTNAME:?Set the public MX hostname}"
: "${INBOUND_MAIL_DOMAIN:?Set the receiving domain}"
for value in "$MAIL_POSTFIX_HOSTNAME" "$INBOUND_MAIL_DOMAIN"; do
  case "$value" in
    *[!a-zA-Z0-9.-]*) echo 'Invalid mail hostname or domain' >&2; exit 1 ;;
  esac
done
MAIL_DOMAIN=$(printf '%s' "$INBOUND_MAIL_DOMAIN" | tr 'A-Z' 'a-z')
MAIL_MAX_MESSAGE_BYTES="${MAIL_MAX_MESSAGE_BYTES:-10485760}"
case "$MAIL_MAX_MESSAGE_BYTES" in
  ''|*[!0-9]*) echo 'Invalid message size limit' >&2; exit 1 ;;
esac
MAIL_OPERATOR_ADDRESS="${MAIL_OPERATOR_ADDRESS:-}"
if [ -n "$MAIL_OPERATOR_ADDRESS" ]; then
  case "$MAIL_OPERATOR_ADDRESS" in
    *[!a-zA-Z0-9.@_+-]*|*@*@*|@*|*@|*..*) echo 'Invalid MAIL_OPERATOR_ADDRESS' >&2; exit 1 ;;
    *@*) ;;
    *) echo 'Invalid MAIL_OPERATOR_ADDRESS' >&2; exit 1 ;;
  esac
  if [ "$(printf '%s' "${MAIL_OPERATOR_ADDRESS#*@}" | tr 'A-Z' 'a-z')" = "$MAIL_DOMAIN" ]; then
    echo 'MAIL_OPERATOR_ADDRESS must not use the receiving domain' >&2
    exit 1
  fi
fi
test -r /run/secrets/mail_tls_certificate
test -r /run/secrets/mail_tls_key
postconf -e "myhostname=$MAIL_POSTFIX_HOSTNAME"
postconf -e "relay_domains=$MAIL_DOMAIN"
postconf -e "message_size_limit=$MAIL_MAX_MESSAGE_BYTES"
if [ -n "$MAIL_OPERATOR_ADDRESS" ]; then
  # Virtual aliases count as valid recipients and are forwarded over SMTP.
  postconf -e "virtual_alias_maps=inline:{ postmaster@$MAIL_DOMAIN=$MAIL_OPERATOR_ADDRESS, abuse@$MAIL_DOMAIN=$MAIL_OPERATOR_ADDRESS }"
  postconf -e "smtpd_recipient_restrictions=reject_unlisted_recipient"
else
  postconf -e "virtual_alias_maps="
  postconf -e "smtpd_recipient_restrictions=check_recipient_access inline:{ { postmaster@$MAIL_DOMAIN = 550 5.1.1 Mailbox unavailable }, { abuse@$MAIL_DOMAIN = 550 5.1.1 Mailbox unavailable } }, reject_unlisted_recipient"
fi
postfix set-permissions
postfix check
exec postfix start-fg
