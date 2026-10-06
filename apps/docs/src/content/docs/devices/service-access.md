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
| Encrypted service tunnel | The service's hosted listener and the additional loopback listeners configured for it | A valid controller session, Connect to services permission in scope, and a service access token if required |
| Model gateway | The models the device hosts | A valid controller session and Use models permission for the whole device |

The **Endpoint → Open** procedure below describes direct access. It requires
network reachability and does not create a tunnel. To use a service through
the encrypted tunnel instead, use **Connect through Studio** on the same tab;
see [Connect through the encrypted tunnel](#connect-through-the-encrypted-tunnel).

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

A service that consists only of forms and quick actions has no hosted
interface by default. In the deployment wizard's **Endpoint & limits** step,
**Open these forms and quick actions as a deployed app** creates a listener
for it with that step's access settings. Studio then opens the service
through the encrypted tunnel as described below. **Run now…** works with or
without that listener.

## Connect through the encrypted tunnel

A tunnel joins an authorized controller to the device's service transport.
The device opens the destination connection using its own deployed
configuration. The client names a service and one of its listeners; it does
not supply an arbitrary host and port to dial.

A service offers these tunnel targets:

- its hosted listener, `hosting`, which serves the deployed web interface and
  hosted APIs;
- up to 16 additional listeners named in the service's settings. Each is a
  TCP, HTTP, or HTTPS listener on the device's loopback interface, such as a
  database or a REST or MCP server that a Flow opens itself.

The tunnel does not reach other hosts on the device's network.

**Connect to services** (`service_connect`) must be granted for the device,
project, or placement involved. Status access alone does not permit a service
connection. The placement must be running its current configuration. If the
service requires a token, it still checks that token through the tunnel. The
tunnel does not inject a token or change the service's access settings.

The models a device hosts are a separate tunnel target that needs **Use
models** for the whole device. Connect to services does not include model
access, and Use models does not include service connections. See
[Host models on a device](/devices/models/).

### Use a service from Studio

Unlock the device, connect live, select the service, and open **Endpoint**.
**Connect through Studio** lists the service's listeners under **Service
listener**:

- **Open deployed app** opens the hosted interface's Pages, chat, forms, and
  quick actions inside Studio, through the tunnel. Enter the service access
  token there if the service requires one; it stays in that session. Closing
  the app clears the session's conversations and Page data.
- **Send request** sends one HTTP request to an HTTP or HTTPS listener, with
  a method, path, headers, and body. Include the service's access token as a
  header if the service requires one. The response streams in; Studio shows
  its most recent 1,048,576 characters.
- **Open local port**, in the desktop app only, opens a port on this
  computer's loopback address that leads to the listener. Enter `0` for any
  free port, then copy the address. Programs on this computer, such as a
  browser or a database client, can connect to it while the service page stays
  open. **Close local port** ends it. Before opening the port, Studio checks
  that you may connect and that the listener answers.

A TCP listener has no request form. Connect a program to it through a local
port from the desktop app.

Locking the device, leaving the service page, or losing the connection ends
open requests and local ports. Headers and tokens entered on the page are
kept only until then.

### Add service listeners

Under **Additional service listeners**, select **Add listener**, then enter a
**Service ID**, the **Protocol**, the **Device loopback address**, and the
**Device port**. For HTTPS, also enter the **TLS server name** and the server
certificate's **Certificate SHA-256 fingerprint**. Saving changes the
service's configuration; a running service applies it with health checks or
restarts, as you choose. Changing listeners needs Deploy & configure
permission for the service. Saving does not start the program behind a
listener. That program must already listen on the address.

Everyone with Connect to services in the service's scope can reach these
listeners. Require authentication in each program behind them.

For a request to an HTTPS listener, or to a hosted listener with a
certificate, the device opens the TLS connection itself. It checks the
certificate's SHA-256 fingerprint and server name before any request passes.
Update the fingerprint when that certificate changes. A local port passes
bytes through unchanged, so a program connecting to an HTTPS listener this
way speaks TLS with the listener itself.

### Browser connections

A browser cannot create a general operating-system TCP listener for other
programs. A WebRTC data channel also does not turn a private device address
into an ordinary public URL. The web app therefore offers the deployed app
and single requests through the tunnel; local ports need the desktop app.

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
| A loopback link opens the wrong machine | Open it on the device itself, or open a local port from the desktop app and use that address. |
| The service returns `401` | Check whether the running service requires a token and supply its current token, even when using an encrypted connection. |
| The browser rejects HTTPS | Check the certificate hostname, trust chain, and expiry. A management connection does not make the service's certificate trusted. |
| A tunnel is refused despite visible status | Check Connect to services permission, its scope and expiry, agent compatibility, and the running service configuration. |
| Requests fail after an update | Verify readiness and the current listener, reconnect, and check for a changed token or route. |
| A web app cannot call the direct endpoint | Check browser origin restrictions and whether the client uses the supported service-access path. |
| Requests to an HTTPS listener fail after its certificate was renewed | Update its **Certificate SHA-256 fingerprint** under **Additional service listeners**. |
| A listener shows no request form | It is a TCP listener. Open a local port from the desktop app. |
| Model requests are refused although service connections work | The model gateway needs Use models permission for the whole device and an agent that hosts models. |

Read [Device security and networking](/devices/security/) before widening a
listener's exposure to resolve a connectivity problem.
