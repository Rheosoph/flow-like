---
title: Explore
description: Discover apps and node packages on one curated storefront, and search everything the store holds
sidebar:
  order: 26
---

**Explore** is the storefront of your hub. It mixes community apps with WASM
node packages on one page, and a search page lets you narrow everything down.
Open it from **Explore** in the sidebar (or the bottom bar on phones).

## The landing page

The top of the page is a grid of tiles that the hub's curators arrange:

- **Spotlight** — a rotating hero with an app, a package or a collection. Use
  the numbered bars or the arrows to switch slides. The pause button next to
  the arrows stops the rotation until you press play again. Rotation also pauses
  while you hover the tile or move keyboard focus into it, and stops completely
  when your system asks for reduced motion.
- **Announcement** — news from the hub, such as a launch or planned
  maintenance. Close it with **×**; **Restore announcement** next to the page
  subtitle brings it back. A changed announcement shows again even if you
  closed an older version.
- **Featured** — one app or package the curators picked.
- **Collection** — a themed group of apps and packages. **Open collection**
  lists all of its items.
- **New this week** and **category tiles** — quick counts and shortcuts into
  search.

Below the grid, rows show what is **Popular right now**, **Suites &
Platforms**, **Top paid** (best sellers of the last 30 days), **For builders**
and any collections the curators added.

Apps always appear as cover cards and packages as technical cards with their
version, permissions and install count, so you can tell at a glance what you
use and what you build with.

A hub that has not published anything to Explore yet says so and offers
**Browse everything**, which searches everything public on the hub.

## Apps and packages

Node packages are builder material, so they only appear when
[Developer Mode](/start/developer-mode/) is on. With it on, the **All · Apps ·
Packages** switch in the header narrows the page to one kind. Each view keeps
its own picks: when a tile or row has nothing to show for the current view, the
hub fills it with the next suitable content instead of leaving a gap.

With Developer Mode off, Explore shows apps only. A shared link to the package
view then shows the apps instead of an empty page.

## Search and filters

Type in the search field and press <kbd>Enter</kbd> to open the search page,
or open it from **See all** on any row. The search page keeps your query and
filters in the address, so you can bookmark or share a result list. The last
few searches you submitted with <kbd>Enter</kbd> appear as **Recent** next to
the search field; they are stored only in this browser.

| Filter | What it narrows |
| --- | --- |
| **Type** | Apps, packages (Developer Mode) or collections |
| **Category** | App categories, and package categories in Developer Mode |
| **Price** | Free or paid |
| **Trust** | Packages from verified publishers (Developer Mode) |
| **Permissions** | Packages that request no permissions, network, models or storage (Developer Mode) |

Results are grouped into a matching collection, **Apps** and **Packages**.
Packages found through a collection say so under the card. A search with a
**Permissions** filter looks at the first 200 matching packages; when there
are more, the list ends with a hint to narrow your filters. When few results
match, **You might also like** suggests popular apps nearby. **Clear filters**
resets everything but your query and sort order; inside a collection you stay in
that collection.

In Flow-Like Desktop with Developer Mode on, **Build it yourself** starts a new
node package from a template when nothing in the store fits.

Opening a package from Explore and pressing **Back** on its store page returns
you to the same Explore or search view.

## Hubs without the new storefront

Self-hosted hubs that have not been updated yet keep the previous Explore apps
page and package list. Nothing needs to be configured; Flow-Like picks the
right page for the hub you are connected to. A link with a search, an app
category or a sort order opens the previous page with the same search, category
and sort; filters that page does not have, such as package categories or
permissions, are dropped. A link to packages opens the previous package list
when Developer Mode is on.

## For hub administrators

Curators with the **Write landing page** permission see **Edit layout** in the
header, which opens the Explore editor in the admin area (see
[Platform administration](/dev/platform-administration/)).
