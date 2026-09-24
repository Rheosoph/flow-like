---
title: Package Store
description: Discover and install WASM node packages
sidebar:
  order: 28
---

WASM packages extend Flow-Like with additional workflow nodes. The package
registry is part of the **Explore** hub alongside community Apps.

## Open the registry

Open **Explore**, then select **Packages** in the header. The same destination
is available from **Library → Packages → Browse Packages** in Flow-Like
Desktop.

## Browsing Packages

Package cards show the current version, category, install count, rating, price,
and visibility. A shield marks a package that has completed the registry's
verification process.

Use the search field to match package metadata, then sort by:

- **Most Downloads**
- **Relevance**
- **Name**
- **Recently Updated**
- **Newest**

Select **Verified** to limit results to packages carrying the registry's
verified status. Without a search query, packages are grouped into category
swimlanes; search results use a regular results grid.

## Inspect before installing

Open a package to review:

- **Overview** — description, README, author, links, usage, and publication
  information supplied by the maintainer.
- **Nodes** — the nodes exported by the package.
- **Permissions** — resource tiers, allowed hosts, and OAuth scopes from the
  manifest, and the capabilities derived from the package's nodes.
- **Versions** — available, installed, yanked, disabled, or review versions.
- **Reviews** — user reviews and ratings.

Capabilities can include network access, storage, database access, runtime
variables, cache, streaming, A2UI, or model access. Authors do not enter them
in the manifest: each node declares its permissions in code, the sandbox
enforces them per node, and the registry derives this listing from the compiled
node definitions. The manifest adds the memory and timeout tiers, a
package-wide allowed-host list, and OAuth scopes with their reasons. A listed
capability does not mean the package should receive it blindly—compare the
listing with what the nodes are supposed to do.

:::caution[Verification is not a security warranty]
A verified badge records registry review state. It does not make third-party
code risk-free or guarantee that every future version behaves identically.
Review the package, version, permissions, author, and source links before
installing it.
:::

## Install and use a package

1. Open the package detail page.
2. Select **Install**. If more than one installable version is available, you
   can choose a version from **Versions**.
3. Open the target App and select **Packages**.
4. Select **Add Package**, choose the package version, and confirm.

Installation and App linkage are separate:

- **Installed on this device** means Flow-Like Desktop has the package code
  available locally.
- **Linked to an App** records the package and version the App requires. That
  declaration is used to populate its catalog and resolve remote execution.

For online Apps, the App's **Packages** screen can enable automatic updates.
Offline Apps keep an explicit linked version. Review available updates before
changing a production App's package version.

## Paid packages and project licences

A paid package shows its price on the detail page. Select the price to buy it.
The package unlocks for installing once the payment is confirmed. Your
purchases are listed under **Account → Purchases**.

Paid, private and access-request packages are licensed per App. An admin or
the owner who has the package adds it to the App and holds its licence. Every
member of the App can then use it, including on Desktop, without buying it
themselves. The **Add Package** dialog marks the packages you own. For one you
don't have yet, it sends you to the store to get it first.

If the licence holder leaves the App or loses the package, the licence passes to
another admin or the owner who has the package. If nobody else has it:

- The package's licence lapses. Updates and version changes stop right away.
- Admins and the owner get a notification. The **Packages** screen shows a
  countdown to the day the package stops working.
- The package keeps working for **30 days**. Reminders go out a week and a day
  before it is disabled.
- After 30 days the package is disabled in that App. Cloud runs and Desktop
  downloads no longer load it, and flows that use its nodes stop running.

As soon as an admin or the owner gets the package, the licence moves to them
and the package works again. You can also select **Reactivate** on the
**Packages** screen after getting it.

## Remove or update

Open **Library → Packages** to search installed packages, apply available
updates, inspect details, or uninstall a package from the device.

Before uninstalling, check which Apps use the package. Removing the local copy
can make its nodes unavailable for local editing or execution until the
required version is installed again. Removing a package from one App is a
separate action in that App's **Packages** screen.

## If nodes do not appear

- Confirm that the package is both installed and linked to the current App.
- Check the linked version and any compile-status badge.
- Reload the Flow after changing App packages.
- Open the package's **Nodes** tab to verify that the expected node is exported
  by the selected version.

## Next Steps

- [Managing Installed Packages](/start/packages-library/) — local package and
  update management
- [Creating Custom Packages](/dev/wasm-nodes/overview/) — build a WASM package
- [Registry and Governance](/dev/wasm-nodes/registry/) — publish and request
  review
