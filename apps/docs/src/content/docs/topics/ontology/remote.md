---
title: Shared & Remote Ontologies
description: Expose an ontology to connected projects, and discover, install, and use ontologies published by other projects.
sidebar:
  order: 2
---

A **remote ontology** lets one project publish its semantic layer as a **contract** that other, connected projects can install and build on — without ever seeing the underlying data or the boards that implement its actions. This is how a shared, organization-wide ontology works: a producer owns the definition, and consumers subscribe to it.

Everything here is in **Data Studio → Sharing**.

## The mental model

- A **producer** exposes an ontology. It stays the source of truth.
- A **consumer** installs a **sanitized copy** of the contract. It gets the object types and the *semantics* of actions — but not the producer's private implementation (board IDs, versions, start nodes are stripped).
- When a consumer invokes a remote action, execution happens **in the producer's project**, which resolves those opaque identifiers itself. The consumer never runs the producer's board directly.

:::note[Only exposed contracts are shared]
The discovery endpoint returns *only* ontologies the producer has explicitly exposed. Exposure can be turned off at any time, which stops new discovery and installs immediately.
:::

## Prerequisites: connect the projects

Sharing rides on **app connections**. Before anything appears in the Sharing tab:

1. Create an active connection from the consuming app to the producing app under **Team → Connections**.
2. The connection's **role must grant Read Files or Read Database** on the producer. Without it, the producer's contracts stay invisible even if exposed.

If there are no active connections, the Sharing tab shows *"No active app connections. Create one from Team → Connections."*

## Producer side: expose an ontology

On each of your ontologies in the Sharing tab there are two switches:

| Switch | Effect |
|--------|--------|
| **Expose to connected projects** | Makes this contract discoverable by permitted connected projects. |
| **Generate board bindings** | Adds this ontology's object and action bindings to *this* project's own node catalog. |

Turning **Expose** on is all a producer has to do. Consumers with a qualifying connection can then discover and install it.

## Consumer side: discover and install

The **Available remote ontologies** panel lists your active connections. For each one:

1. **Discover** (or **Refresh**) — calls the connected project and lists the ontology contracts it exposes. Each shows its name and object-type count.
2. **Install** — installs the sanitized contract into your project. Its status badge flips to **Installed**, and the contract's object bindings are added to your node catalog.

An installed contract is labelled **"bindings only"** — a reminder that you receive object semantics and action *definitions*, never the producer's boards.

### Keeping in sync

Each installed contract remembers the producer version it was captured from. When the producer changes their ontology, the badge shows **Update available**. Click **Refresh** to pull the latest sanitized contract and regenerate your bindings.

### Removing one

**Uninstall** removes the contract and its generated bindings from your project. Uninstall works even if the connection is no longer active, so you can always clean up a stale import.

## Using a remote ontology in boards

Once installed with bindings enabled, the remote ontology contributes nodes to your catalog:

- **Query Remote Ontology Objects** — read objects of a remote object type. This runs against the producer through your connection, honoring the producer's exposure and connection role.
- **Query Remote Ontology Children** — expand a parent object's [containment children](/topics/ontology/overview/#hierarchy--composition) within the installed contract. Like the objects node, it runs against the producer and honors exposure.
- **Invoke Remote Ontology Action** — invoke the producer's governed actions by object reference. The producer executes the pinned board version and enforces its own parameter schema and permissions.

:::note[Remote hierarchies resolve within the exposed contract]
A producer's containment links keep their hierarchy flag when shared, so you can drill down through an installed contract. But a link that pointed at *another* of the producer's ontologies has its target stripped during sanitization — a remote subtree only ever resolves within the exposed contract, never into a producer overlay you weren't given.
:::

## Governance summary

| Guarantee | How |
|-----------|-----|
| Consumers see only what producers allow | Discovery returns exposed contracts only; the connection role must permit reading data |
| No implementation leaks | `board_id`, `board_version`, `start_node_id`, and event IDs are stripped from shared action contracts |
| No cross-ontology leaks | Containment link targets (`dst_ontology`, `dst_binding_id`) are stripped, so a remote subtree can't resolve into a producer overlay that wasn't exposed |
| Actions can't be widened by editing metadata | Each action pins an immutable board version and a contract hash that's re-checked at invoke time |
| Revocation is immediate | Turning off **Expose** stops new discovery/installs; existing consumers lose access on their next remote call |
| Consumers can always clean up | Uninstall never requires the connection to still be active |

## Related

- [Ontology & Knowledge Graph](/topics/ontology/overview/) — object types, link types, the graph explorer, and actions.
