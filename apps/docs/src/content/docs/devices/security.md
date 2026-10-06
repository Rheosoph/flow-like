---
title: Device security and networking
description: Understand device identity, encrypted remote access, service exposure, permissions, and the security boundaries of an edge deployment.
---

You can manage a Flow-Like device behind a company firewall or home router
without publishing a management port on that device. The agent connects outward
to the hub and signaling service. An authorized controller, such as Flow-Like
Desktop or the web app, then establishes an encrypted session with the agent.

This removes the need for a permanent public device-management endpoint and a
router port-forwarding rule. It also keeps access decisions tied to a device,
user, and permitted operation. The host, controller, and deployed workflows
remain part of the trust boundary.

For installation, start with [Set up a device](/devices/setup/). For listener
configuration and the service access available in your client, read
[Service access and port forwarding](/devices/service-access/).

## How a remote connection reaches the device

The hub handles account identity and issues short-lived signaling credentials.
The signaling service helps the controller find the device and exchange the
information needed to establish a connection. The peers try WebRTC and can fall
back to an authenticated WebSocket relay when a peer connection is unavailable.

WebRTC uses Interactive Connectivity Establishment (ICE) to find a usable
network path. STUN helps discover network mappings; a configured TURN server
can relay WebRTC traffic when a direct path is unavailable. These mechanisms
depend on the network allowing the required traffic. They cannot guarantee
connectivity through every firewall. See the [ICE specification, RFC
8445](https://www.rfc-editor.org/rfc/rfc8445.html).

WebRTC therefore describes the transport, not proof of a direct connection.
A WebRTC session can use either a direct path or TURN. Its selected ICE
candidate pair identifies which path was established.

Flow-Like's WebSocket relay is a separate fallback from TURN. A session that
uses a relay is still encrypted between the controller and device. Relay use
does introduce a dependency on the relay's availability, capacity, and latency.

### What the network needs

| Connection | Purpose | Network requirement |
| --- | --- | --- |
| Device and controller to the hub | Enrollment, account access, authorization, and credential renewal | Outbound HTTPS to the configured hub |
| Device and controller to signaling | Discovery, connection setup, and relay fallback | Outbound WSS with WebSocket upgrades allowed |
| WebRTC peer traffic and configured ICE servers | Establish a direct or relayed data channel | Firewall policy must allow the selected ICE paths; STUN and TURN destinations depend on hub configuration |
| Workflow to local equipment | Read sensors, call a local API, or operate an existing service | Reachability from the device to that equipment |
| Workflow to cloud providers | Hosted models, online data, or external integrations | Outbound access to each provider the workflow actually uses |

Use your configured endpoint list when writing firewall rules. There is no
single fixed device port that covers all possible WebRTC paths. Self-hosting
the hub, signaling service, or TURN infrastructure also requires making those
services reachable by their clients. The absence of a public device listener
does not remove that infrastructure requirement.

### Management access and service exposure are separate

A workflow can create its own listener. The service exposure setting determines
whether that listener accepts connections only on the device, on one selected
interface, or on all interfaces. Remote management does not make a listener
private automatically.

For a service used only through controlled remote access, keep its listener on
loopback where the access method supports it. For equipment on the local
network, bind a suitable private interface and restrict access with the host
firewall. A listener on `0.0.0.0` also listens on any public IPv4 interface the
host has; IPv6 exposure needs its own review. A home router's IPv4 NAT is not
proof that an IPv6 listener is unreachable.

Noise protects the controller-to-agent session. Direct calls to a service use
that service's own transport. A plain HTTP listener sends its payload and any
bearer token without TLS, even if management traffic is encrypted. Configure HTTPS
on the service, or terminate TLS at a controlled proxy, before using it across
a network. Keep any plaintext proxy-to-service hop within the intended host or
network boundary.

Public webhooks and certificate validation can require deliberate ingress.
For example, [Let's Encrypt HTTP-01 validation](https://letsencrypt.org/docs/challenge-types/#http-01-challenge)
requires public port 80 to reach the challenge responder, directly or through
a proxy. That is an additional service decision, independent of encrypted
device management.

## Device identity and encrypted sessions

Enrollment creates a relationship between the owner and a particular device.
The permanent private device keys are generated on the target; the hub receives
their public identity. The bootstrap package enables enrollment; treat it as a
credential until enrollment has completed or its authorization has expired.

The controller pins the device identity it first verifies. If that device
later presents different keys for the same enrollment, management is blocked.
Investigate an unexpected identity change before replacing a saved identity.

The controller also needs its management key. Signing in to the hub and
unlocking that key serve different purposes: the account provides hub access,
while the key proves authority to the device. Keep the controller recovery
material separate from the deployment package and from the target host.

Controller keys are stored in a password-encrypted vault. Browser storage can
be deleted or evicted, so retain an encrypted controller or account backup and
its password. Signing in again cannot reconstruct a lost private key.

WebRTC data channels use DTLS transport encryption, as required by the
[WebRTC security architecture, RFC
8827](https://www.rfc-editor.org/rfc/rfc8827.html). Flow-Like also establishes an
authenticated Noise session between controller and agent. Noise is the
cryptographic handshake and encrypted message layer used for management,
including when messages travel through the WebSocket relay. Service tunnels
use a separate Noise session from management traffic.

The relay can observe connection identities, timing, traffic volume, and
routing information. It cannot read the encrypted management or service
payloads. Encryption does not conceal all metadata, and a relay can still
interrupt or withhold traffic. Protect the hub and the web app that delivers
the controller code, as well as the device itself.

## Grant only the operations an operator needs

Device sharing uses owner-signed access policies. A grant binds a controller
key to capabilities, an expiry, and a scope: the whole device, a project, or a
particular service deployment. The device checks this policy when handling
management operations.

| Operator's task | Relevant access | Consequence |
| --- | --- | --- |
| Check health | Status, and Metrics if needed | Observing health need not include deployment authority |
| Investigate a failure | Logs within the affected scope | Logs can contain workflow inputs or business data |
| Restart an existing service | Restart within that service's scope | Restarting runs the deployed code again |
| Change deployed code or configuration | Deploy & configure | This is code-execution authority and includes sensitive service configuration |
| Open a service tunnel | Connect to services (`service_connect`) | Allows connections to configured listeners in scope; the service checks its token if one is required |
| Update the agent, reboot, or manage certificates | The corresponding device capabilities | These operations affect the host or other services, so reserve them for device operators |

Service tokens provide a second access boundary by default. Someone holding
the token and a network path to a service can call it without being a Flow-Like
management operator. Choosing **No token** during deployment removes that
boundary for every hosted endpoint, Page, chat, and action in the service.
Anyone who can reach its listener can use them. Management and tunnel
permissions still apply to their respective connection paths.

Removing a management grant does not rotate a copied service token. It also
does not stop services that the person previously deployed or revoke
cloud approvals they created. Review each of those separately when removing
an operator.

Access changes take effect when the device applies the new policy. An offline
device cannot immediately learn that someone has been removed. Expiry and
authorization leases bound access when fresh authority is unavailable; do not
promise instantaneous offline revocation. For an urgent incident, combine hub
revocation with local containment and rotation of affected service or provider
credentials.

## Local credentials and host isolation

The agent protects private state with ownership and file-permission checks.
Queued secret updates are encrypted, while installed service secrets are
private files available to the runtime. Device keys and secrets still rely on
the operating-system account and host storage boundary. This is not protection
against an administrator or an attacker who controls that account. Use a
dedicated account, protect backups, and apply disk encryption where appropriate.

The default `trusted_process` execution profile assumes trusted workloads on a
dedicated account. It provides no workload isolation. A project that can run
native code can act with that process's host permissions.

The optional `linux_sandbox` profile adds host-enforced controls when the target
provides the required Linux facilities, delegated resource controls, and disk
quotas. It is unavailable on macOS. A requested sandbox must pass its checks;
it does not silently fall back to the trusted profile. Device operators can
require isolation through the host policy described in
[Set up a device](/devices/setup/#configure-the-host).

The Linux sandbox shares the host network. It does not filter loopback or
link-local metadata addresses. Use host firewall rules for the workload boundary
and require authentication on local services. A service bound to loopback can
still be reached by other permitted processes on that host.

These device profiles differ from the gVisor execution boundary used by
self-hosted backend deployments. See [Security architecture](/reference/security/)
for the boundaries of those other deployment types.

## What data leaves an edge device

Local execution puts computation near the equipment. Data movement depends on
what the workflow and deployment are configured to do.

- A hosted model receives the inputs sent to it. Local model execution needs
  its model files and sufficient device resources.
- An online deployment can use cloud files and tables under its approved
  grants. Offline caching and queued writes have explicit limits and expiry.
- Retained logs and metrics have their own encrypted sharing and recipient
  controls. Review who can decrypt history separately from who can operate the
  live service.
- A service page can load permitted external content. Review its allowed
  origins and any external resources used by the UI.

Minimize sensitive log content before enabling retention. Revocation cannot
erase plaintext that an authorized reader already downloaded. Document the
actual path from input to storage, model, logs, and output for each deployment.

## Verify the boundary before rollout

Test the intended operator experience from a different network. Confirm that
the device connects without a router forwarding rule. Check whether the
session uses WebRTC or WebSocket fallback; inspect the selected ICE path if
you also need to distinguish direct WebRTC from TURN. Separately check that
private service ports are unreachable from untrusted networks.

Use a restricted operator account to test an allowed status read and a denied
deployment change. Check that protected service requests without the required
credentials fail. Revoke a test grant while connected, then test the behavior
during a controlled outage. Record the validity periods and recovery steps
that apply to your installation.

For implementation details, see the [management protocol](https://github.com/Rheosoph/flow-like/blob/dev/packages/device-protocol/src/management.rs),
[agent management](https://github.com/Rheosoph/flow-like/blob/dev/apps/standalone/src/management.rs),
[secret storage](https://github.com/Rheosoph/flow-like/blob/dev/apps/standalone/src/secrets.rs),
and [device isolation](https://github.com/Rheosoph/flow-like/blob/dev/apps/standalone/src/isolation.rs).
