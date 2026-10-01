Prerequisites: a bot you control, a permitted private test chat and a supported delivery path. Do not use a production channel for this exercise.

1. Build a flow with **Chat Event** and a fixed synthetic response, `Practice status: ready`, using the provider's Send Message node or the configured chat response path.
2. In Events, select Discord or Telegram for that node. Enter the test bot configuration and restrict its allowed chat/channel.
3. Choose the available adapter for your deployment. Desktop polling/Gateway connections require the process to remain running. Hosted delivery depends on the hub's support and configuration; inspect those controls instead of assuming every hub hosts bots.
4. Invoke `/status` or the configured command from the allowed chat. Expect one response and one run.
5. Try an unrelated message that does not match the configured trigger. Confirm it does not cause an unintended reply. Keep other chats outside the allowlist.

Telegram polling and webhooks share one bot's delivery configuration. Do not switch modes casually while another worker uses the token. Hosted Discord interactions and Gateway messages can have different payloads; inspect the actual entry context before reusing field paths.

Record the delivery adapter, runtime location, trigger rule and response evidence. Stop or disable the test bot when finished. For an always-on deployment, test while the desktop is closed and check the supported server connection; the course quiz cannot establish service availability.
