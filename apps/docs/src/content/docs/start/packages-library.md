---
title: Packages
description: Build and publish your node packages in Mine, manage the ones you use in Library, and edit each package in its workspace
sidebar:
  order: 29
---

**Packages** is where you work with WASM node packages. It appears in the
sidebar when [Developer Mode](/start/developer-mode/) is on and has two tabs:

| Tab | What it lists |
| --- | --- |
| **Mine** | Packages you build: the ones you own or maintain in the registry, plus package folders on this computer (desktop) |
| **Library** | Packages you use: installed on this computer (desktop), bought, or shared with you |

**Explore packages** in the header opens [Explore](/start/explore/) filtered to
packages, where you discover new ones.

Packages are managed in three related scopes:

| Scope | Where to manage it | Purpose |
| --- | --- | --- |
| Registry | **Explore** and a package's store page | Discover packages and inspect their listed capabilities |
| Device | **Packages › Library** | Manage the packages installed on this computer |
| App | Open an App → **Packages** | Declare the package versions that belong to that App |

This separation lets a device cache several packages while each App keeps an
explicit dependency set.

## Mine

Every package appears once, whether it lives in a folder on this computer, in
the registry, or both. Folders are matched to registry packages by the id in
their `flow-like.toml`. Each card shows the package's state and one action for
what to do next:

| State | Meaning | Typical action |
| --- | --- | --- |
| **Local only** | The folder has never been published | **Publish…** |
| **Unpublished changes** | The folder's version is ahead of the live one | **Publish** the new version |
| **In review** | A version waits for publication review | **View review** |
| **Live** | The published version matches the folder | **Open** |
| **Not on this machine** | You maintain the package, but there is no folder here | **Link folder…** |
| **Disabled** | The package is hidden from the store | **Restore…** in Releases |

Problems take priority over the state. Lint errors show **Fix N errors**, a
changed `.wasm` shows **Reload into catalog**, and an id someone else already
owns shows **Rename id**. An id that still starts with `com.example.`, as every
template's does, shows **Choose an id**, because it has to change before you
publish.

Use the filter chips, search and **Needs attention** or **Name** sorting to
narrow the list. **Show disabled** also lists disabled packages that have no
folder here. Click a card to open its workspace. The **⋯** menu opens the
folder in your editor or file manager, jumps to **Access & people**,
**Releases** or the store page, and **Remove from list** forgets the folder
without deleting any files.

In Flow-Like Desktop, **New package** starts a project from a language
template and **Add folder** adds an existing package project. The **Developer
Settings** button next to them sets your preferred editor and links to
developer tools such as the workflow benchmarks.

On the web, **Mine** lists the packages you own or maintain in the registry.
Package folders are built and managed in the desktop app.

## Library

**Library** lists what you can use. On the web that is the packages you bought
and the ones shared with you. Flow-Like Desktop also lists every package
installed on this computer, including free ones, with its state:

- **Ready** — installed and in your node catalog;
- **Update available** — a newer version is published;
- **Not on this machine** — you have access, but it is not installed here;
- **Problem** — the installed copy failed to compile.

Filter by state or by access (**Bought**, **Shared with you**, **Owner**,
**Maintainer**, **Free**, **Local**), and search by name or id. On desktop,
**Check for updates** asks the registry again, and an update that requests new
permissions asks you to allow them first. **Load local .wasm** loads a package
file for a quick test without a project.

The **⋯** menu on a card offers **Install on this machine**, **View store
page** and **Uninstall**. Uninstalling a package you bought, own, maintain or
that was shared with you keeps your access, so you can reinstall it anytime.
For packages with widgets, **Clear widget permissions on this device** revokes
every approval you gave its widgets.

:::note[Installed version and App version]
Updating the device copy does not automatically rewrite every App's dependency
declaration. Open the App's **Packages** screen to review its linked version and
automatic-update setting.
:::

## The package workspace

Opening a package from **Mine**, or **Manage package** on its store page, opens
its workspace. The header shows the package's state, the live version next to
the version on disk, **View store page**, and the next action. The tabs are:

| Tab | What you do there |
| --- | --- |
| **Overview** | Live version, installs, rating, the people with access, listing health and permissions. On desktop it also shows the folder, when it was built, what changed since the live version, and build and lint checks. |
| **Nodes** | The package's nodes. On desktop, with lint results for the local build. |
| **Test** | Run nodes with your own inputs and try the package's widgets (desktop). |
| **Manifest** | Edit `flow-like.toml` (desktop). |
| **Listing** | The store listing: name, descriptions, icon, thumbnail, keywords and links, the price (owners), and a preview of the store card and page. A private package can request publication review here. |
| **Access** | Access requests and the people who can use or maintain the package. |
| **Releases** | Every published version, **Install for testing**, the publication review history, and the next release. |

The web shows the registry tabs only: Overview, Nodes, Listing, Access and
Releases. On desktop, **Test** and **Manifest** need the package's folder;
without one they offer **Link folder…**. A folder that was never published
opens with the local tabs only.

Only owners and maintainers can open the workspace of a registry package.
Anyone else who follows a workspace link lands on the package's store page.

## Develop a package locally

1. In **Packages › Mine**, select **New package** and pick a language
   template, or **Add folder** for an existing project.
2. Replace the template's `com.example.` id in the **Manifest** tab.
3. Build the project, then **Reload into catalog** to compile and test its
   nodes locally. Reload again whenever the `.wasm` output changes.
4. Use the **Test** and **Nodes** tabs to run nodes and fix lint errors before
   publishing.

A package loaded from its folder is a local development source. It is distinct
from a published registry version and can be iterated without submitting each
build.

## Publish

Select **Publish…** on the card in **Mine**, or the publish action in the
workspace header:

1. Confirm the package id and version. If they differ from `flow-like.toml`,
   publishing saves them there.
2. Review the manifest metadata.
3. Review the resource tiers and allowed hosts.
4. Build and publish the release artifact.

The publisher checks package-id and version availability, locates the release
WASM artifact, uploads it, and creates the registry version. A new package is
published privately first. When publishing finishes, the workspace opens on
**Listing** and points out what the store listing still misses, such as a
thumbnail, a description or keywords. Request publication review from
**Listing** when the package is ready.

The manifest (`flow-like.toml`) authors only what a node cannot state in code:

- memory and execution-time tiers;
- `allowed_hosts`, a package-wide outbound host allowlist;
- OAuth scopes, each with provider, scopes, reason, and whether it is required.

Capabilities are not authored in the manifest. Each node declares its own
permissions in code: HTTP, WebSocket, TCP, UDP, and DNS access, storage read
and write, database read and write, variables, cache, streaming, models, A2UI,
OAuth, and functions. The sandbox grants each node exactly what it declares.
The registry derives the capability listing shown in the store from the
compiled node definitions and replaces capability flags authored in a
manifest. When a developer project is loaded, Desktop derives the same listing
and logs a warning for every authored flag that no node backs.

Declare only the permissions each node actually uses. Package consumers see
the tiers, allowed hosts, OAuth scopes, and derived capabilities before
installation. See [Package Manifest](/dev/wasm-nodes/manifest/) for where
`allowed_hosts` is enforced.

## Publication states

A package or version can be private, pending review, active, deprecated,
rejected, disabled, or yanked depending on ownership and review state. The
workspace's **Releases** tab is the source of truth for the current state and
holds the publication-review history. A disabled package is restored there.

Do not promise a review date to users. Respond to the review record, publish a
new version when code changes are required, and keep previous versions
available only when they remain safe to use.

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
