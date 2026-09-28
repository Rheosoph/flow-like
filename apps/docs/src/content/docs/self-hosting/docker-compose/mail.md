---
title: Inbound email
description: Receive email for inbound email events with Postfix and the mail ingress gateway
sidebar:
  order: 24.5
---

The optional `docker-compose.mail.yml` overlay adds two services. `mail-postfix`
is the public MX for one receiving domain. It checks each recipient with the
`mail-ingress` gateway and hands accepted messages to it over LMTP. The gateway
stores every message through the API, which runs the matching inbound email
event on the server.

## Prepare

- Choose a dedicated receiving domain, such as `events.example.com`. Publish
  an MX record for it that points to the public MX hostname, and allow inbound
  TCP port 25 to the host.
- Obtain a TLS certificate chain and key that cover the MX hostname. Postfix
  offers STARTTLS with them.
- As a platform administrator, register a service token with
  `POST /api/v1/admin/sinks` and the body below, then save the returned token
  to a file readable only by the deployment user. Revoking the registration
  stops ingestion immediately.

  ```json
  { "sink_type": "inbound_email", "name": "Postfix mail ingress" }
  ```

## Configure

Add the overlay to `COMPOSE_FILE` and set its variables in `.env`:

```dotenv
COMPOSE_FILE=docker-compose.yml:docker-compose.mail.yml
INBOUND_MAIL_DOMAIN=events.example.com
MAIL_POSTFIX_HOSTNAME=mx.example.com
MAIL_SINK_JWT_FILE=/etc/flow-like/mail-sink.jwt
MAIL_TLS_CERTIFICATE_FILE=/etc/flow-like/mx-fullchain.pem
MAIL_TLS_KEY_FILE=/etc/flow-like/mx-key.pem
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `INBOUND_MAIL_DOMAIN` | required | Receiving domain. Also enables inbound email on the API |
| `MAIL_POSTFIX_HOSTNAME` | required | Public MX hostname, covered by the certificate |
| `MAIL_SINK_JWT_FILE` | required | File containing the registered `inbound_email` token |
| `MAIL_TLS_CERTIFICATE_FILE`, `MAIL_TLS_KEY_FILE` | required | SMTP certificate chain and private key |
| `MAIL_BIND_ADDRESS` | `0.0.0.0` | Address on which port 25 is published |
| `MAIL_MAX_MESSAGE_BYTES` | `10485760` | Largest accepted message, at most 40 MiB |
| `MAIL_OPERATOR_ADDRESS` | empty | Mailbox that receives `postmaster@` and `abuse@` mail |
| `MAIL_AUTOMATION_ENABLED` | `false` | The API's `sending_enabled` setting |

The overlay builds its images locally. Build them, then start as usual:

```bash
docker compose build mail-ingress mail-postfix
python3 scripts/up.py
```

Create an active inbound email event with server execution, send a test
message to its address and check `docker compose logs mail-postfix mail-ingress`.

## Sending

The overlay starts the API receive-only. `MAIL_AUTOMATION_ENABLED` decides
`sending_enabled` and overrides the value in the runtime configuration.
Before setting it to `true`, configure an outbound provider: the transactional
`mail` section of the hub configuration with SMTP, SendGrid or SES, or an SMTP
`outbound` block in the private `mail_automation` section of the runtime
configuration. Azure Communication Services cannot send from event
addresses. Without a provider the API logs the problem and keeps sending
disabled.

## Network boundary

Postfix relays mail only to the receiving domain; it is not an open relay.
The gateway's LMTP and recipient lookup listeners are on two internal
networks: `mail`, shared with Postfix, and `mail-api`, shared with the API
gateway it calls. Runtimes and other services cannot reach them, and the
gateway has no internet access.

The gateway caches recipient lookups for 30 seconds when an address exists
and for 10 seconds when it does not. A message sent to a new address within
10 seconds of a rejected attempt is rejected as well. Every stored message is
checked against the API again.

## Limits

Postfix accepts at most 100 recipients per message and 20 concurrent
connections per client. Within each minute, one client may open 60
connections, send 60 messages and name 200 recipients. Clients over a limit
receive a temporary failure and retry later. Limits apply per client address
as Postfix sees it: when a proxy forwards the connections, its clients share
one budget. To change them, edit
`apps/backend/mail-ingress/postfix/main.cf` and rebuild the image.

## Role mailboxes

Mail servers are expected to accept `postmaster@` their domain. Set
`MAIL_OPERATOR_ADDRESS` to a mailbox on another domain to forward
`postmaster@` and `abuse@` of the receiving domain to it. Postfix sends these
over outbound SMTP, so the host needs outbound TCP port 25, which many cloud
providers block by default. Forwarded mail can fail the operator provider's
SPF checks. Without the variable, Postfix rejects both addresses with
`550 5.1.1`.

## Sender authentication and filtering

This path forwards no SPF, DKIM, DMARC, spam or virus verdicts. The event's
authentication metadata stays empty, so checks based on those verdicts do not
apply: a reply is not refused because the original message failed DMARC or a
spam or virus scan. Treat the sender and content as unverified in flows that
act on them.

To filter mail before it reaches Flow-Like, add a milter such as
[rspamd](https://rspamd.com/doc/integration.html) in a derived Postfix image:
set `smtpd_milters = inet:rspamd:11332` and `milter_default_action = tempfail`,
and let rspamd reject spam and, with ClamAV, viruses instead of only adding
headers. Its container needs to share a network with Postfix and needs
internet DNS for SPF, DKIM and DMARC lookups. The gateway passes no rspamd
result to the flow. It also requires the Postfix `Received` header to remain
the first header, so the milter must not insert headers above it.
