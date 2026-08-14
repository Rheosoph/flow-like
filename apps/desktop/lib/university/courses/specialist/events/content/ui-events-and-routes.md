A teammate builds a beautiful support landing page, posts the link in the team channel — and everyone who clicks it lands on the app's default view instead. The page exists. It renders fine in the builder. Why can't anyone open it?

Because a Page doesn't own a route. Nothing in Flow-Like does, except an Event.

## 1 · Routes resolve through Events

@RouteEventArchitecture

Follow the diagram left to right: a request path like `/reports?period=30d` matches a **route mapping** that holds exactly two public fields — the path and an event ID. That resolves to an **App Event**, which owns everything else: active state, event type, board, default page. From there the Event opens whichever surface it targets — a Page, a Quick Action form, a Chat UI, or a backend workflow run. Three rules ride along the bottom: a missing path falls back to the `/` mapping, query parameters stay with navigation (they're not part of the mapping), and the Events UI enforces one owner per path.

The route stays small on purpose. Change what an Event does and its path keeps working; the mapping never needs to know.

## 2 · The intake form gets a door

Time to grow the support app: visitors should open `/support/new`, fill in subject, description, and priority, and get validation feedback. Filter one from last lesson says that's a **Generic Form** event on a **Generic Event** node.

Build it in your own app:

1. Add a **Generic Event** node with output pins for `subject`, `description`, and `priority`.
2. Validate the values and persist the request downstream — the form is not the validation.
3. Create a **Generic Form** event targeting that node; name it "New support request".
4. Set the Route Path to `/support/new`, save, and open the path directly.
5. Submit one valid and one deliberately broken request.

A valid route begins with `/`, is unique within the app, and stays short, lowercase, and descriptive. Form field names must align with the Generic Event's output pins — drift there and submitted values silently fail to map.

Here's the triage event in edit mode, showing where a route lives:

@RouteConfiguration

The **Route Path** field reads `/triage` with the helper text "Used for path-based navigation. Must be unique." — the two route rules, enforced at the source. To the right, **Flow Configuration** selects the flow, the Flow Version (*Latest* here), and the target node; an *Editing mode* bar at the bottom offers Discard or Save Changes.

## 3 · Pages ride the same rails

That orphaned landing page from the hook? Create or open a **Page-target Event**, select the page, assign a unique route like `/support`, save, and test the link again. A red **No route** badge on a UI event means it exists but can't be opened by path. Backend events — Cron, Daemon, API — never need routes; nobody navigates to a schedule.

Keeping every path on Events gives Pages, Quick Actions, Generic Forms, and Chat UIs one navigation model — and gives you one place to look when a link misbehaves.

One published-path warning: changing a live route is a user-facing compatibility change. Bookmarks break. Pick semantic, stable paths the first time.

> **Watch out:** validation that lives only in the form is decoration. API calls and replayed requests reach the same entry node without ever seeing your UI — revalidate inside the flow, before side effects.

## Recap

- Routes map a path to an Event; the Event owns the target and behavior.
- One owner per path, leading `/`, unique within the app.
- Pages become reachable only through Page-target Events.
