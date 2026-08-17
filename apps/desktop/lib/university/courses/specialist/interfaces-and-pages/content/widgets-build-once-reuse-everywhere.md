The lead saw the dashboard and immediately pointed at the SLA block: "I want that on the escalation console too." Easy — copy the components, paste them on the other page, done by lunch. And by Friday the two copies disagree, because Tuesday's tweak landed on one and not the other. What you want isn't a copy. It's one definition with many instances.

> **Predict first:** what's the actual difference between pasting the same card onto two pages and *reusing* it?

## 1 · One definition, many instances

A **Widget** is a reusable A2UI component tree owned by the app — not by a flow, and not by any single page. Define the SLA card once in the app's **Widgets** workspace, place instances of it wherever they're needed, and every instance follows the definition.

@WidgetsOverview

That's the details view of the **Support Health Card** widget. The left half renders the live **Widget Preview** — "Support health, 94%, On target, median first response 4m 18s" — and the right half is its contract with the rest of the app: name, description ("a reusable SLA and response-time summary for support pages"), tags, a detailed description telling other builders where to use it, and one **Configurable Property**: *Card title*, "heading shown above the support metrics." Keep that property in mind. It's the whole trick.

## 2 · Build it like a page, keep it small

**Open Builder** drops you into the same three-part workspace as the Page Builder — components/hierarchy, canvas, Inspector — so there's nothing new to learn there.

@WidgetBuilder

In the builder, the **Hierarchy** tab spells the card out completely: a `card` container holding a `row`, a `progress`, and a `text`. Four components. That's not laziness — that's the design goal. A widget should do one reusable job, render something meaningful before any data arrives, and use theme tokens (`bg-card`, `text-muted-foreground`) so it reads in both light and dark mode. Test it narrow, long, and empty.

## 3 · Make instances differ — without forking

An **exposed property** maps a friendly, instance-facing option to one internal component property. *Card title* → the `content` of the heading `text`. Now the dashboard's instance can say "Queue health" while the console's says "Escalation health," and neither touched the definition. Expose what a page author genuinely needs — a heading, an accent, a bound value — and keep internal layout private.

## 4 · Let the page own the behavior

A widget with a button faces a question: which flow does it run? Wrong answer: hardcode one — the widget would work in exactly one context. Instead, define a named event in **Settings → Events** — say `acknowledge` — and have the button trigger it with a `widget_event` action carrying that `actionId`. When the widget is placed, *the page* binds that event to a workflow. Same button, different behavior per page, and the widget stays independent of any specific flow.

Settings also holds **Versions**: creating one saves the widget and records a snapshot. **Patch** for a compatible correction, **Minor** for a compatible addition, **Major** when existing uses should be reviewed — like restructuring what the card displays.

**Watch out:** the runtime supports per-instance exposed values and action bindings, but the current Page Builder doesn't yet mount a per-instance inspector — dragging a widget onto a page creates an instance with empty overrides. Today you supply instance values through Dev Mode or FlowPilot. The model is per-instance; the visual editor is still catching up.

**Recap**

- Widgets are app-owned reusable component trees: one definition, many instances that follow it.
- Exposed properties let each instance customize chosen internals (like *Card title*) without forking the definition.
- Behavior stays out of the definition: widgets emit named events, pages bind them to workflows. Version with Patch / Minor / Major.
