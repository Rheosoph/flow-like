---
title: Package Library
description: Manage installed packages and local package projects
sidebar:
  order: 29
---

Flow-Like Desktop separates packages into three related scopes:

| Scope | Where to manage it | Purpose |
| --- | --- | --- |
| Registry | **Explore → Packages** | Discover package versions and inspect their declared capabilities |
| Device | **Library → Packages** | Manage packages installed on this computer |
| App | Open an App → **Packages** | Declare the package versions that belong to that App |

This separation lets a device cache several packages while each App keeps an
explicit dependency set.

## Manage installed packages

Open **Library → Packages**. The **Installed Packages** page shows each
package's:

- name, description, keywords, and installed version;
- local compile status where available;
- newer registry version, when one is available;
- **Details**, **Update**, and **Uninstall** actions.

Search filters the installed collection by name, description, or keyword.
Select **Update All** when you have reviewed the listed updates and want to
apply them to the device.

:::note[Installed version and App version]
Updating the device copy does not automatically rewrite every App's dependency
declaration. Open the App's **Packages** screen to review its linked version and
automatic-update setting.
:::

## Link a package to an App

1. Open the App.
2. Select **Packages** in its navigation.
3. Select **Add Package**.
4. Choose a package and version.
5. For an online App, decide whether that dependency should update
   automatically.

The App screen lists nodes contributed by each linked package. Removing a
package from the App removes that dependency and its nodes from the App
catalog; it does not necessarily uninstall the package from the device.

## Develop a package locally

Use the Desktop **Developer** workspace for source projects:

1. Create a project from a supported language template or add an existing
   package project.
2. Build the project and inspect its `flow-like.toml` manifest.
3. Select **Load into Catalog** to compile and test its nodes locally.
4. Reload it after the WASM output changes.
5. Use **Debug & Test** before publishing.

The developer-loaded package is a local development source. It is distinct
from a published registry version and can be iterated without submitting each
build.

## Publish from Developer

Publishing starts from a package project in **Developer**, not from the
Installed Packages list:

1. Open the project's **Publish to Registry** action.
2. Confirm the package ID and version.
3. Review the manifest metadata.
4. Review resource tiers and permissions.
5. Build and publish the release artifact.

The publisher checks package-ID and version availability, locates the release
WASM artifact, uploads it, and creates the registry version. A new package is
published privately first. Manage its metadata and request publication review
from its registry detail page when it is ready.

Declared permissions currently cover:

- memory and execution-time tiers;
- HTTP, WebSocket, TCP, UDP, and DNS access, including allowed HTTP hosts;
- node-scoped and user-scoped storage;
- OAuth scopes;
- variables, cache, streaming, A2UI, and model capabilities.

Request only the capabilities the nodes actually use. Package consumers see
this declaration before installation.

## Publication states

A package or version can be private, pending review, active, deprecated,
rejected, disabled, or yanked depending on ownership and review state. The
registry detail page is the source of truth for the current state and includes
publication-review history for maintainers.

Do not promise a review date to users. Respond to the review record, publish a
new version when code changes are required, and keep previous versions
available only when they remain safe to use.

## Before uninstalling

Check the Apps that link the package. Uninstalling its device copy can make
those nodes unavailable for local editing or execution until the required
version is installed again. The App dependency remains until you remove it
from that App.

## Next Steps

- [Package Store](/start/packages-store/) — browse and install packages
- [Creating WASM Nodes](/dev/wasm-nodes/overview/) — build custom nodes
- [Registry and Governance](/dev/wasm-nodes/registry/) — publication and
  review
