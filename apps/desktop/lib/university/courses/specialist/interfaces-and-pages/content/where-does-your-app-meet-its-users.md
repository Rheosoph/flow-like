The support team's verdict on Customer Support Automation was short: "Love it. Where do we click?" The flow listens for requests, drafts replies with AI, routes them past a human reviewer, and sends — and the team has seen none of it, because right now the one place it's visible is Studio. They are never going to open Studio. Over the next seven lessons you'll ship them a **support console** instead: an operations dashboard, a reusable health card, a branded chat assistant, and a one-click triage action.

> **Predict first:** a dashboard, a chat window, and a reusable card sound like three different UI technologies. How many does Flow-Like actually use?

One.

## 1 · One model behind every surface

Every interface in Flow-Like — full-page dashboard, reusable card, chat window — is an **A2UI surface**: structured data that describes components, not hand-written application markup. Because the interface is data, you can build it visually, edit its JSON directly, or ask FlowPilot to generate and refine it, and the renderer draws the same result either way.

@A2uiArchitecture

The infographic sums up the whole pipeline. On the left, two authoring paths — the Visual Builder ("drag, arrange, inspect") and FlowPilot ("generate and revise") — feed one declarative surface in the middle: a flat `components[]` graph connected by IDs, plus a `dataModel[]`, canvas settings, actions, and bindings. On the right, the A2UI renderer maps that data onto allowlisted React components and serves both **Pages** (full experiences) and **Widgets** (reusable blocks). Along the bottom, user actions travel to an app event or workflow, which streams A2UI updates back to the surface.

Two details in that picture do a lot of work later:

- **Flat, ID-connected structure.** A surface has one root component; a layout component doesn't nest its children, it lists their IDs. That's what lets the builder move or replace one section without touching the rest.
- **The loop at the bottom.** Interfaces aren't static: a click becomes a workflow run, and the workflow can push A2UI updates back. Lesson 6 is entirely about that loop.

## 2 · The surfaces you'll ship

Your console needs three kinds of surface, and Flow-Like has a home for each:

- **Pages** are full-screen interfaces — dashboards, forms, tools. A Page belongs to a **Flow**, so interface and behavior stay together.
- **Chat UI** exposes a flow as a conversation. Nothing to lay out — you configure and brand it (lesson 5).
- **Widgets** are reusable component blocks owned by the app, placed onto any Page (lesson 4).

To see every Page across an app, open the app's **Events** workspace and select the **Pages** tab:

@PagesOverview

That's the Customer Support Copilot app you'll work in, and its Pages tab already names this course's targets: a **Support Operations Dashboard** ("live queue health, response quality, and SLA risk"), a **Customer Intake** page, and an **Escalation Console**. Notice the **Flow** button on each card — every page advertises the flow that powers it, and you can jump straight to that flow or delete the page from here.

Try it now in one of your own apps: open **Events**, switch to the **Pages** tab, and open any page — or note that the list is empty, which you'll fix in lesson 3. That's the whole geography of this course, seen in under two minutes.

## 3 · Three ways to author, one surface

You'll touch all three authoring paths in this course:

- **Visual Builder** — component catalog, hierarchy, live canvas, property Inspector.
- **FlowPilot** — describe the interface or the change you want; review the result on the canvas.
- **Dev Mode** — inspect or edit the underlying JSON for precise changes and debugging.

All three modify the same component model, so you can move between them freely in a single session — arrange visually, let FlowPilot restyle, then fine-tune one property in Dev Mode. There's a fourth author, too: a **flow can return A2UI updates at runtime**. That's how the dashboard's numbers will stay honest while the team watches.

**Watch out:** Dev Mode is not "export to code." There's no generated React project drifting out of sync somewhere — the JSON you see *is* the interface, and the canvas re-renders it the moment it changes.

**Recap**

- Every surface — Page, Widget, chat — renders from one declarative A2UI model: a root plus ID-connected components.
- Pages belong to Flows; the app-wide list lives in **Events → Pages**.
- Visual Builder, FlowPilot, and Dev Mode edit the same surface; flows can update it at runtime.
