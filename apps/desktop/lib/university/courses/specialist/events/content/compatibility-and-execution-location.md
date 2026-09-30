An Event must match both its entry node and an available execution environment. Check these independently.

| Input needed | Entry node | Examples |
| --- | --- | --- |
| No payload | Simple Event | Quick Action, Cron, API, REST, MCP |
| Named fields | Generic Event | Generic Form, API, Deeplink |
| Conversation context | Chat Event | Chat UI, Discord, Telegram, Teams |
| IMAP mail context | Mail Event | Email |
| Mail at a generated address | Inbound Email | Inbound Email |
| Device enters or leaves a region | Location Event | Geolocation |

This table is a selection aid. The Create Event dialog is the current list for the selected node and deployment.

**Local** execution uses the desktop environment. **Remote** execution uses the configured server. Availability also depends on the hub's supported sinks. Discord and Telegram have local adapters and can have hosted delivery on a supporting hub; their static local defaults do not describe every deployment. Teams and generated-address Inbound Email require server support. Geolocation monitoring needs the device even when its Event invokes a remote flow.

## Inspect your environment

Open Practice action for editing. Inspect its execution setting without changing it. Then inspect the types offered for a Generic Event in a scratch flow. Note which runtime choices are actually available.

For each planned Event, write three facts: the input it receives, the environment that receives the trigger, and where the flow executes. A local device being asleep can stop trigger delivery even if remote execution is configured.

Credentials must exist in the execution environment. Laptop Secret runtime values are excluded from remote execution payloads. Follow the deployment's supported credential setup before testing remotely; changing a dropdown does not provision a credential.
