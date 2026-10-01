Wire one tool and inspect its use. Use a new practice board with a **Chat Event**, a configured model and the synthetic data from the previous lesson. No external service is needed.

## Build the tool first

Create a Flow function named `lookup_service_status` with a string input `service` and string outputs `service` and `status`. Use two equality checks and branches: `payments` returns `degraded`, `search` returns `operational`, and every other value returns `unknown_service`. Return the requested service name unchanged. Test these three paths directly before registering the function.

The allowlist belongs inside the function. A later model prompt cannot bypass those branches.

## Assemble the agent

1. Feed the model into **Agent from Model**. Set **Max Iterations** to `3`.
2. Pass its Agent output into **Set Agent System Prompt**. Use the instruction below.
3. Pass that Agent output into **Register Function Tools** and add only `lookup_service_status` through the node's function references.
4. Feed the configured Agent and Chat Event's **History** into **Invoke Agent**. Connect the event's execution output to its Input.
5. Send Invoke Agent's Response to **Push Response** on its Done path, using the Chat Event context required by that node.

```text
Answer service-status questions using lookup_service_status.
If a requested service is unknown, say so. If a call fails, report unavailable.
Follow conditional lookup requests when they use the two supported services.
You cannot restart services or access credentials. Do not claim such an action.
```

Run the four fixtures from chat. For each, record requested service, actual tool calls, returned statuses and final answer. Inspect the run and tool results, not exact answer wording. Repair any unsupported claim before continuing.

Max Iterations limits tool iterations. It is not a currency budget or a wall-clock timeout; those need separate controls in the surrounding deployment. See the [agent node guide](https://docs.flow-like.com/topics/genai/agents/).
