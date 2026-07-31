---
title: Routes
description: Map URL paths to pages and events in your app
sidebar:
  order: 42
---

Routes give a UI Event a stable path inside your app. Examples include `/` for
the default experience, `/support` for a Page, or `/chat` for a Chat UI.

Flow-Like stores each route as a path-to-Event mapping. The Event then decides
which interface to open:

- a **Page-target Event** opens its configured Page;
- a **Chat UI Event** opens the chat experience;
- another UI Event opens the interface provided by its Event type.

Backend-only Events do not need a UI route.

## See all routes

Open the app's **Events** workspace. Every UI Event shows its current route as a
badge next to the Event. A red **No route** badge means that the Event is not
yet reachable by path.

![The Events workspace in Flow-Like Desktop, showing route badges for UI Events](../../../assets/RoutesOverview.webp)

Use the Event search field to filter by Event name, description, Flow, or route.

## Set or change a route

You can edit a route in either place:

1. On the Events list, select the route badge, edit the path, and press
   `Enter`. Press `Escape` to cancel.
2. Open the Event, select its **Route Path** value under **Basic Information**,
   and edit it with the rest of the Event.

![The Route Path field while editing a UI Event in Flow-Like Desktop](../../../assets/RouteConfiguration.webp)

Route paths:

- start with `/`;
- must be unique within the app;
- should be short, lowercase, and descriptive;
- are matched as configured, so use separate Events for separate paths.

Use `/` for the interface that should open at the app's root URL.

:::tip[Name the experience]
Prefer `/support`, `/orders`, or `/account-settings` over names such as
`/page1`. A route is part of the app's user-facing navigation.
:::

## Connect a Page

A Page does not own the route by itself. To expose a Page:

1. Create the [Page](/apps/pages/) from its Flow.
2. Create or open a UI Event in the app's **Events** workspace.
3. Configure the Page as that Event's target.
4. Give the Event a unique **Route Path**.
5. Save the Event and test the path in the app.

Keeping routes on Events lets the same navigation model work for Pages, Chat
UI, and other app interfaces.

## Navigate between routes

A2UI buttons and links can navigate to another configured path. A Flow can
also return navigation instructions to the active interface. When navigating,
Flow-Like resolves the new path to its Event and opens that Event's UI.

For example, a support app might use:

| Path | UI Event | Experience |
| --- | --- | --- |
| `/` | Support home | Support operations Page |
| `/chat` | Support assistant | Chat UI |
| `/triage` | Triage selected request | Quick Action interface |

See [Events](/apps/events/) for Event setup and [Custom UI
(A2UI)](/apps/a2ui/) for navigation actions inside an interface.
