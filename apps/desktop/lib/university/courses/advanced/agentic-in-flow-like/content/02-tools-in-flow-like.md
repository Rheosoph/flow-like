Time to give the assistant hands. In this lesson you'll wire a minimal agent in your own app — model, instructions, one tool — and learn to read a board the way Studio does. First, a bet: look at the board below and decide when the two orange nodes at the bottom run. If your answer involves their position on the canvas, Studio disagrees — and that disagreement matters the moment an agent starts pulling on wires you didn't draw.

## 1 · Read the wires, not the layout

@TypedConnections

Two wire types run through the support board. The white wires with diamond connectors are **execution**: they decide *when* a node runs, starting at the event and moving connection by connection. The dashed pink wires are **data**: they carry values — the request text into *Prepare Support Reply*, the reply into *Send Reply* — and trigger nothing. Below the main row sit **Customer Message** and **Format Generic Value**, joined only by a data wire: pure nodes with no execution pins at all. They evaluate when something downstream needs their value, wherever they happen to be parked. Placement is for humans; wires are for the engine.

## 2 · Find the agent nodes

@NodeCatalog

Right-click an empty spot on your board and the **Actions** catalog opens — in the screenshot it's mid-search for "mail", offering *Send Email*, *Parse Mailbox*, *Watch Inbox*. Type `agent` instead and you'll find the family you need. Wire this path in your own app:

1. Provide a model — a provider-specific node, or **Find Model** with preferences.
2. **Agent from Model** (`agent_from_model`) turns it into an agent.
3. **Set Agent System Prompt** (`agent_set_system_prompt`) installs the operating instructions.
4. **Register Function Tools** (`agent_register_function_tools`) attaches Flow functions as tools.
5. **Invoke Agent** (`agent_invoke`) runs one full exchange — or **Stream Invoke Agent** for progressive output.

The agent value flows through each of these nodes in order, so the invoke receives an agent that already carries its prompt and tools. Streaming changes presentation, not truth: emit stage names and confirmed tool status, and never let an early fragment of fluent text become the run's final state.

## 3 · Pick the model deliberately

@ModelCatalog

**Explore Models** shows what your profile can reach — here, nine models with two already in the profile, filterable by capability tabs like chat & reasoning, speech, and embeddings. Each card states context size and whether the model is hosted or on-device; the Mistral Medium card even advertises "reliable tool calling", which is exactly what a tool-using agent needs. A chat model isn't automatically an embedding model — that distinction returns with force in the RAG module. And credentials never belong in boards, prompts, or logs; providers are configured in the profile.

## 4 · Design one bounded tool

Your assistant's first tool reads runbooks. A teammate proposes:

```text
run_query(query: string) -> unknown
```

Reject it. Nothing says which data it can touch, how much comes back, or whether arbitrary syntax rides along. Compare:

```text
search_runbooks(query: string, section?: string, limit: integer <= 10)
  -> { matches: [{source_id, section, excerpt}], truncated: boolean }
```

The implementation derives tenant scope from the trusted run context — not from a model-supplied argument — clamps the limit, and distinguishes "no matches" from "failed". When you later add writes, split intent from commit: a `preview_...` tool that returns a diff and an operation key, and a `commit_...` tool that rechecks authorization before changing anything. Registration is a grant of capability: start with one read tool, test it with valid, empty, malformed, and out-of-scope requests, and add a second tool only when your evaluation set proves you need it.

> **Watch out:** a tool that accepts `tenant_id` from the model is an open door with a polite sign on it. Trusted scope comes from run context, and the tool rejects anything outside it.

## Recap

- Execution wires decide when; data wires carry what; pure nodes evaluate on demand.
- The agent path is model → Agent from Model → system prompt → register tools → invoke.
- Tools are contracts: typed, bounded, and scoped from trusted context.
