Your support flow works — when *you* press Run in the board editor. Your teammates don't live in the board editor. They need a button. In the next few minutes you'll build one, press it, and then we'll name what you just made.

This course follows one app — a **Customer Support Copilot** — as it grows a new surface each lesson: a triage button today, then routes and a public intake form, unattended APIs and schedules, chat and mail, and finally a rollout that can't be broken by a Friday-afternoon edit.

## 1 · Press the button

Do this in any app of yours:

1. Open a flow and drop in a **Simple Event** node — the entry node that needs no input payload. Wire something visible behind it; a single log node counts.
2. Save, then open the app's **Events** workspace from the sidebar.
3. Click **Create Event**. Select your flow, the Simple Event node, and the **Quick Action** type.
4. Name it, save it, activate it.
5. Look at the left sidebar: a **Quick Actions** section has appeared with your action in it. Press it.

That's your flow running without the editor — an invocation surface in five steps.

Here's the same recipe grown up, in the support app:

@AppEvents

The Events workspace lists two UI events: the Quick Action **"Triage selected request"** on route `/triage` and a Chat UI event **"Support assistant"** on `/chat`, both pointing at the *Customer Support Automation* flow. Bottom-left, the **Quick Actions** panel already offers the triage button — exactly where your own action just appeared.

## 2 · Name the two objects

You touched two different things, and the distinction carries the whole course:

- The **flow event node** — the Simple Event on your board — is the *entry contract*. Execution starts there, and any invocation data enters the flow through its output pins.
- The **Event record** — what you created in the Events workspace — *points at* that node and wraps it in invocation configuration: type, interface, route, schedule, version.

The Event is not the workflow. Delete the Event and your flow is untouched; delete the entry node and the Event has nothing to invoke. That's also why the build order is flow-first: most Event types can't be created until a compatible entry node exists to point at.

Click the triage event and you can read a record's full anatomy:

@QuickActionEvent

The detail page shows **Basic Information** (name, description, route path `/triage`, the event ID) next to **Flow Configuration**: flow *Customer Support Automation*, Flow Version *Latest*, node *Quick Action (triage-request-node)*. Below, the **Inputs** panel explains that input pins are captured at publish time — none were captured here, so this action fires without asking the user for anything.

## 3 · Four entry contracts

Callers deliver different data, so boards offer four main entry nodes:

- **Simple Event** — no input payload. Buttons, schedules, daemons, bare endpoints.
- **Generic Event** — a structured payload; extra output pins map named fields into typed pins. Forms, APIs, deep links.
- **Chat Event** — conversation history and chat context: sessions, tools, attachments, user info.
- **Mail Event** — the entry for Email integrations.

The Create Event dialog reads your selected node and offers only compatible types. Choosing a type never transforms the contract — a Simple Event can't sprout chat history.

One habit to start now: everything arriving through an Event is untrusted. Validate required fields and authorization inside the flow, before side effects. The caller's interface is not a security boundary.

> **Watch out:** "The interface opens but nothing runs" almost always means the Event targets a stale node ID or an incompatible flow version. Open the Event record and re-select the node.

## Recap

- Entry node = the contract inside the flow; Event record = how callers reach it.
- Build flow-first: no compatible entry node, no Event.
- Four contracts: Simple (nothing), Generic (payload), Chat (conversation), Mail (email).
