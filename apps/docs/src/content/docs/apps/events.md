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
| Quick Action, Chat UI, Generic Form, Page target | Invoked through their App interface |

Choose **Local** or **Remote** in the Event editor where both are supported.
REST and MCP Events can additionally be **Public** or **Internal**. An
Internal endpoint is callable by connected Apps through the App-connection
proxy and does not expose a public endpoint.

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
