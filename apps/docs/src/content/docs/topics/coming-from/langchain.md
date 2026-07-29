---
title: For LangChain Users
description: Move LangChain workloads into Flow-Like or connect them through the SDK
sidebar:
  order: 2
---

LangChain and Flow-Like can meet in two different places:

1. Keep orchestration in LangChain and use Flow-Like's official chat-model and
   embedding adapters.
2. Rebuild the orchestration as a typed visual Flow and expose it through App
   Events, chat, pages, or an API.

The current Studio importer does not translate a LangChain graph. Choose the
path based on who should own orchestration, state, deployment, and debugging.

## Choose an integration path

| Keep LangChain code | Move orchestration into a Flow |
| --- | --- |
| Existing application already owns routing and lifecycle | Teammates should edit and inspect the logic visually |
| LangChain-specific components remain important | Typed node contracts should define the pipeline |
| Only model or embedding access should move | The workflow needs Flow-Like Events, Pages, Widgets, storage, or automation nodes |
| Existing tests and deployment stay in code | Run history and App-level configuration should live together |

You can use both approaches: a LangChain service can call a Flow-Like Event,
and a Flow can call an external API that is implemented with LangChain.

## Keep LangChain and use the SDK

The Python and Node.js SDKs include LangChain-compatible chat-model and
embedding wrappers. With the Python SDK, a chain can use a model configured in
Flow-Like:

```python
from langchain_core.output_parsers import StrOutputParser
from langchain_core.prompts import ChatPromptTemplate

chat_model = client.as_langchain_chat("your-model-bit-id")

chain = (
    ChatPromptTemplate.from_messages([
        ("system", "You are a helpful assistant."),
        ("human", "{input}"),
    ])
    | chat_model
    | StrOutputParser()
)

response = chain.invoke({"input": "What is Flow-Like?"})
```

The equivalent factory methods are documented for
[Python](/dev/sdks/python/) and [Node.js/TypeScript](/dev/sdks/nodejs/).
Authentication, model access, and data handling still follow the Flow-Like
backend and credential configuration used by that SDK client.

## Translate the mental model

| LangChain concept | Current Flow-Like concept |
| --- | --- |
| Runnable or chain | Connected nodes in a Flow |
| LCEL pipe | Typed data and execution wires |
| Chat model | Configured model plus [Invoke Model](/nodes/ai/generative/ai-generative-invoke/) |
| Prompt template | History/message nodes and, when needed, [Render Template](/nodes/utils/string/string-render-template/) |
| Agent | [Agent from Model](/nodes/ai/agents/builder/agent-from-model/) plus registered tools and [Invoke Agent](/nodes/ai/agents/agent-invoke/) |
| Tool | Typed Flow function registered with the agent |
| Conversation history | History output from [Chat Event](/nodes/events/events-chat/) |
| Session state | Chat local/global session values |
| Durable agent memory | [Register Memory](/nodes/ai/agents/builder/agent-register-memory/) or an explicit data store |
| Retriever | Vector, full-text, or hybrid database search |
| Vector store | Flow-Like database opened and populated by database nodes |
| Output parser | [AI Extractor](/nodes/ai/generative/llm-extractor/) with a JSON schema |
| Callback or trace | Flow run history and node logs |

The mapping is conceptual. A wire is not a serialized `Runnable`, and a board
variable is not automatically equivalent to a LangChain memory implementation.

## Chains become typed graph stages

An LCEL pipeline such as:

```python
chain = prompt | model | parser
result = chain.invoke({"topic": "AI"})
```

usually becomes these graph stages:

| Stage | Flow-Like implementation |
| --- | --- |
| Receive input | Event-node output pin |
| Build instructions and messages | Make or update chat history |
| Invoke | Configured model into Invoke Model |
| Validate output | AI Extractor, JSON parsing, or ordinary typed nodes |
| Return | Event-specific response or result node |

Connect execution wires only where ordering or side effects require them.
Connect data wires for the values each stage consumes.

For ordered fan-out use [Sequence](/nodes/control/control-sequence/). For
intentional concurrency use
[Parallel Execution](/nodes/control/control-par-execution/) or
[Parallel For Each](/nodes/control/control-par-for-each/), followed by
[Gather](/nodes/control/parallel/control-gather/) when all branches must finish.

## Prompts and chat history

Flow-Like model invocation is history-oriented. A typical chat Flow:

1. starts with Chat Event;
2. reads the event's current History output;
3. applies a system instruction with
   [Set System Message](/nodes/ai/generative/history/ai-generative-set-system-prompt-message/);
4. invokes the configured model;
5. returns a complete response or streams response chunks to chat.

Use [Push Message](/nodes/ai/generative/history/ai-generative-add-history-message/)
to add messages. Configure the Chat UI Event's history window deliberately, or
construct a smaller history before invocation when the Flow should use only a
subset. See [Chat and conversations](/topics/genai/chat/) for the complete
event, session, attachment, and streaming contract.

Do not copy prompt variables into a single opaque template when the values need
validation. Keep important inputs as typed pins and assemble the message only
after those inputs pass their checks.

## Agents and tools

A visual agent is assembled explicitly:

| Responsibility | Node |
| --- | --- |
| Create the agent from a configured model | [Agent from Model](/nodes/ai/agents/builder/agent-from-model/) |
| Set operating instructions | [Set Agent System Prompt](/nodes/ai/agents/builder/agent-set-system-prompt/) |
| Add Flow functions as tools | [Register Function Tools](/nodes/ai/agents/builder/agent-register-function-tools/) |
| Add MCP tools | [Register MCP Tools](/nodes/ai/agents/builder/agent-register-mcp-tools/) |
| Invoke once and return a complete result | [Invoke Agent](/nodes/ai/agents/agent-invoke/) |
| Stream the result | [Stream Invoke Agent](/nodes/ai/agents/agent-stream-invoke/) |

Flow functions should expose small typed operations, validate their arguments,
and enforce authorization outside the prompt. Keep side effects, retries, and
confirmation visible in the surrounding Flow.

## Memory is a lifecycle decision

LangChain's “memory” label can refer to several different lifecycles. Map the
requirement, not the class name:

| Required lifecycle | Flow-Like choice |
| --- | --- |
| Current conversation messages | Chat Event History |
| State for one chat | Local Session |
| User-level chat state | Global Session |
| Agent-managed persistent recall | Register Memory |
| Durable application records | Database or App Storage |
| Temporary graph state | Flow variable |

Limit history before model invocation, define retention for session and memory
data, and never use restored conversational state as proof of authorization.

## RAG becomes two Flows

Keep indexing and query execution separate.

### Indexing Flow

| Step | Current nodes or guide |
| --- | --- |
| Extract text and preserve provenance | [Document processing](/topics/document-processing/overview/) |
| Split text | [Chunk Text](/nodes/ai/preprocessing/chunk-text/) |
| Load the embedding model | [Load Embedding Model](/nodes/ai/embedding/load-model/) |
| Embed each chunk | [Embed Document](/nodes/ai/embedding/embed-document/) |
| Open and populate the index | [Open Database](/nodes/data/database/open-local-db/) and database write nodes |

### Query Flow

| Step | Current node |
| --- | --- |
| Embed the question | [Embed Query](/nodes/ai/embedding/embed-query/) |
| Semantic retrieval | [Vector Search](/nodes/data/database/search/vector-search-local-db/) |
| Exact-term retrieval | [Full-Text Search](/nodes/data/database/search/fts-search-local-db/) |
| Combined retrieval | [Hybrid Search](/nodes/data/database/search/hybrid-search-local-db/) |
| Generate from selected evidence | History nodes plus Invoke Model |

Retain source IDs and locations through retrieval so the final response can
cite its evidence. Apply access filters before retrieved content enters model
context. The full operating guidance is in
[RAG and knowledge bases](/topics/genai/rag/).

## Structured output

Use AI Extractor when the model must return a known shape. Its schema is JSON
Schema, for example:

```json
{
  "type": "object",
  "properties": {
    "name": { "type": "string" },
    "priority": {
      "type": "string",
      "enum": ["low", "medium", "high"]
    }
  },
  "required": ["name", "priority"]
}
```

Treat schema validation as one boundary, not proof that the extracted facts are
correct. Validate identifiers, permissions, ranges, and business rules before
using the result in a side effect.

## Observability and evaluation

[Run history](/studio/logging/) records executions and node logs. It is the
nearest Flow-Like inspection surface to callbacks or traces, but it is not a
drop-in replacement for every LangChain observability product.

During migration, test the layers independently:

- prompt and structured-output behavior;
- tool selection and tool arguments;
- retrieval rank and source propagation;
- history and session boundaries;
- timeout, cancellation, and partial failure;
- final response and externally visible side effects.

## Migration checklist

1. Decide whether LangChain or Flow-Like will own orchestration.
2. Inventory models, prompts, tools, retrievers, memory, and callbacks.
3. Define typed inputs and outputs for each target Flow function and Event.
4. Split RAG indexing from query execution.
5. Choose explicit storage for every memory lifecycle.
6. Move secrets into [Runtime Variables](/apps/runtime-variables/).
7. Rebuild one representative path and compare its outputs with the source.
8. Add App Events or chat only after the underlying Flow passes its tests.

## Next steps

- [SDK overview](/dev/sdks/overview/)
- [Models](/topics/genai/models/)
- [Chat and conversations](/topics/genai/chat/)
- [RAG and knowledge bases](/topics/genai/rag/)
- [AI agents](/topics/genai/agents/)
- [Extraction and structured output](/topics/genai/extraction/)
