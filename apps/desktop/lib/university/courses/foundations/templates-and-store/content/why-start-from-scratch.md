Monday, 9:04. Your third new client this quarter just signed, and Priya is already rebuilding the intake triage flow — squinting at a screenshot of the last version taped to her second monitor. Your team ships the same support kit every single time: a triage flow, a reply drafter, a weekly digest. By the end of this course, that kit lives in a shared template library and your app sits in the Flow-Like store. The habit to break comes first, though: before you build anything, check whether somebody already built it.

> **Predict first:** You need mail-handling nodes for the triage flow. Where do you look before dragging a single node — and is what you're grabbing an *app* or a *package*?

Here's the whole course in one picture. The banner shows a store window of package cards — one stamped with a green check — a dotted line flowing from that window onto a pair of wired flow nodes, with a version card hanging beneath them:

@CourseBanner

Pick something proven, drop it into your board, keep it versioned. That's the loop you're learning.

## 1 · Window-shop from the home screen

Open Flow-Like. The home screen is already a storefront: under the "What do you want to build?" headline and the Ask FlowPilot bubble sit **Browse by category** chips (Productivity, Business, Communication, and friends), a **Featured** rail of hand-picked apps and the latest Flow-Like news, and **Top Charts** — the most popular community apps, by category.

@LandingPage

**Explore** in the left sidebar opens the full hub. That's your two-minute win: open it now and search for "support". You'll see what other people have already shipped for exactly your problem — before you commit an afternoon to rebuilding it.

## 2 · Two kinds of reuse

The Explore hub carries two different shelves, and mixing them up costs real time:

- **Community apps** are whole projects — flows, interfaces, data, access settings. You join one to use it, or fork it to make it your own (next lesson).
- **Packages** are node collections. They do nothing on their own; they add new nodes to the catalog your flows build from. Select **Packages** in the Explore header to browse the registry — the same shelf is reachable from **Library → Packages → Browse Packages**.

Package cards show the current version, category, install count, rating, price, and visibility, and a shield marks a package that completed the registry's verification process. Lesson 2 teaches you to actually read those signals.

## 3 · Installed is not linked

Here's the part that bites almost everyone once. Packages live in three scopes:

| Scope | Where | What it means |
| --- | --- | --- |
| Registry | Explore → Packages | Versions that exist in the world |
| Device | Library → Packages | Code installed on this computer |
| App | Open the app → Packages | The version *this app* declares it needs |

Installing puts code on your device. **Linking** — Add Package inside the app's own Packages screen — records the dependency that fills that app's node catalog and resolves remote execution. Priya once installed a mail package, opened her flow, found no mail nodes, and restarted Flow-Like twice before spotting it: she never linked the package to the client app.

When nodes don't show up, work this list:

1. Confirm the package is installed *and* linked to the current app.
2. Check the linked version and any compile-status badge.
3. Reload the flow after changing the app's packages.
4. Open the package's **Nodes** tab to confirm the node is exported by that version.

> **Watch out:** Updating the device copy in Library → Packages does not rewrite each app's linked version. Every app keeps its own declaration — review an app's Packages screen instead of assuming it moved.

## Recap

- The home screen and the **Explore** hub are your storefront: community apps and the package registry live there.
- Apps are whole projects; packages add nodes to your catalog.
- Installed on the device ≠ linked to the app — linking is what makes nodes appear.
