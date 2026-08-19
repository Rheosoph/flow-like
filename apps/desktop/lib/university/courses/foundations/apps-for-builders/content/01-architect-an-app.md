Your support team is drowning. Every morning brings forty new tickets, half of them variations of "where is my refund?", and the person answering them keeps a policy PDF open in a second window to copy-paste from. Over this course you'll architect a fix: the **Customer Support Copilot**, an app that drafts grounded replies, keeps a human in control, and records every case. Decision by decision, lesson by lesson — and in the next two minutes, you'll create its shell.

## 1 · Create the shell (do it now)

Open your Library and click **Create Flow**. Despite the dialog's title, it creates a whole project — its confirm button even says **Create Project**.

@CreateAppDialog

Two fields, two durable decisions. **Project Name**: name the outcome, not the experiment. `Customer Support Copilot` will still make sense in a year; `API Test 3` won't. **Connectivity**: *Online — Sync with cloud* needs a login; *Offline — Local only* stays on this machine. In the screenshot, Offline is selected — fine for a first draft. The next lesson gives you the real decision rule, and moving online later is a supported (if deliberate) step.

Click **Create Project**. That's it — you own an App. Everything else in this course happens inside this boundary.

## 2 · The App is the hub

So what did you just create? Not a Flow. A workspace that will *contain* Flows, plus everything they need to become a product.

@AppArchitecture

Look at the geometry: the App — Customer Support Copilot, "a project boundary for everything needed to build and ship an experience" — sits at the center, and five concerns hang off it. **Flows**: typed visual workflow logic. **Experiences**: Events, Pages, routes, and chat. **Data**: Storage and Data Studio. **Reuse**: widgets and Flow templates. **Delivery**: team access, releases, and sharing.

The division of labor is strict. Inside a **Flow** lives executable behavior: event nodes, branches, provider calls — the logic that turns a customer message into a draft reply. At the **App** level live the things that behavior needs to reach people and survive releases: the `/chat` route, the shared policy files, the case table, the team's roles, the version you actually shipped.

A quick test when you're unsure where something belongs: *would it survive a rewrite of the graph?* The route, the data, and the permissions all would. A Branch node wouldn't.

## 3 · Split Flows by trigger, not by size

One App holds any number of Flows — so when does the Copilot need a second one?

Say the team also wants a nightly summary of resolved cases. That's a genuinely separate Flow: its own trigger (a schedule instead of a chat message), its own lifecycle, its own failure story. Give it a name that states its responsibility.

What does *not* justify a split: a crowded canvas. When the reply-drafting graph grows, fold related nodes into a layer or a function inside the same Flow. Splitting one process across two Flows because it looks big buys you coordination problems, not clarity — connected logic belongs on one board.

> **Watch out:** don't add production credentials or real customer data while you're still sketching architecture. Empty shells are cheap; leaked keys aren't.

Before you place a single node, jot five lines about your App: who invokes it and through which surface, which Flows exist and why each earns its trigger, which files and tables it needs, which values vary by environment, and what evidence proves a run succeeded. Lessons 2 through 5 walk exactly that list.

## Recap

- An App is the product boundary; Flows inside it hold the executable graphs.
- Routes, storage, data, roles, and releases live at the App level — they outlast any graph rewrite.
- Split Flows when triggers and lifecycles differ, never because the canvas looks crowded.
