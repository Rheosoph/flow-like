---
title: Events
description: Configure events to trigger flows
sidebar:
  order: 40
---

With **Events**, you can connect your **Flows** to app interfaces and external
systems. The Events workspace also includes a **Pages** tab for managing visual
interfaces and their navigation paths.

Creating a workflow-backed **Event** requires at least one existing **Flow** in
your app that includes an *event node*. You can create and manage them in your
app’s [**Flows** section](/apps/boards/).

Most workflow-backed Events target a specific *event node* within a particular
Flow. You can create multiple Events that reference the same event node and
differentiate them by their payloads and configurations. Page-target Events can
instead open a visual page directly.

![The Events workspace in Flow-Like Desktop, showing configured UI Events](../../../assets/AppEvents.webp)

## Event Types

The list groups Events into **UI Events**, which expose an app interface and a
route path, and **Backend-only Events**, which run without a built-in app
interface.

Which Event types are available depends on the event node in the selected
Flow:

| Flow event node | Available Event types |
| --- | --- |
| **Chat Event** | Chat UI, Discord, Telegram |
| **Mail Event** | Email |
| **Location Event** | Location Region |
| **Generic Event** | Generic Form, API, Deeplink |
| **Simple Event** | Quick Action, API, Cron, Daemon, Deeplink, REST, MCP |

The built-in UI types are **Chat UI**, **Generic Form**, and **Quick Action**.
A Page-target Event is also listed under UI Events because it opens a visual
page directly.

### Quick Action

A **Quick Action** adds a manually invoked action to the App. Its form can
collect exposed Flow variables before triggering the selected event node.

![The configuration screen for a Quick Action event in Flow-Like Desktop](../../../assets/QuickActionEvent.webp)

### Chat UI

A **Chat UI** Event invokes a Flow through the built-in
[chat interface](/apps/chat-ui/). It passes the chat context, such as message
history, to the event node and can also accept file attachments, tools, and
default prompts.

### Generic Form

A **Generic Form** creates a route-backed form from the Flow's exposed
variables. Submitting the form invokes the selected Generic Event node.

### External and background Events

- **API** exposes one configured HTTP endpoint.
- **REST** exposes a multi-endpoint REST surface with authentication.
- **MCP** exposes a Model Context Protocol server.
- **Cron** invokes a Flow on a schedule.
- **Daemon** supervises a long-running local Flow and can restart it after a
  failure.
- **Location Region** invokes a Flow when this device enters or leaves a
  configured circular region.
- **Deeplink** invokes a Flow through a Desktop deep link.
- **Discord**, **Telegram**, and **Email** connect their respective services to
  the compatible event node.

## Local and Remote availability

Event types are constrained by where their sink can run:

| Event type | Availability |
| --- | --- |
| API, Cron | Local or Remote |
| Daemon, Deeplink, Discord, Telegram, Email | Local |
| REST, MCP | Remote |
| Location Region | iOS or macOS sensor; Local or Remote workflow |
| Quick Action, Chat UI, Generic Form, Page target | App interface; Remote for hosted frontends |

Choose **Local** or **Remote** in the Event editor where both are supported.
REST and MCP Events can additionally be **Public** or **Internal**. An
Internal endpoint is callable by connected Apps through the App-connection
proxy and does not expose a public endpoint.

### Publish a hosted chat, form or page

An App's chats, forms and pages can open from one link in the existing Flow-Like
web application, including its static export: `/a/<app-id>/<route>`. The route
selects the interface, the same way it does inside the App, so `/a/<app-id>/`
opens the App's default route and `/a/<app-id>/orders` opens the Event whose
route is `/orders`. Workflow execution runs on the Flow-Like server.

Each route is published separately. Hosting starts disabled, and turning it on
requires visitors to sign in by default.

1. Give the Event a **Route Path** under **Identity**. Chat UI, Generic Form,
   Quick Action and Page-target Events have one; the App's default Event
   answers `/`.
2. Select **Hosting** and turn on **Enable static hosting**.
3. To accept anonymous visitors, separately turn on **Allow anonymous access**
   and confirm the warning: anyone with the link can execute workflows, and the
   App owner pays for their usage. Leave this off to require sign-in.
4. Set execution to **Remote**, exposure to **Public**, and activate the Event.
   The Hosting section lists each requirement that is still missing, with a
   button to fix it. If the Flow fixes the execution location, change it in the
   Flow first.
5. Save the Event, then copy **Link to this route**.

The editor includes the API host in the link. With separate API and web hosts,
the API redirects visitors to the same path on the existing web application.
With a shared host, the path opens directly in the web application.

Navigation between pages keeps visitors on the same App link and changes only
the route. A navigation target works when its Event is published too and uses
the same sign-in setting as the current page; otherwise visitors see that the
page has not been published on this link.

When sign-in is required, a visitor follows the installation's Flow-Like login
flow and returns to the same frontend. The workflow receives the authenticated
user's identity. **Public** exposure allows the hosted route to exist;
**Allow anonymous access** separately controls anonymous use.
Disabling hosting, removing the route, deactivating the Event or changing it to
Local or Internal removes hosted access for that route. Disabling hosting also clears the anonymous choice in
the editor, so enabling it again starts with sign-in required. Save the Event
to apply these changes.

Self-hosted installations use their existing web deployment and login callback.
See [Host chat, form and page frontends](/self-hosting/containers/#host-chat-form-and-page-frontends)
for static route and API redirect configuration.

### Location Regions

Add a **Location Event** node to a Flow, then create a **Location Region** Event
from it. Choose a Geometry Point center in longitude-latitude order, a radius
in meters, and entering, leaving, or both transitions. Save and activate the
Event to register the region on this device. For a Remote Event, this device
sends the configured center, radius, and transition to the server workflow.

**While the app is open** stops monitoring when Flow-Like leaves the
foreground. **Also in the background** opts into native region monitoring.
Use the explicit permission buttons in the Event editor to grant location
access. Reading the permission status, editing a region, or activating an
Event never opens a permission prompt. iOS background monitoring requires
Always authorization; the operating system may ask for this in stages.

The editor shows authorization, monitoring errors, and the device's region
count. Up to **20 regions** can be monitored at once. On macOS, Flow-Like must
stay running and the computer must be awake. On iOS, the system schedules
crossing delivery, so callbacks can be delayed or unavailable after
force-quitting the app. Region monitoring does not record a continuous path.

The node's **Region center** Geometry output is the monitored center. It is not a
measurement of the device's position. Use **Get Current Location** separately
while the app is open when the workflow needs a fresh fix and its accuracy.

Pending crossings use an at-least-once queue, capped at 128 transitions and
one hour. A retry can deliver the same crossing again. Use **Transition ID**
to deduplicate workflow side effects. Disabling the Event or changing account
invalidates its pending crossings.

## Select the Flow implementation

An Event targets an event node in a Flow and can use:

- **Latest**, which follows the editable Flow draft; or
- a numbered, immutable Flow version.

Pin externally consumed or production-facing Events when draft changes should
not alter live behavior. See [Versioning](/studio/versioning/).

## Configure and test

1. Create the compatible event node in Studio.
2. In **Events**, create an Event and select the Flow, Flow version, and node.
3. Choose an Event type and execution location supported by that node.
4. Configure its route, schedule, service credentials, or interface options.
5. Activate the Event and test the complete invocation path.

For online and server-side behavior, see
[Offline vs. Online](/apps/offline-online/).

## Siri, Shortcuts, and native widgets

In the iOS and macOS apps, open an Event's **Identity** settings and enable
**Native integrations**. Choose its system surfaces, then save and activate the
Event. Page and Chat launchers open their existing interface. Quick Actions,
forms, and API Events can run with their configured input defaults. Your current
access and the saved Event settings are checked again each time.
The Shortcuts **Input** accepts a JSON object keyed by Event input names and
overrides those defaults. For an Event with one String input, plain text also
works. Unknown inputs or values with the wrong type stop the run.
Siri and Shortcuts share one action catalog and are enabled together.

### Use an answer in another Shortcut action

- **Ask FlowPilot** takes a question and returns its final answer as Text.
- **Ask App Chat** takes a Chat Event and question and returns the chat reply as Text.
- **Run Event** returns an Event Result with **Text** and **JSON** properties.
  Chat replies populate Text. The **Return Generic Result** node supplies JSON,
  preserving objects, arrays, numbers, booleans, and null. String results also
  populate Text. Execution logs and intermediate reasoning are excluded.

For example, add **Ask FlowPilot**, enter a question, then add **Create Note**
and select the Ask FlowPilot output as the note's content. The same chain works
with **Ask App Chat**. For an object result, select its **JSON** property
and use **Get Dictionary from Input** before reading its fields.

These actions bring Flow-Like to the foreground and wait for execution. A
question that needs more interaction can continue in the app. Supply required
Event inputs in the Shortcut; a missing input or a page-only Event reports that
interaction is needed. The response wait ends after 90 seconds, and the system
may cancel it sooner. A run that already started can continue in Flow-Like.
Responses are limited to 256 KiB in total, with at most 192 KiB each for Text
and JSON. Larger responses report an error rather than returning truncated data.

For a hosted **MCP** Event, select one registered tool in its native settings.
**Run Event** calls that selected tool with a JSON object in **Input** and
returns its JSON result. Widget launchers open an arguments dialog for review.
Changing or removing the selected tool invalidates an old shortcut. REST and
background trigger Events need a supported app interface before they can
become native entry points.

**Open App**, **Open Notifications**, and **Open Workspace** open their named
surfaces. The FlowPilot widget also has a voice entry point: start dictation in
the app, review the transcript, and press **Send**. If speech recognition is
unavailable, use keyboard dictation or type instead. Microphone capture stops
when the screen is hidden.

Add native widgets for Open App, FlowPilot, Inbox, Needs attention, Recent runs, Recently
used apps, Usage, Workspace, or Favorite Events. To include an Event in
**Favorite Events**, select **Widgets** and enable its favorite setting. On
iOS 18 and macOS 26 or later, system controls also open FlowPilot,
Notifications, or a selected Event. Widgets display cached app data; the
operating system controls their refresh schedule. Open the app to refresh a
stale widget. **Data Chart** and **App Page** widgets let you choose content
in **Settings → Native widgets**. App pages use a supported set of native
elements; widgets cannot display a live camera.

### Create a Data Chart or App Page widget

Open **Settings → Native widgets** in Flow-Like on the device where you want
the widget. Choose **Data chart** or **App page**, give it a name, and select
its app. Preview the content, choose a color, and save. Then add Flow-Like's
**Data Chart** or **App Page** from the iOS Home Screen widget gallery. Edit
the widget to select the configuration you saved. Both types support small,
medium, and large sizes.

A Data Chart reads a table, ontology object type, or saved query. Configure
measures, grouping, filters, and number formatting before choosing a chart.
Supported types are single metric, progress, gauge, columns, horizontal bars,
stacked columns, line, area, donut, and pie. Progress and gauge require a
positive target. Aggregations run over the selected source before limiting
the result. Each widget displays up to 120 points and six series.

An App Page displays a published page, or one container within it. Choose
its internal path and enter query values as plain text; Flow-Like handles
link encoding. The native renderer supports text, images, icons, cards,
badges, progress, compact tables, common charts, and basic layouts. Buttons
and links open the app. Inputs, camera and audio elements, and other
interactive components need the full app. The preview lists content that
cannot appear in the widget.

Open the selected app page to capture content populated by workflows or
user input. Widget refreshes do not run page lifecycle workflows. Charts
refresh at the configured interval while Flow-Like is open and visible.
When the app is closed, widgets display the last cached content and its
update time. iOS controls when that content appears on the Home Screen;
the configured interval is not a background execution schedule. Content
expires after seven days without an update.

Saved configurations and cached content belong to the selected account,
profile, and hub on this device. Switching accounts clears the active
native content. Open the intended workspace before choosing a saved widget
in the gallery. You can save up to 12 configurations per workspace.

### Open an app at a chosen path

In Shortcuts, add Flow-Like's **Open App** action and choose an **App**. Leave
**Path** empty to open the app normally, or enter a path saved on one of its
active UI Events, such as `/orders/123?tab=details`. The path stays inside the
chosen app. A missing or inactive route shows an error instead of opening a
different page.

For a Home Screen or desktop launcher, add the **Open App** widget and edit it.
Choose the app, optional **Internal path**, and optional **Widget title**.
The widget opens that destination when tapped; it does not render the page
inside the widget. Open Flow-Like in the desired account and profile first
to refresh the app picker.

Both the Shortcut and widget have separate lists of query names and values.
Pair them by position, keeping the lists the same length. Enter values as
plain text, including spaces, Unicode, `+`, `%`, `&`, and `#`. Flow-Like encodes
them for the link. For example, the value `A+B & 東京 50%` reaches the Event
unchanged. Do not percent-encode values in these lists yourself.

A query already written in **Path** uses normal URL syntax: `+` means a space,
and `%2B` means a literal plus. Structured pairs are appended after that query,
so names can repeat. With `/orders?tag=first` and another pair `tag: second`,
**Get Query Params** returns `second` for `tag`. The Event payload also includes
`_query_param_values.tag` as `["first", "second"]`, preserving every value in
order. Names such as `id`, `eventId`, and `route` remain app data and cannot
change the selected app. Literal names such as `_id` remain distinct from `id`.

Use at most 32 pairs in total. Paths are limited to 4,096 UTF-8 bytes, names to
256 bytes, values to 4,096 bytes, and the combined path and structured pairs to
8,192 bytes. External URLs, traversal segments, control characters, and raw
fragments in **Path** are rejected. A `#` in a structured value is supported.

### Widget data and Handoff

**Recently used apps** records actual app openings on this device, separately
for each account and profile. It does not use the app's last edit date.
**Needs attention** lists Error and Fatal execution records from the last seven
UTC calendar days, including today. **Usage** shows account-wide execution and
model totals across profiles, covering all recorded usage. Recorded AI cost
includes model and embedding costs in USD. **Workspace** counts apps in the
selected profile; its execution count covers the whole account across profiles
over the last seven UTC calendar days, including today.
Available live execution metadata also feeds Recent runs and iOS Live
Activities while the app receives run updates.

App and Event pickers in Shortcuts show cached app artwork when available.
The Open App widget, recent app rows, and workspace app tiles use the same
artwork, with initials as a fallback. Apple controls the icons on top-level
Shortcut action tiles.

The device's system cache contains display names, status, navigation targets,
and small copies of app artwork, without Event configuration, input defaults,
or access tokens. Answer requests and results are stored briefly in the shared
app container so Shortcuts can receive the response. Account
or workspace changes clear the native cache and invalidate queued actions.
Spotlight can open FlowPilot, usable apps, and Events selected for Spotlight.
Handoff can continue an exposed page or chat on another device signed into the
same account and profile when the hub supplies an HTTPS app address. For a
configured app path, Handoff includes that path and its app-owned query values.
Unrelated outer URL fields are excluded.
