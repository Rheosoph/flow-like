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
- **Permissions** — declared resource limits and host capabilities.
- **Versions** — available, installed, yanked, disabled, or review versions.
- **Reviews** — user reviews and ratings.

Permissions can include network access, scoped storage, OAuth scopes, runtime
variables, cache, streaming, A2UI, or model access. Network declarations can
also constrain allowed hosts and protocol families. Requesting a permission
does not mean the package should receive it blindly—compare the declaration
with what the nodes are supposed to do.

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
