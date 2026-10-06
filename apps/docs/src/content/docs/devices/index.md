---
title: Device deployment
description: Run Flow-Like apps and services on your own computers, edge gateways, and home servers.
sidebar:
  order: 1
---

Deploy a Flow-Like app to a computer that can reach the equipment, files, or
local network it needs. The device runs its own agent, so deployed services keep
running after you close Studio. You can manage a company gateway, a machine in
a branch office, or a home automation server from the same Devices area.

The agent is a persistent runtime for selected app events. It can host Pages,
chats, and HTTP endpoints, keep a background workflow running, execute
schedules, and run supported bots. Your existing Flow-Like hub handles account
access and connection setup; the work executes on the device.

## What you deploy

An **app** contains the Flows, events, and supporting assets you author. An
**event** is an entry point into a Flow. A **service** is the installed unit on
one device: selected events, pinned versions, runtime settings, and any cloud
access they need. The agent's CLI and status output call a service a
**placement**.

You can deploy a whole app, select several events, or deploy one event on its
own. Putting events in one service makes them share configuration and service
access. Separate services let you give them different secrets, permissions,
limits, and update schedules. Each target device gets its own service even
when you deploy to several devices together.

## Where the data lives

| App type | What the device receives | Data during operation |
| --- | --- | --- |
| Local-only app | An offline copy prepared in Desktop, including pinned definitions and required assets | Each service initializes its own local data from the copy. Later app updates preserve that data. |
| Online app | Approved definitions pinned to exact event and Flow versions | Project data stays in cloud storage, accessed through a service-specific cloud approval. The device caches reads and can buffer explicitly selected writes during outages. |

An offline copy can run without the hub when all its dependencies are local.
Remote management still needs connectivity, and a Flow that calls an external
API or hosted model still needs that service. An online deployment does not
become an independent offline copy when the connection drops. Its cached reads,
write queue, and authorization have defined limits.

See [Deploy and operate services](/devices/deployments/) for the data model,
versioning, outage behavior, and update procedure.

## Reach a device without a public service port

Device management starts with outbound connections. Flow-Like uses a direct
WebRTC connection where available and an encrypted relay path when needed.
You do not need a router port-forwarding rule for this management path.

Services can remain bound to the device's loopback interface. Read
[Service access and port forwarding](/devices/service-access/) for Studio
forwarding and browser access paths. Publishing a listener to the LAN or
internet is a separate deployment choice.

Read [Device security](/devices/security/) for encryption, access rules,
host isolation, and their limits.

## Choose a starting point

- [Set up a device](/devices/setup/): create a package, enroll the agent, and
  keep it running on Linux or macOS.
- [Deploy and operate services](/devices/deployments/): select events,
  configure secrets, approve cloud access, and roll out changes.
- [Device use cases](/devices/use-cases/): plan an enterprise edge layer or
  a home automation installation.

Device deployment extends an existing hub. To operate the hub itself, use
[Self Hosting](/self-hosting/overview/).
