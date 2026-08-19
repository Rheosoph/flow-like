The console looks finished: dashboard, health card, branded chat. Then a teammate presses your shiny **Escalate** button and… nothing. No run, no log, no reply. A surface without an event behind it is a poster, not a product — so this lesson is the wiring.

> **Predict first:** list what has to exist between "finger presses button" and "flow runs." How many pieces can you name?

## 1 · Events are the contract

**Events** connect flows to app interfaces and external systems. A workflow-backed Event needs an *event node* in the target flow — the flow's entry point — and the Event selects and configures it. One node can back several Events: the same entry could serve a UI action and an API endpoint, differentiated by their payloads and configurations.

@AppEvents

The support app's Events tab shows the two wired surfaces from lesson 2 side by side — the **Triage selected request** Quick Action at `/triage` and the **Support assistant** Chat UI at `/chat`, both active, both entering the Customer Support Automation flow. This list is your wiring diagram: when a surface misbehaves, start here.

## 2 · Match the node

Not every Event type fits every entry node. The flow's event node determines what you can create:

| Flow event node | Available Event types |
| --- | --- |
| **Chat Event** | Chat UI, Discord, Telegram |
| **Mail Event** | Email |
| **Generic Event** | Generic Form, API, Deeplink |
| **Simple Event** | Quick Action, API, Cron, Daemon, Deeplink, REST, MCP |

The built-in UI types are **Chat UI**, **Generic Form**, and **Quick Action** — plus Page-target Events, which open a visual page directly. A Quick Action can even collect exposed Flow variables in a small form before triggering the node; that's how "Triage selected request" asks which request you mean.

## 3 · Wire the click

On a Page, an interactive component carries an **actions** array. To run a flow, it uses the fixed `workflow_event` action, identifying the selected Event's node by `nodeId`. Navigation is deliberately separate — `navigate_page` for routes, `external_link` for the outside world — so "go somewhere" and "run something" never blur.

Inside a Widget, remember lesson 4's contract: the component triggers `widget_event` with an `actionId`, and the *page's instance* binds that to a workflow. Same destination, one indirection more — and that indirection is what keeps the widget reusable.

So your silent Escalate button has exactly three suspects: no event node in the flow, no Event targeting it, or a button action that doesn't reference it.

## 4 · Pin before you ship

An Event runs its flow at a chosen version: **Latest** follows the editable draft, while a numbered version is immutable. On your own machine, Latest is exactly right — edit, run, repeat. The moment the team depends on the console, pin production-facing Events to a reviewed numbered version, and your weekday draft edits stop being everyone's surprise feature. Rolling back is just repointing.

The pre-flight, straight from the docs and worth following literally:

1. Create the compatible event node in Studio.
2. In **Events**, create an Event and select the Flow, Flow version, and node.
3. Choose an Event type and execution location supported by that node.
4. Configure its route, schedule, service credentials, or interface options.
5. Activate the Event — then test the complete invocation path, not just the flow in Studio.

**Watch out:** ownership is not wiring. A page belonging to a flow doesn't make its buttons run that flow — every click needs an explicit action referencing an Event. Nothing fires by association.

**Recap**

- Events connect surfaces to flows; the flow's event node determines which Event types are possible.
- Buttons run flows via `workflow_event` + `nodeId` — or `widget_event` + a page-bound `actionId` inside widgets. Navigation actions are separate.
- Ship on pinned numbered versions, and test through the real route.
