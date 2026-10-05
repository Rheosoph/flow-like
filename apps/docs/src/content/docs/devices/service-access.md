---
title: Service access and port forwarding
description: Connect to deployed device services, distinguish private tunnels from network listeners, and configure browser access and service credentials.
---

A device management connection lets you operate the agent. A service connection
lets you use an app running on that device, such as a Page, chat, form, or HTTP
endpoint. Each path has its own permissions and credentials.

You can manage a device remotely without publishing its service ports. Direct
service access requires a reachable listener. On-demand forwarding carries
service traffic through an authenticated tunnel and can keep the listener
private. Neither access method changes where the workflow executes: the
deployed service still runs on the device.

## Choose the connection path

| Path | What it reaches | Required access |
| --- | --- | --- |
| Live device management | Status, logs, deployment operations, and supported on-demand actions | Account access, unlocked controller key, and permission for each operation |
| Direct service address | The device's hosted interface or API | A network route to the listener and a service access token if required |
| Encrypted service tunnel | A configured, running service listener on the device | A valid controller session, Connect to services permission in scope, and a service access token if required |

The tunnel transport supports private service connections. Studio's local
port-forwarding controls and the web app's tunneled service viewer are being
integrated. The **Endpoint → Open** procedure below describes direct access;
it requires network reachability and does not itself create a tunnel.

## Open a service on the device or private network

First deploy an event with a hosted interface or endpoint, and confirm that
the service is running. A background-only service may have no hosted address.

1. Open **Devices**, select the device and service, then open **Endpoint**.
2. Check the listener address, port, and TLS certificate. Configure these under
   the service's settings when a change is needed.
3. If the listener uses all interfaces, enter a hostname or IP address that
   the client can reach. `0.0.0.0` is a bind address, not a destination for a
   browser. Entering an address here does not change routing, DNS, or the
   listener's exposure.
4. Select **Open**, copy the link, or use the QR code on a device that can
   reach that network.
5. Enter the service access token if prompted. Select the available
   Page, chat, or action you want to use.

The bundled service page lives at:

```text
https://<reachable-device-host>:<service-port>/ui/
```

The scheme is `http` when the service has no TLS certificate. Use HTTPS for
direct access across a network so requests and service tokens are encrypted.
The encrypted management session does not protect a separate HTTP request.

A loopback address such as `127.0.0.1` or `::1` refers to the computer opening
the link. A direct loopback link works in a browser on the device itself. On
your laptop or phone, the same address refers to that laptop or phone.
Connecting live to the agent does not change this meaning.

For Docker deployments, both the service listener inside the container and
the host's published-port configuration must allow the intended connection.
The generated package publishes its service port on host loopback by default.
See [Set up a device](/devices/setup/#docker) before changing that mapping.

## Use the service access token

The deployment wizard generates a service token by default. To omit it,
choose **No token** under **Endpoint & limits**. Anyone who can reach the
listener can then use the service's endpoints, Pages, chats, and actions
without a token. Management and tunnel permissions still apply to their
respective connection paths. **Keep current** preserves this choice on updates.

Newly generated service tokens are shown once after deployment. Save the token
in an appropriate credential store and give it only to people or clients that
need to use that service. The deployment can also keep or accept an existing
token. If a token is lost or exposed, replace it on the service's **Endpoint**
tab. Existing clients must then use the new token.

One token covers the hosted endpoints, Pages, and chats in that service. Use
separate services when those interfaces need independent credentials.
Management sharing and service-token distribution are separate decisions.
Removing someone's management permission does not revoke a token they copied.

When a service requires a token, an API client sends it in the authorization
header. For a service with
a `POST /webhook` event, a request looks like this. Set `DEVICE_SERVICE_TOKEN`
in your shell from your credential store first:

```sh
curl --fail-with-body 'https://device.example:8443/webhook' \
  -H "Authorization: Bearer $DEVICE_SERVICE_TOKEN" \
  -H 'Content-Type: application/json' \
  --data '{"event":"test"}'
```

Use the actual hostname, certificate, port, path, and payload for your service.
Omit the `Authorization` header when the service is configured with **No token**.
The device route has no hub `/api/v1/sink/trigger/http/{app_id}` prefix. An
HTTP event's separate hub token is not the deployed service token. See
[HTTP events on a device](/dev/sinks/http/#on-a-device) for route rules,
request limits, and response codes.

## Run actions from Desktop or the web app

For supported quick actions and forms, the Devices workspace can start a run
over the management connection. Connect live, open the service's **Status**
tab, and select **Run now…** beside the desired action or form. This path
invokes the deployed event and its pinned Flow
version without requiring a direct browser route to the service listener.

The action remains subject to the device's permission checks and supported
input types. Forms that require file input direct you to the service page.
Run history and the service's actual result are the evidence that an action
completed; an accepted request alone is insufficient.

## How on-demand service forwarding works

A tunnel joins an authorized controller to the device's service transport.
The device opens the destination connection using its own deployed
configuration. The client identifies a service deployment; it does not supply
an arbitrary host and port to dial.

The current service target is the placement's built-in `hosting` listener.
That covers the listener serving the deployed web interface and hosted APIs.
It does not provide general forwarding to SSH, a database, another LAN host,
or a listener opened independently by a REST/MCP server Flow.

**Connect to services** (`service_connect`) must be granted for the device,
project, or placement involved. Status access alone does not permit a service
connection. The placement must be running its current configuration. If the
service requires a token, it still checks that token through the tunnel. The
tunnel does not inject a token or change the service's access settings.

### Studio and browser connections

A desktop forward needs a local listening port that connects a local program
to the remote service stream. Bind such a forward to loopback when it is
intended only for programs on your computer. The local port is distinct from
the device's service port and from a router's port-forwarding rule.

A browser consumes the stream through a web client integration. It cannot
create a general operating-system TCP listener for other programs. A WebRTC
data channel also does not turn a private device address into an ordinary
public URL. Browser service access needs the client to handle requests through
the tunnel explicitly.

Direct requests from an unrelated website face the service's browser-origin
restrictions. The hosted HTTP endpoints do not provide a general CORS policy
for cross-origin clients. Use the bundled service page at the service origin
or a supported tunnel client integration instead of assuming that a copied
URL works with any web application's `fetch` call.

The service page's allowed origins setting controls external content loaded
by that page. It does not grant other websites permission to call the API.

### Encryption and connection lifetime

Service traffic has a separate authenticated Noise session. The connection
tries a WebRTC data channel and can use an encrypted WebSocket relay when
needed. WebRTC itself may use a configured TURN relay. These paths do not
require publishing the service listener to the internet.

The agent rechecks authorization and the target's current configuration while
streams are open. Stopping or reconfiguring a service, losing authorization,
or breaking the transport can close its streams. Open a new connection after
recovering the underlying problem. Before retrying a request, check whether
the first attempt produced an external effect.

Credential and session-key renewal can preserve active streams, but the tunnel
is not a durable queue. A lost connection does not replay service requests.
For capacity limits and relay configuration, see
[Realtime signaling](/self-hosting/signaling/).

## Diagnose connection problems

| Observation | Check |
| --- | --- |
| Live management works, but a direct service link fails | The listener, host firewall, private-network route, and Docker port mapping. Management connectivity does not establish direct reachability. |
| A loopback link opens the wrong machine | Open it on the device itself or use an access path that creates a local forward on the client. |
| The service returns `401` | Check whether the running service requires a token and supply its current token, even when using an encrypted connection. |
| The browser rejects HTTPS | Check the certificate hostname, trust chain, and expiry. A management connection does not make the service's certificate trusted. |
| A tunnel is refused despite visible status | Check Connect to services permission, its scope and expiry, agent compatibility, and the running service configuration. |
| Requests fail after an update | Verify readiness and the current listener, reconnect, and check for a changed token or route. |
| A web app cannot call the direct endpoint | Check browser origin restrictions and whether the client uses the supported service-access path. |

Read [Device security and networking](/devices/security/) before widening a
listener's exposure to resolve a connectivity problem.
