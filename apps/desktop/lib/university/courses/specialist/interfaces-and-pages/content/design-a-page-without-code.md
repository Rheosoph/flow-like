Time to build the thing the support lead actually asked for: "open tickets, response times, SLA risk — one glance." That's the Support Operations Dashboard, and you'll build it without writing a line of interface code.

> **Predict first:** the dashboard shows "Open tickets: 24." Where should that number live so it stays current — typed into the page, or supplied by the flow?

## 1 · Create the page from its flow

A Page belongs to the Flow that powers it, so that's where it's born: open the Customer Support Automation flow, open its **Pages** panel, select **New**, enter a name, and select **Create Page**. Flow-Like drops you straight into the visual builder. A flow can own several pages — intake form and dashboard side by side — and each stays linked to the behavior behind it.

## 2 · Three panels, one canvas

@PageBuilder

That's the dashboard mid-build, and the whole builder in one frame. On the left, the **Components** palette (Row, Column, Grid, Scroll Area … Text, Image, Badge) with a **Hierarchy** tab beside it for inspecting the tree. In the middle, the **Canvas** renders the real thing: "Support operations" with a *Live · updated now* badge, stat cards for **Open tickets** (24, six need attention), **First response** (4m 18s), and **Within SLA** (94%), then a priority queue and automation coverage below. On the right, the **Inspector** shows the selected component — the Open tickets card, with its ID `open-tickets-card`, its type `card`, its Title, Description, and `bordered` variant under the Props / Style / Canvas / Actions tabs.

Build layout-first: start with a `column`, `row`, or `grid`, then place display and interactive components inside. Selecting a component on the canvas pops a small toolbar over it and fills the Inspector. The toolbar up top gives you copy, cut, paste, and delete, plus **Dev Mode**, manual save, and **Preview**. And relax about saving — page changes are also saved automatically after you stop editing.

## 3 · Literals and bindings

Now, that prediction. In the Inspector, a property can hold a **literal** — a fixed value like the title "Open tickets" — or a **binding**, which reads from Page data. The card's title is presentation; make it a literal. The 24 is a fact about the world; bind it, and let the flow keep it fresh. That split is the difference between a dashboard and a screenshot of one.

Interactive components carry one more Inspector section: **actions**, which trigger a selected Flow Event — lesson 6's territory. Navigation to another route gets its own dedicated actions, separate from workflow triggers.

## 4 · Page Settings

Select **Settings** in the Page header for the page-level knobs:

- **General** — name, description, ID, and version.
- **Behavior** — Flow events to run on load, on unload, or at an interval. A cached page can even show its last rendered state while the load event refreshes it: the dashboard appears instantly, then updates.
- **Layout** — layout type, background, spacing, and custom canvas styling.
- **SEO** — browser and social metadata.

That interval option is how "refresh every minute" belongs to the page itself — no scheduler required.

Try it in one of your own apps:

1. Open a flow → **Pages** → **New**, and name the page anything.
2. Add a `column`, then a heading `text` and a `card` inside it.
3. Change the card's title in the Inspector, then select **Preview**.

**Watch out:** style with semantic theme classes — `bg-card`, `text-foreground`, `text-muted-foreground` — instead of fixed colors. Your dashboard has to survive both light and dark mode, and the tokens do that for free.

**Recap**

- Pages are created from their flow (**Pages → New**) and edited in a three-panel builder: components/hierarchy, canvas, Inspector.
- Literals for fixed presentation, bindings for values the flow owns.
- Page Settings → Behavior runs flow events on load, unload, or an interval — that's your live refresh.
