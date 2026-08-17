A teammate discovers that the status-platform team ships an MCP server. Fourteen tools: `get_service_status`, `list_incidents`, `restart_service`, `purge_cache`, and ten more. "Just register the whole thing," they suggest. "The assistant only *needs* status anyway — the prompt will keep it polite." You already know the name of that bug from lesson 1. But what exactly changes the moment you connect an MCP server? That's this lesson.

## 1 · What connecting really adds

The Model Context Protocol lets a client discover and call another system's tools through a shared protocol. Convenient — and boundary-changing: the moment you connect a server, its tools, credentials, network reach, data access, and failure behavior all join your agent's effective permission surface, before a single call is made. MCP standardizes capability exchange; it doesn't standardize trust. A tool description saying "read-only" is a label, not an audit.

Flow-Like speaks MCP in both directions.

**Consume:** **Register MCP Tools** adds tools from an MCP server to your agent, next to your own function tools. **Register Remote MCP Tools** does the same for a connected app's MCP Event, using a short-lived app-to-app token refreshed each run. Either way, inspect what the server offers and register only what the task needs.

**Expose:** **MCP Server Config** starts a server definition; **Register MCP Functions** exposes your Flow functions as tools; **Register MCP Resource** publishes a `FlowPath` resource; **Register MCP Prompt** adds a prompt template; **Register MCP Auth** defines authentication; **MCP Server** starts the composed server. Exposing a function relaxes none of its obligations — it still validates identity, scope, and arguments itself, because you no longer control the caller.

## 2 · Register the subset, review the boundary

The answer to your teammate: one tool, `get_service_status`, registered after a short review. For any server you consume, record who owns it, how it authenticates and what that credential can reach, what each needed tool's argument and result schemas are, whether each operation reads or writes, and how it behaves on timeout and partial failure. Then constrain broad arguments — URLs, paths, filters — in your own workflow, validate results before they feed the model or another tool, and set per-call and overall timeouts in the Flow, not in hope.

Two failure patterns deserve special fear. First, *catalog drift*: the server adds tools, and they appear in discovery — registration must not automatically follow. Second, *the outage lie*: the MCP server goes down, and the agent, unable to check status, invents one. Degraded capability must surface as degraded capability — "status unavailable, answering from runbooks only" — never as a confident guess.

## 3 · The remediation question

Could the assistant one day call `restart_service`? Maybe — as a separate, explicitly registered write tool, behind a confirmation step, with an idempotent operation key and its own audit line. That's a different release with its own review. The version you're shipping draws the line at reads, and the *absence* of registered write tools is what makes the line real — not the sentence in the prompt asking the model to be careful.

> **Watch out:** registering a whole server because one tool is useful hands the model a menu you never reviewed — and tool-selection ambiguity grows with every redundant entry.

## Recap

- Connecting a server merges its capabilities and credentials into your agent's permission boundary.
- Consume minimally: inspect, register the task subset, validate results, plan for outage.
- Exposing your functions over MCP keeps every one of their own auth obligations intact.
