---
title: AI Agents
description: Build controlled, tool-using model workflows in Flow-Like
sidebar:
  order: 5
---

An agent is a configured model with instructions and a bounded set of tools. It can choose tools and combine their results, while the surrounding Flow-Like workflow controls permissions, iteration limits, confirmation, and delivery.

## When an agent helps

| Use an agent | Prefer a fixed workflow |
|--------------|-------------------------|
| The task requires choosing among several approved tools | The steps and order are already known |
| The user request needs interpretation before execution | The input maps directly to one operation |
| Results require a small amount of adaptive planning | The operation is high-volume or latency-sensitive |
| The agent must inspect intermediate results | Exact repeatability is the primary requirement |

Start with a deterministic board when it is sufficient. Agent autonomy adds model variability, latency, cost, and a larger permission surface.

## Build an agent

### Start from a model

[Agent from Model](/nodes/ai/agents/builder/agent-from-model/) creates an agent from a configured generative model. [Simple Agent](/nodes/ai/agents/simple-agent/) is available for a simpler setup.

Choose a model that supports the required tool behavior and context size. Model names and capabilities depend on the configured provider; do not hard-code one provider's current model list into a reusable board.

### Set the operating instructions

[Set Agent System Prompt](/nodes/ai/agents/builder/agent-set-system-prompt/) defines:

- the role and task boundary;
- allowed tools and when to use them;
- required confirmation;
- source and data-access rules;
- the expected final response;
- no-result and failure behavior;
- a maximum level of acceptable uncertainty.

Prompts guide behavior, but they are not authorization. Tool implementations and the workflow must enforce access and validation.

### Register tools

| Tool source | Node | Use |
|-------------|------|-----|
| Flow-Like functions | [Register Function Tools](/nodes/ai/agents/builder/agent-register-function-tools/) | Typed, board-owned operations |
| Deferred Flow-Like functions | [Lazy Register Function Tools](/nodes/ai/agents/builder/agent-lazy-register-function-tools/) | Register function tools only when needed |
| MCP server | [Register MCP Tools](/nodes/ai/agents/builder/agent-register-mcp-tools/) | External tool protocol |
| DataFusion | [Add DataFusion](/nodes/ai/agents/builder/add-datafusion-to-agent/) | Governed SQL exploration |
| Memory | [Register Memory](/nodes/ai/agents/builder/agent-register-memory/) | Explicit agent memory capability |
| Planning aid | [Register Thinking Tool](/nodes/ai/agents/builder/agent-register-thinking/) | Deliberate internal planning support |

Register only the tools required for the task. Separate read operations from write operations and give them distinct names and descriptions.

### Invoke the agent

Use [Invoke Agent](/nodes/ai/agents/agent-invoke/) for a complete result or [Stream Invoke Agent](/nodes/ai/agents/agent-stream-invoke/) for progressive output.

The surrounding workflow should impose:

- a maximum number of tool turns;
- model and tool timeouts;
- output and context limits;
- a cancellation path;
- confirmation before destructive, costly, or externally visible actions;
- a clear final success or failure state.

## Design effective tools

A good agent tool:

- performs one bounded operation;
- has a typed, minimal input schema;
- validates every argument;
- returns a compact, structured result;
- distinguishes no result from failure;
- is idempotent when retries are possible;
- logs safe identifiers without exposing secrets;
- checks authorization independently of the model.

Prefer names such as `get_order_status`, `search_policy`, or `create_draft_invoice` over generic names such as `run_workflow` or `execute_action`.

### Read tools

Read tools can search, inspect, or calculate. They should still:

- enforce tenant and row-level scope;
- cap result counts;
- avoid returning unnecessary private fields;
- treat retrieved text as untrusted data.

### Write tools

Write tools should:

- expose the smallest useful action;
- support a dry-run or preview where practical;
- require confirmation for meaningful side effects;
- use stable idempotency keys;
- return the resulting object or operation identifier;
- make partial failure visible.

Do not let one tool accept an arbitrary URL, command, file path, or SQL statement unless that broad authority is truly required and separately controlled.

## Use MCP carefully

MCP can expose a large set of external tools. Before [Register MCP Tools](/nodes/ai/agents/builder/agent-register-mcp-tools/):

1. inspect the server and tool list;
2. review credentials and network reach;
3. select only the tools needed;
4. validate arguments and results in the surrounding workflow;
5. decide which calls need user confirmation;
6. define timeout and failure behavior.

An MCP server is part of the agent's effective permission boundary.

## Add data access

[Add DataFusion](/nodes/ai/agents/builder/add-datafusion-to-agent/) can expose a registered DataFusion session. Use read-only credentials and register only approved tables.

For analysis agents:

- let the agent list and describe tables before querying;
- enforce result limits;
- aggregate before returning data to model context;
- record the executed query;
- never rely on a prompt alone to prevent sensitive access.

See [AI-powered data analysis](/topics/datascience/ai-analysis/).

## Memory and state

Agent memory should be explicit about:

- what is stored;
- which user or tenant owns it;
- how long it is retained;
- whether the user can review or delete it;
- which facts are durable versus turn-specific.

Do not use conversation history as an unbounded database. Keep operational state in typed workflow or app storage and retrieve only what the current task needs.

## Progress and streaming

Stream user-facing progress, tool status, and final response content. Do not expose hidden reasoning or raw internal chain-of-thought. A useful progress event names the stage, not the model's private deliberation.

If a tool fails after partial output, the final state must say that the operation did not complete. Avoid optimistic text before a side effect is confirmed.

## Multi-agent orchestration

Multiple agents are justified only when they have genuinely different tools, data boundaries, or review roles. Use the parent workflow to:

- define each agent's responsibility;
- pass typed, minimal handoff data;
- limit recursive delegation;
- resolve conflicting results;
- keep one final authority for side effects.

Do not split a simple task into multiple agents only to imitate an organizational chart.

## Evaluate the agent

Build a test set that covers:

| Area | Cases |
|------|-------|
| Tool choice | correct tool, unnecessary tool, unavailable tool |
| Arguments | missing, malformed, adversarial, out-of-scope |
| Side effects | confirmation, duplicate delivery, partial failure |
| Retrieval | no result, conflicting sources, prompt injection |
| Control | iteration limit, timeout, cancellation |
| Response | factuality, uncertainty, source references, clear failure |

Review tool traces by failure category. A fluent final answer can hide a wrong tool call or an operation that never succeeded.

## Security checklist

- [ ] Tools and credentials use least privilege
- [ ] Authorization is enforced outside the prompt
- [ ] Tool arguments are validated
- [ ] Write actions require appropriate confirmation
- [ ] Iterations, time, and output are bounded
- [ ] Retrieved content is treated as untrusted
- [ ] Secrets and private fields are redacted from traces
- [ ] Retries are idempotent
- [ ] The final state reflects actual tool outcomes

## Next steps

- [Chat and conversations](/topics/genai/chat/)
- [RAG and knowledge bases](/topics/genai/rag/)
- [AI-powered data analysis](/topics/datascience/ai-analysis/)
- [API integrations](/topics/api-integrations/overview/)
