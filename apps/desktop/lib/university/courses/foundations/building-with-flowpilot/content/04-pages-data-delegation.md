The triage board works — but your support team is never going to open Studio. They need a dashboard, and you need answers about ticket data. Good news: the same conversation that built the board builds these too. FlowPilot hands interface work to its UI specialist and data work to Data Studio, and you keep directing in plain language.

## 1 · Draft a page in conversation

Describe the surface the team needs — layout, content, data, and states:

```text
Create a responsive support operations dashboard with summary cards for
open tickets, first response time, and SLA, a priority queue list, and
loading, empty, and error states.
```

FlowPilot generates a complete component tree, validates it, and stages it as a preview. Apply replaces the surface; dismiss leaves it untouched — same contract as board edits.

@PageBuilder

Here's the result open in the Visual Page Builder: a **Support Operations Dashboard** with summary cards (Open tickets 24, First response 4m 18s, Within SLA 94%), a Priority queue of tickets closest to their SLA, and an Automation coverage panel. The Open tickets card is selected, so an orange Card toolbar hovers over it, and the Props panel on the right shows exactly what FlowPilot generated: Component ID `open-tickets-card`, its title, description, and a `bordered` variant. Component palette on the left, live page in the middle, properties on the right — everything the specialist produced is inspectable, and nothing is hidden in code.

Targeted edits work the same way as on boards: open FlowPilot *inside* the builder and it receives the component tree plus your selected components. Select the card, then say "make this more compact."

One boundary matters. The UI specialist owns the page, widgets, styling, and bindings. It does **not** build the workflow that feeds them. When you need the page *and* its data, give global FlowPilot one outcome-oriented ask:

```text
Build the support dashboard page and the workflow that loads its ticket
data. Expose it as a page event and include useful error handling.
```

It coordinates the UI specialist with the board specialist and wires the connecting Event.

## 2 · Ask about your data

@DataStudioOverview

This is Data Studio for our app — its data home. A **Customer Operations** ontology (6 objects · 6 relationships) sits over 11 tables, with counters for ontologies, object types, and actions across the top. From the global assistant you rarely need to click through any of it: FlowPilot delegates data work here — inspecting tables, shaping ontologies and overlays, writing SQL or Cypher, running analytics — and presents results inline, charts included. "How many open tickets mention refunds, by week?" is a one-line ask.

One routing rule is worth memorizing. For normal app use, FlowPilot first inspects the app's configured Events and prefers a matching chat, page, or headless interface. It goes to the raw data deliberately — when you explicitly ask for schema, table, or SQL-level access, or when no configured Event covers the request. And if a matching Event fails or declines, FlowPilot does *not* silently bypass it through the underlying data.

## 3 · Open the app where you are

FlowPilot can also open an app's page or chat inline in the conversation — which is how you test what you just built without leaving the chat.

@ChatUI

This is the triage app's own customer-facing surface: a **Support assistant** chat with a welcome line, a message box, suggestion chips ("Where is my order?", "Help me update my subscription", "I need to speak with support"), and a footer noting that responses may need human review. Ask FlowPilot to open it inline and you're experiencing exactly what a customer will. Attach a file first and — when the app chat is compatible — FlowPilot passes it along.

Quick recap:

- The UI specialist builds pages, widgets, and bindings — previewed before apply — but never the workflow behind them.
- One outcome-oriented ask to global FlowPilot coordinates page, workflow, and Event together.
- Data requests route Event-first; Data Studio delegation is a deliberate tool, not a silent fallback.
