Three requests land in the support team channel. Marketing wants a Discord presence for the community. Ops wants the reconciliation job on the company server, hourly. A power user wants a desktop shortcut that opens triage. You have one flow with a Simple Event node.

> **Predict first:** Which of the three can you build on that node today — Discord, the hosted hourly job, the desktop shortcut? Lock in your guesses.

Every Event decision passes two independent filters: **what the entry node supports**, then **where the sink can run**. Apply them in order and impossible configurations reject themselves before they waste your afternoon.

## 1 · Filter one: the node decides the menu

The Create Event dialog reads your selected entry node and offers only the types that node supports:

| Flow event node | Available Event types                                |
| --------------- | ---------------------------------------------------- |
| Chat Event      | Chat UI, Discord, Telegram                           |
| Mail Event      | Email                                                |
| Generic Event   | Generic Form, API, Deeplink                          |
| Simple Event    | Quick Action, API, Cron, Daemon, Deeplink, REST, MCP |

Score your predictions. Discord sits in the **Chat Event** row — your Simple Event flow fails filter one, and no amount of configuration changes that. The hourly job (Cron) and the desktop shortcut (Deeplink) both live in the Simple Event row: still alive.

Several Event records may target the same node. The triage node could carry a Quick Action *and* a Deeplink — two doors, one contract — as long as each is intentional.

## 2 · Filter two: where the sink can run

In the current availability model:

- **API and Cron** run locally or remotely.
- **Daemon, Deeplink, Discord, Telegram, and Email** are local.
- **REST and MCP** are remote.
- **Quick Action, Chat UI, Generic Form, and Page targets** are invoked through the app interface itself.

Rows in this matrix will shift as hosting expands, so learn the reasoning, not the list: *a sink runs where its connection and runtime live*. A Daemon supervises a long-running local process. A Deeplink is a desktop invocation path. Discord and Telegram hold persistent service connections from a local environment. REST and MCP are hosted service surfaces.

When the editor doesn't offer Remote for a type, that's filter two talking. It's not a bug and not a missing permission — it's the sink telling you where it can exist. Your move is to re-check the caller's needs against location, not to fight the dropdown.

Prediction two, confirmed: the hosted hourly reconciliation is Simple Event → Cron → Remote. Passes both filters.

## 3 · API, REST, MCP: siblings, not synonyms

Three types sound alike and get confused constantly:

- **API** exposes *one* configured HTTP endpoint — method, path, exposure, size limits, error codes — and runs locally or remotely.
- **REST** exposes a *multi-endpoint*, authenticated service surface. Remote only.
- **MCP** exposes a Model Context Protocol server for tool-capable clients. Remote only.

Need one inbound webhook? API. Need a real service with several authenticated routes? REST. Want agents and IDEs to call your flows as tools? MCP.

## 4 · Credentials live where the run lives

An interactive local run can stop and ask you for a missing value. An unattended remote run at 3 a.m. cannot — and locally stored **Secret** runtime variables are deliberately excluded from remote execution payloads. The moment reconciliation moves to the server, the token saved on your laptop stops traveling with it. Provision a server-side credential and test the remote path before anyone depends on it.

Location is never just a performance choice. It decides which storage, network, service connections, and credentials your flow can reach.

> **Watch out:** "Board tests pass, Event fails" usually means the sink environment was never tested. A local board run proves logic, not location.

## Recap

- Filter one: the entry node decides which Event types exist at all.
- Filter two: the sink decides where it can run — and the editor enforces it.
- Credentials, connections, and storage all follow the execution location.
