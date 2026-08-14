Forty-one unread support requests this morning, and the person who used to triage them just left. Here's the plan: over this course, you and **FlowPilot** — Flow-Like's built-in AI assistant — co-build a support-inbox triage app. It listens for requests, drafts replies with AI, and waits for a human before anything reaches a customer. FlowPilot does most of the wiring. You do the directing and the reviewing, because FlowPilot is best treated like a sharp junior colleague: fast, well-read, genuinely useful — and still someone whose work you check before it ships.

By the end of this lesson you'll know what FlowPilot can build, what it can actually see, and which model to give it.

## 1 · Say hello

@LandingPage

This is home. The glowing bubble in the middle of the screen — **Ask FlowPilot** — starts a conversation, and the chips floating around it ("Create a new app", "What can I build with Flow-Like?", "Show me the package store") are ready-made prompts. FlowPilot also has its own entry in the left sidebar, right under Home.

Click the bubble and send:

```text
Explain what you can access in Flow-Like. Do not change anything.
```

That message is deliberately read-only, so nothing in your workspace moves. FlowPilot answers with what it can currently see and do. Two minutes in, and you've held your first stand-up with your new colleague.

## 2 · The capability map

FlowPilot covers four areas and coordinates all of them from one conversation:

- **Workflows.** Explain the open board without touching it. Add, connect, configure, move, and remove nodes. Find the right nodes in the live catalog. Generate and repair FlowScript against the current board, and use run logs to investigate failures.
- **Interfaces.** Create a new page, build reusable widgets, or modify whatever you have open in the page or widget builder. Generated UI appears as a preview you accept or dismiss.
- **Data.** From the global assistant, delegate to Data Studio: inspect databases and tables, edit ontologies, write SQL or Cypher, run analytics, and present results as inline charts.
- **The platform.** Create an app and coordinate its workflow, UI, data, and Events. Navigate to Flow-Like views, open an app's page or chat inline in the conversation, and search public web sources when your request needs current information.

For the triage app, that means one assistant can plant the board, draft the team dashboard, model the ticket data, and wire the entry points — while you steer.

## 3 · What it can see

FlowPilot receives the relevant *visible context*, not your entire workspace: the open board, the nodes or components you've selected, the active Data Studio selection, and run context. Attached images travel as visual context when your model supports vision. Other attached files can be forwarded to a compatible app chat — FlowPilot doesn't read arbitrary non-image files itself.

Treat that as your steering wheel. What you open and select is how you point.

## 4 · Give it a capable brain

The provider/model picker offers two families. **Profile/Bits** uses a model configured in your Flow-Like profile and works in both the browser and the desktop app. **GitHub Copilot**, **Claude Code**, and **Codex** reuse a signed-in CLI on your computer and are available in the desktop app only.

The free profile model is genuinely useful for questions and small changes. It will probably not create a complete app reliably. When you ask for something big — say, an inbox triage app — pick a higher-tier model in your profile or one of the external coding agents.

Quick recap:

- FlowPilot builds workflows, interfaces, and data work from one conversation — and you review everything it makes.
- It sees visible context: the open board, your selections, run context. Not your whole machine.
- Free model for questions and small edits; a capable model for real builds.
