---
title: Offline vs. Online
description: Choose where an app is stored, shared, and executed
sidebar:
  order: 10
---

Flow-Like Desktop can create **offline** and **online** apps. This choice
primarily controls persistence and collaboration; each Flow and Event also has
execution settings that control where a run happens.

## Storage and collaboration

| Capability | Offline app | Online app |
| --- | --- | --- |
| Primary app data | Local device | Configured Flow-Like backend |
| Works without signing in | Yes | No |
| Available in the web app | No | Yes |
| Multi-device access | Through explicit export/import | Through the online account |
| Team roles and invitations | No | Yes |
| Publication workflow | No | Yes |

Offline is a good default for personal experiments, local automation, and
work that must stay on one machine. Online apps are the right choice when the
app needs web access, collaboration, server-side events, or publication.

## Create an online copy later

An offline app is not mutated in place. From its configuration, select
**Create an online copy** to upload a new, secret-stripped copy to your
account. The local source app remains unchanged.

The copy process removes secret variable defaults and known token fields
before upload. Runtime values stored on the device are separate from the app
bundle and are not used to configure the online copy. Review the new app's
Flows, storage, credentials, roles, and event settings before relying on it.

Online apps can likewise be forked to a local or another online destination
when the source app and your permissions allow it.

## Where a Flow runs

The client and the Flow's execution mode work together:

| Flow mode | Flow-Like Desktop | Web app or server-only caller |
| --- | --- | --- |
| **Hybrid** | Normally local | Remote |
| **Local** | Local | Not remotely executable |
| **Remote** | Remote | Remote |

An offline app runs on Desktop and cannot be invoked by the web backend. An
online app can still run locally from Desktop when the Flow and required nodes
allow it.

**Hybrid** means the same Flow may run locally in Desktop or remotely through
the web/API path. It does not divide a single graph into local and remote
sections.

## Event execution

Events have their own **Local** or **Remote** execution setting and reference
either the latest Flow draft or a pinned Flow version. A remote event is useful
for webhooks, schedules, and other triggers that must work while a user's
Desktop app is not running.

The selected Flow must be compatible with that location. Pre-run analysis can
require local execution when a node needs capabilities unavailable on the
server.

## Permissions and local execution

Local execution requires enough read access to obtain the Flow definition.
A role that can invoke an app but cannot read its Flows uses server-side
execution instead. Configure exact capabilities on the app's
[Roles](/apps/share/#rights-and-roles) page.

## Local-only capabilities

Some nodes depend on the current device—for example desktop input, a local
path, locally installed software, or attached hardware. When a Flow requires
those capabilities, run it from Desktop and keep the app available on that
machine.

A local run can still call remote APIs and cloud services. “Local” identifies
the execution host, not a network-disconnected sandbox.

See [Local-only execution](/studio/local-execution/) and
[Runtime Variables](/apps/runtime-variables/) for device-specific
configuration.

## Self-hosted online execution

With a [self-hosted backend](/self-hosting/overview/), remote runs stay on the
infrastructure and execution backend you configure. They may use a shared
runtime pool, a per-run Kubernetes Job, Lambda, or another supported
[execution backend](/self-hosting/execution-backends/); self-hosting does not
imply one dedicated container per run.

Keep Desktop, web, API, and executor versions compatible when the same online
app is used across them.
