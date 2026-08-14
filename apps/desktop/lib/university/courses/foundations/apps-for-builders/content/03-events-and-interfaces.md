Your Customer Support Automation flow drafts lovely replies — for exactly one user: you, with Studio open. Nobody on the support team is ever going to run a graph by hand. They need a chat window at `/chat` and a one-click triage button. Turning "a flow that runs" into "a product people open" is what Events do — and it starts one node earlier than most people expect.

## 1 · The entry node signs the contract

An Event doesn't attach to a Flow; it attaches to a specific **event node** inside a Flow. That node's family decides which Event types are even on the menu:

| Entry node in the Flow | Event types it can back |
| --- | --- |
| **Chat Event** | Chat UI, Discord, Telegram |
| **Mail Event** | Email |
| **Generic Event** | Generic Form, API, Deeplink |
| **Simple Event** | Quick Action, API, Cron, Daemon, Deeplink, REST, MCP |

So design in this order: choose the interaction, place the matching entry node, define its inputs and result, *then* configure the Event. Designing the route first and hoping some node will fit ends in a contract mismatch at the last step.

Multiple Events can point at the same node — a `/chat` route for the team and a Telegram connection for the field crew can share one Chat Event node.

## 2 · Pick the smallest surface that works

For each Copilot interface, climb this ladder and stop at the first rung that fits:

- **Quick Action** — a manually triggered App action. Its form collects the Flow's exposed variables, then fires the event node. Perfect for "triage this request now."
- **Generic Form** — a route-backed form that submits the Flow's exposed variables to a Generic Event node.
- **Chat UI** — when conversation history and a chat-shaped response are the point. Attachments, tools, prompts, and an AI disclosure are configurable; enable only what the Flow actually handles.
- **Page** — dashboards and multi-control tools built from typed A2UI components. A Page belongs to a Flow; interactive components trigger behavior through a `workflow_event` action aimed at a node ID, and the App exposes the Page through a UI Event with a unique route. Pages can also fire events on load, unload, or an interval — treat those as execution contracts with real cost, not decoration.

Backend and integration Events skip UI entirely: API exposes one endpoint, REST a multi-endpoint remote surface, MCP a Model Context Protocol server, Cron a schedule, Daemon a supervised long-running local Flow.

## 3 · Wire it, then test the real route

Here's the Copilot's Events workspace with both interfaces live:

@EventWorkspace

Two UI Events, each listing its type, route, and backing Flow: **Triage selected request**, a Quick Action on `/triage`, and **Support assistant**, a Chat UI on `/chat` — both wired to the Customer Support Automation flow. The **Pages** tab sits right beside the Events tab, and **Create Event** is the button you'll press for every new surface.

Creating one takes a minute: pick the Flow, the event node, and a supported type; choose the Flow version (Latest or a numbered snapshot — lesson 5); choose Local or Remote where the type allows; then set route, schedule, credentials, or interface options; activate.

Location follows one organizing rule: **anything that must answer while every Desktop is closed has to be Remote.**

| Location | Event types |
| --- | --- |
| Local or Remote | API, Cron |
| Local only | Daemon, Deeplink, Discord, Telegram, Email |
| Remote only | REST, MCP |
| Through the App's interface | Quick Action, Chat UI, Generic Form, Page |

> **Watch out:** a green run in Studio proves the graph, not the route. The Event adds its own moving parts — selected version, location, exposed variables, route configuration — and any of them can diverge from your Studio session. An interface is done when the real button, form, or URL produces the run you expect.

## Recap

- Events attach to entry nodes, and the node's family fixes the menu of Event types.
- Pick the smallest surface that fits: Quick Action → Generic Form → Chat UI → Page.
- Test the real route end to end; a Studio run proves only the graph.
