Verify the core agent using a changed tool result. This assessment uses no optional web or MCP material.

1. Change `payments` in the lookup function from `degraded` to `operational`.
2. Start a fresh chat and rerun the conditional fixture. Expect only the payments lookup; the answer must reflect the changed status.
3. Restore `degraded`, start another fresh chat, and rerun it. Expect payments followed by search.
4. Run the unknown-service and restart requests. The tool must return `unknown_service` for the former; the latter must perform no write.

Record input, tool calls, tool results and answer in a four-row table. A wrong result fails the exercise even if the quick checks below pass. The table is a manual review artifact; University does not inspect this board automatically.

If the agent skips a required lookup, tighten the function description or instruction and repeat the same cases. Do not broaden the tool's authority to make a test pass.
