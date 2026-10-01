Build an assistant that chooses which service status to inspect, while its tool controls what it can read. You need to be able to create a Flow function and run a tool-capable model. Complete Models & Profiles and Thinking in Flows first if either is unfamiliar.

## Decide whether a choice is needed

A fixed sequence can read two services every time. An agent is useful when the second lookup depends on the first result or the request needs interpretation. The model chooses a tool and its arguments; the function validates those arguments and returns the result.

For this lab, use synthetic values. Download the cases:

@AgentCases

Write this contract in a board comment:

- Read only `payments` and `search`.
- Return `unknown_service` for another name.
- Register no restart, network, file or credential tool.
- Allow at most three tool iterations.
- Report a tool failure as unavailable. Never replace it with an invented status.

The first fixture asks for a conditional second lookup. The other fixtures exercise the boundary. Success means the answer matches the function's data and the run contains only permitted calls. A fluent answer alone does not establish either condition.

For document-backed answers, take the separate RAG course and use the [retrieval reference](https://docs.flow-like.com/topics/genai/rag/). This core does not require an index, web search or MCP server.
