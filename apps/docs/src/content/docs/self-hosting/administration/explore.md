---
title: Curate Explore
description: Prepare, preview, and publish the Explore storefront
---

## Curate Explore

Administrators with `WriteLandingPage` (or `Admin`) curate the Explore storefront at
`/admin/explore` in the web and desktop apps. The **Explore** card appears under
**Content & publishing** on the admin dashboard only for those roles. See
[Explore](/start/explore/) for what viewers get.

The editor works on a shared **draft**. Viewers keep seeing the **live** layout until
someone selects **Publish changes**. Before anyone edits, the draft is the built-in
default: a spotlight filled from the most popular items, the new-this-week count, the
category tiles, and the Trending, Suites & Platforms, Top paid and For builders rows.

### Layout

The page has fixed grid slots and a list of rows:

| Slot | Takes |
| --- | --- |
| Spotlight | A spotlight rotation of apps, packages and collections |
| Announcement | An announcement with a tone, a message, an optional button and image |
| Feature | One featured app or package |
| Collection | A hand-picked or rule-based collection |
| New count | The new-this-week count rail only |
| Categories | The category tiles rail only |
| Rows | Collections and the Trending, New this week, Top paid, For builders and Suites rails |
| Unplaced | Anything; never shown as a tile |

A slot can hold several placements. The first one a viewer is allowed to see wins, and
the others are its **fallbacks**. Use them for developer-only content: a For builders row
with a New this week fallback, or a package feature with an app feature behind it. An
unplaced collection can still be a spotlight slide or a `?collection=` page.

The editor has three panes. On narrow windows the inspector opens as a sheet.

- **Placements**: add a placement, then pick one of the slots that accept it. **On this page**
  lists every placement by slot with its status. Fallbacks are indented. Drag a row to
  change its priority within its slot, or drop it on a canvas slot to move it there.
- **Canvas**: a 12-column diagram of the page. Select a tile or row to edit it, drag the
  handle to move a placement, reorder rows, add a row, or remove an empty one. Slots that
  cannot take the dragged placement are outlined in red and refuse the drop.
- **Inspector**: the selected placement's settings. Edits save to the draft automatically
  after a short pause. The **⋯** menu duplicates the placement (as a draft named "(copy)"),
  moves it to another compatible slot or to Unplaced, or deletes it. A collection that a
  spotlight still shows as a slide cannot be deleted until you remove it from that spotlight.

A placement's kind never changes. To turn an announcement into a collection, add a new
placement. Every placement has:

- **Turn on** (or **Show this rail**): until it is on, the placement is a draft that only
  admins see.
- **Schedule**: start and end in your local time zone. The hub stores them in UTC.
  Status reads Scheduled before the start and Ended after the end.
- **Audience**: Everyone, or any mix of developer mode, signed in or out, desktop or
  web, and one or more languages. Tags of the same kind match any of them; different
  kinds must all match.

Copy fields show their character limits. Links must be an app path such as
`/store/explore` or an `https://` address. Artwork and announcement images upload to the
hub's CDN. Without `platform_config.cdn`, custom artwork is off and tiles use the item's
own cover. Sponsored placements can be prepared but not turned on yet.

### Preview and publish

**Preview as** simulates a viewer (developer mode, signed in or out, desktop or web, and
language). The hub resolves the draft for that viewer. The canvas then marks the
placements it would show instead of the first choice as **fallback**, and the list marks
live placements that viewer would not get with a hidden icon. If the preview cannot load,
the canvas says so and shows no markers until you select **Try again**.

The header counts unpublished changes. **Publish changes** checks the whole draft,
copies it to live and invalidates the cached public page. Missing or private apps and
packages do not block publishing. They are listed as warnings, and viewers skip them.
**Discard** resets the draft to the live layout for every admin.

Every change carries the draft revision it was based on. If another admin changed the
draft first, the editor shows their version and keeps your unsaved inspector edits.
Steps you had already started behind that change, such as a publish, are not sent.
Choose **Save my edits again** to apply them on top, or **Drop my edits**. Publishing
first saves the inspector's pending edits and stops if they cannot be saved. Selecting
another placement while the inspector holds edits that cannot be saved asks before
dropping them.

The editor uses these endpoints, all behind `WriteLandingPage`:

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/admin/explore` | Draft, live revision, item names and unpublished changes |
| `POST /api/v1/admin/explore/placements` | Add a placement to a slot |
| `PUT /api/v1/admin/explore/placements/{id}` | Save a placement |
| `DELETE /api/v1/admin/explore/placements/{id}` | Delete a placement |
| `PUT /api/v1/admin/explore/order` | Reorder rows and priorities, or move placements between slots |
| `POST /api/v1/admin/explore/publish` | Publish the draft |
| `POST /api/v1/admin/explore/discard` | Reset the draft to live |
| `GET /api/v1/admin/explore/preview` | Resolve the draft for a simulated viewer |
| `GET /api/v1/admin/explore/media` | Sign an artwork upload |

Apply the `20260924200000_explore_layout` migration before you deploy a hub that serves
Explore. Hubs without these endpoints show a notice in the editor, and viewers keep the
classic Explore lists.

## Related

- [Platform administration](/dev/platform-administration/)

