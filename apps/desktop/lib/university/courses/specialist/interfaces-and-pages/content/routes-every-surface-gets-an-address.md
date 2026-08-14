You demo the console at your desk and it sings. Then the support lead installs the app, opens it — and lands nowhere useful. The dashboard exists. The chat runs fine in preview. She can reach neither, and in the Events workspace the culprit is a small red badge: **No route**. A surface without an address might as well not exist.

> **Predict first:** who should own the address `/support` — the Page, the Flow, or something else entirely?

## 1 · A route is a path → Event mapping

Flow-Like stores each route as a mapping from a path to a **UI Event**. Not to a page — to an Event. The Event then decides what opens:

- a **Page-target Event** opens its configured Page;
- a **Chat UI Event** opens the chat experience;
- another UI Event opens the interface its type provides.

Backend-only Events — a Cron schedule, an API endpoint — don't need a route at all. Nobody navigates to a schedule.

@RoutesOverview

That's the Events workspace of the support app. Both UI Events wear their route as a badge: **Triage selected request**, a Quick Action, answers at `/triage`; **Support assistant**, a Chat UI, answers at `/chat`. Both point at the same Customer Support Automation flow, and the green dot on each shows it's active. The search field above filters by event name, description, flow — or route.

## 2 · Set an address

Two ways to edit a route, depending on where you are:

1. **On the Events list** — select the route badge, edit the path, press `Enter`. `Escape` cancels.
2. **In the Event editor** — open the Event and edit **Route Path** under Basic Information, alongside everything else.

@RouteConfiguration

Here the Triage event is open for editing. The **Route Path** field reads `/triage`, with the fine print that matters: *used for path-based navigation, must be unique*. On the right, Flow Configuration shows what this route ultimately reaches — the Customer Support Automation flow, version Latest, entering at its Quick Action node.

The rules are short. A route starts with `/`, must be unique within the app, and is matched as configured — `/chat` answers `/chat`, not `/chat/anything`. Keep it short, lowercase, and descriptive: `/support` beats `/page1`, because a route is part of the app's user-facing navigation. And `/` is special — it's what opens at the app's root.

Your console's map, then:

| Path | UI Event | Experience |
| --- | --- | --- |
| `/` | Support home | The operations dashboard Page |
| `/chat` | Support assistant | Chat UI |
| `/triage` | Triage selected request | Quick Action |

## 3 · Move between addresses

Routes aren't just for typing into an address bar. A2UI buttons and links can navigate to another configured path, and a flow can return navigation instructions to the active interface — "reply sent, back to `/`". Either way, Flow-Like resolves the new path to its Event and opens that Event's UI. The dashboard's "Open assistant" button will simply navigate to `/chat`.

**Watch out:** a Page never owns its route, and no page setting makes it reachable. If a page can't be opened, you're not missing a page option — you're missing a UI Event with that Page as its target and a unique Route Path.

**Recap**

- A route maps a path to a UI Event; the Event decides which interface opens.
- Edit routes on the Events list badge or in the Event's Route Path field; paths start with `/`, stay unique, and match as configured.
- `/` is the app's front door — spend it on the surface people should see first.
