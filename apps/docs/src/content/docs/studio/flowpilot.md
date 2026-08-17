---
title: FlowPilot
description: Build, inspect, run, and improve Flow-Like apps with the built-in AI assistant
sidebar:
  order: 15
---

**FlowPilot** is Flow-Like's built-in AI assistant. It can answer questions about Flow-Like, work on the app or editor you currently have open, and coordinate workflow, interface, and data tasks from one conversation.

FlowPilot is available in the web and desktop apps. The tools it can use depend on your current context and the selected model provider.

:::caution[Choose a capable model]
The free profile model is useful for questions and small changes, but will probably not be enough to create a complete app reliably. For larger builds, select a higher-tier model in your Flow-Like profile or use **GitHub Copilot**, **Claude Code**, or **Codex** in the desktop app.
:::

## What FlowPilot can do

### Build and explain workflows

FlowPilot can:

- Explain the current board, selected nodes, and connections without changing them
- Add, connect, configure, move, and remove workflow nodes
- Find appropriate nodes and declarations in the live catalog
- Generate and repair FlowScript against the current board
- Use run context and execution logs to investigate failures
- Run safe verification steps and inspect their logs when runtime tools are available

Board edits are compiled and validated before they are offered for application. The **FlowScript** workspace shows the generated source, status, and compiler diagnostics.

### Design pages and widgets

FlowPilot can create a new A2UI page, build reusable widgets, or modify the interface currently open in the page or widget builder. It understands the existing component tree and selected components, and can generate responsive layouts, data bindings, theme-aware styling, and custom canvas settings.

Generated UI is shown as a preview before it is applied. See [FlowPilot UI generation](/reference/flowpilot-ui/) for UI-specific examples.

### Work with app data

From the global assistant, FlowPilot can delegate data work to Data Studio. This includes:

- Inspecting and managing databases and tables
- Creating and editing ontologies and overlays
- Writing and running SQL or Cypher queries
- Exploring paths, neighbors, and subgraphs
- Running analytics and presenting inline charts
- Inspecting or executing ontology actions

For normal app use, FlowPilot first inspects the app's active configured Events and prefers the best matching chat, page, REST/API, MCP, or other headless interface. It delegates directly to Data Studio only when you explicitly need raw schema/table/ontology/SQL/DataFusion access, no configured Event covers the request, or an Event reports that it cannot provide the required result. A failed or declined Event is not silently bypassed through the underlying data.

When Data Studio is open, FlowPilot receives the current app, overlay, and selected-table context. Those IDs resolve references such as "this overlay" without overriding Event-first routing. Direct data mutations use the current approval mode.

### Operate Flow-Like and use your apps

The global FlowPilot can also:

- Create an app and coordinate its workflow, UI, data, and Events
- Navigate to supported Flow-Like views
- Create or update app Events and page-load behavior
- Open an app page or app chat inline in the conversation
- Call chat and headless app interfaces on your behalf
- Pass relevant attached files to an app chat
- Search and read public web sources when your request needs current external information

In normal request-and-answer use, configured app Events are the primary interface. Direct datasource queries remain available as a deliberate fallback; app-building data work can still go directly to the Data Studio specialist.

## Where to use FlowPilot

### Global FlowPilot

Open the full FlowPilot chat or use its docked assistant. This is the platform-level assistant: it can create and navigate apps, use app interfaces, and route work to the workflow, UI, or Data Studio specialist.

If a board or Data Studio view is open, the global assistant receives that context. Requests such as "explain this workflow" or "query this overlay" therefore target the visible item without requiring you to copy its ID.

### Board editor

Open FlowPilot inside a board when you want to focus on workflow logic. The panel receives the current board, layer, selected nodes, and optional run-log context. Board changes are staged as commands or a compiled FlowScript review.

### Page and widget builders

Open FlowPilot inside a page or widget builder when you want to focus on the current A2UI surface. Selecting components before sending a request narrows the context. You can also attach a reference image or, where offered, include a screenshot of the current builder.

FlowPilot selects the appropriate scope from the surface you are using. The older **Frontend Agent**, **Backend Agent**, and **General Agent** choices are no longer separate user-facing modes.

## Review and apply changes

With **Auto mode off**, FlowPilot asks before side-effecting tool calls and keeps generated editor changes in a review step:

1. Describe the result you want.
2. FlowPilot inspects the current app, board, UI, or data context and uses the relevant specialist.
3. Review the generated FlowScript, board commands, or UI preview.
4. Select **Apply** to accept it or dismiss it to leave the editor unchanged.

A board review that no longer matches the live board is marked stale instead of being applied over newer work. Deleting existing board items always requires explicit confirmation.

With **Auto mode on**, FlowPilot runs tools and applies completed reviews without asking each time, including destructive actions. Deletion of existing board items is the exception and still requires confirmation. Use Auto mode only when you are comfortable reviewing the result after it has been applied.

## Context, attachments, and memory

- **Visible context:** FlowPilot receives the relevant open board, selected nodes or components, active Data Studio selection, and run context rather than the entire workspace indiscriminately.
- **Attachments:** Images can be sent as visual context when the selected model supports vision. In the global assistant, other attached files can be forwarded to a compatible app chat; FlowPilot does not read arbitrary non-image files itself.
- **History:** Conversations are saved locally in FlowPilot history. Long conversations are compacted so recent work and accepted results remain useful without sending an unbounded transcript.
- **Optional memory:** If your profile contains an embedding model, global FlowPilot can use profile-scoped memory. You can select the memory model and review or delete saved memories from the chat toolbar.

## Model providers

The provider/model picker shows the models and reasoning levels available to the selected backend:

- **Profile/Bits** uses a model configured in your Flow-Like profile and works in both the browser and desktop app.
- **GitHub Copilot**, **Claude Code**, and **Codex** use a signed-in CLI on your computer and are available in the desktop app only.

Model availability depends on the provider account, subscription, and organization policy. Installation, sign-in, CLI lookup paths, the temporary MCP connection, and macOS permission troubleshooting are documented in the [external coding agent setup guide](/studio/flowpilot-external-agents/).

## Prompt examples

Try giving FlowPilot an outcome and enough context to verify it:

- `Explain this workflow and point out where errors are handled. Do not change it.`
- `Build a webhook workflow that validates the payload, stores it, and returns a useful error response.`
- `Create a responsive customer dashboard and wire its table and chart to the workflow.`
- `Why did this run fail? Use the attached log context and suggest a fix.`
- `In this ontology, show orders connected to customers with overdue invoices as a chart.`
- `Open the support app chat here so I can talk to it.`

For a complete app, describe the interface, behavior, data, and entry point together. FlowPilot can coordinate those parts, but generated work should still be reviewed and tested with representative data before production use.

## Related guides

- [External coding agents in FlowPilot](/studio/flowpilot-external-agents/)
- [FlowPilot UI generation](/reference/flowpilot-ui/)
- [Widget Builder](/reference/widget-builder/)
- [Data Studio](/apps/data-studio/)
- [App Events](/apps/events/)
