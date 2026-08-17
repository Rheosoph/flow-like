Your flow from last lesson runs when *you* press play. Riko cannot press your play button. The bridge between "works on my canvas" and "answers customers" is an app-level **Event**: a configured surface — chat box, inbox, API route, schedule — bound to your flow's entry node.

@NodeTypes

Re-anchor on the red node. The event node on the canvas is the *graph's* entry point; the app's Event is the *world's*. Wiring one to the other is this lesson.

## 1 · Pick the surface the entry node allows

In the app's Events workspace you create an Event, select the target flow, and select the event node inside it. Which surfaces you're offered depends on that node: the type of entry node you placed determines which surface types can bind to it — a chat-style entry backs conversational surfaces, a generic one backs forms and API calls, and so on. So the decision you made casually in lesson 6 ("any event I can trigger") is actually the first release decision. Choose the entry node for the surface you intend to ship.

Then configure only what the flow honors. An attachment upload control on a flow that ignores attachments isn't a feature, it's a small lie to your users.

## 2 · Latest, or a numbered version

Every Event points at an implementation of the flow: **Latest** (the live, editable draft) or a **numbered version** (an immutable snapshot you create deliberately).

> **Predict first:** your support automation is now customer-facing, and you're still editing daily. Which pointer does the production Event get?

The version, every time. Latest follows every save — convenient while iterating, and exactly how a Tuesday-afternoon "small tweak" reaches customers mid-edit. The working rhythm: iterate against Latest privately, test, cut a version, point the customer-facing Event at it. Note what a version is *not*: it's not editable (that's the point), and it's not a substitute for testing — it preserves a behavior you've already proven, it doesn't prove it.

## 3 · Match the run location to the graph

Some catalog nodes are local-only — they need the desktop machine (marked with a monitor badge in the catalog). Before a run, the *entire* flow is inspected, nested layers included. One local-only node anywhere means the whole invocation runs locally; a single run is never split between a local machine and a remote worker.

Remember lesson 5's warning: collapsing hides nodes from your eyes, not from the runtime. A local-only node folded three layers deep still pins the flow to the desktop — so if a "fully remote" flow refuses to deploy remotely, open the layers and look. And if a surface depends on a local runner, that machine has to actually be on when customers arrive.

The support board's variables matter here too: it exposes Customer Name and Escalation Enabled (the open eyes in @FlowVariables) so the surface and app configuration can supply them per invocation, while the mail token stays Secret + Runtime Configured on whichever machine runs the flow. Configuration travels with the Event; credentials stay put.

> **Watch out:** an Event is configuration, and configuration fails quieter than graphs. "Wrong flow selected", "still pointing at Latest", "remote Event over a local-only graph" — none of these look like bugs on the canvas, because they aren't on the canvas.

## Recap

- An app Event binds a real surface to your flow's entry node; the entry node's type decides which surfaces are possible.
- Customer-facing Events pin a tested numbered version; Latest is for private iteration.
- One local-only node — even deep inside a layer — makes the entire run local.

Your flow has a front door. Next lesson: proving what walks through it behaves.
