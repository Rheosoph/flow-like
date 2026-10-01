This optional class replaces the local status function with one tool from an MCP server you control. MCP lets clients discover and call tools through a shared protocol. You need a reachable test server exposing a harmless status tool and a second tool that you will leave disabled.

## Register one capability

1. Keep the agent from the core lab and remove its function-tool registration for this test.
2. Add **Register MCP Tools** between the system-prompt node and Invoke Agent.
3. Set the server URI and select **Manual** mode. Let discovery populate the individual tool pins.
4. Enable only the harmless status tool. Inspect its input and output schema before invoking it.
5. Ask for a supported status and inspect the run for the selected tool call.

Automatic mode registers all available tools. Manual mode makes this exercise's intended subset explicit. A tool's name or description does not prove its effects; use a server whose implementation and credentials you can review.

Stop the test server and ask again. Expected result: a reported unavailable capability, with no invented status. Restart it, expose another test tool and confirm that your saved selection still grants only the intended capability.

For another Flow-Like app, **Register Remote MCP Tools** uses a configured app connection and that app's MCP Event. The Connected Apps course covers connection roles and revocation. To publish your own tools, follow the [MCP server guide](https://docs.flow-like.com/topics/genai/agents/); publishing is a separate exercise from consuming this server.
