---
title: Teams bots
description: Connect a Chat Event to Microsoft Teams, send replies and approval cards, and install the generated Teams app
sidebar:
  order: 2
---

A Teams bot lets people run your flow through direct messages, mentions, and
card responses in Microsoft Teams. Flow-Like hosts the messaging endpoint and
runs the workflow on the server, so your desktop can be closed.

## Choose who manages the bot

| Setup path | What you need | Who maintains the registration |
|------------|---------------|--------------------------------|
| **Flow-Like managed** | A server with managed bots enabled and your Teams tenant ID | Flow-Like provisions the Entra application and Azure Bot. |
| **Your bot in Teams** | Permission to register a bot in Teams Developer Portal and create its Entra identity | Your organization owns the registration and renews its secret. An Azure subscription is unnecessary. |
| **Your bot in Azure** | An Azure subscription and permission to manage the Azure Bot and Entra application | Your organization maintains the Azure resource, Teams channel, and secret. |

Azure supports OAuth and single sign-on configuration; **this Flow-Like
integration does not implement Microsoft user sign-in or SSO**. See Microsoft's
[bot registration comparison](https://microsoft.github.io/teams-sdk/cli/concepts/bot-locations/)
for the Microsoft-side capabilities of each option.

Basic messages and cards do not require delegated Microsoft Graph access.
Accessing a user's Microsoft 365 data needs a separate provider connection and
permissions; the Teams sender's identity does not grant them to the workflow.

## Connect and install

Start with an online Flow-Like app and a board that allows server execution.
You need permission to edit Events. Your Teams organization must allow custom
app uploads, or an administrator must install the app for you.

1. Add a [Chat Event](/nodes/events/events-chat/) node to your board and connect
   the workflow you want to run.
2. Create an app Event targeting that node. Choose **Remote** execution and
   **Teams Bot**, then save the Event.
3. Open its **Teams bot** settings. Choose a management path, enter a bot name,
   and enter the **Teams organization tenant ID**. This is the organization
   whose conversations may trigger the Event.
4. For **Flow-Like managed**, select **Create bot**. For a customer registration,
   complete the steps below, then select **Connect bot**.
5. Once the status is **Credentials verified**, select **Download Teams app
   (.zip)**. The file is named after the bot and contains the manifest and
   icons. Bot credentials stay on the server.
6. In Teams, select **Apps → Manage your apps → Upload an app → Upload a custom
   app**, choose the ZIP, and add it. If upload is unavailable, send the ZIP to
   your Teams administrator. See Microsoft's [upload instructions](https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/deploy-and-publish/apps-upload).
7. Activate the Flow-Like Event. Send the bot a direct message, or mention it in
   a team or group chat where it is installed. Check the Event's execution
   history and its reply.

### Connect your own registration

For **Your bot in Teams**, open [Teams Developer Portal](https://dev.teams.microsoft.com/)
and select **Tools → Bot management → New Bot**. Tenant policy may require an
administrator to create the application or service principal. Microsoft documents
the permissions in its [Developer Portal guide](https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/build-and-test/manage-your-apps-in-developer-portal).

For **Your bot in Azure**, [create an Azure Bot](https://learn.microsoft.com/en-us/azure/bot-service/abs-quickstart?view=azure-bot-service-4.0)
with app type **Single Tenant** and enable its **Microsoft Teams** channel.

For either path:

1. Copy the **Messaging endpoint** from Flow-Like into the bot registration.
   Each Event has its own endpoint. Installing a ZIP does not configure this
   Microsoft-side setting.
2. Enter the **Application (client) ID**, **App's home tenant ID**, and **Client
   secret value** in Flow-Like. Use the secret's value, not its identifier.
3. Select **Connect bot**. On later edits, leave the secret blank to retain it
   for the same application and home tenant.

Flow-Like requests a token with new credentials before it saves them. If
Microsoft rejects them, the saved bot keeps its previous credentials and status.
Editing only the name, description, organization tenant, or approvers keeps a
connected bot connected.

The home tenant owns the bot identity; the Teams organization tenant hosts its
users. If those differ, configure the Entra application's supported accounts as
**Accounts in any organizational directory**, while retaining **Single Tenant**
on the Azure Bot resource. Microsoft distinguishes these settings in its
[bot management guidance](https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/build-and-test/manage-your-apps-in-developer-portal).

## Reply and collect input

Use the Chat Event's **History** output with your existing chat workflow. Its
final **Push Response** text is sent to Teams. Streaming chunks and Flow-Like
chat widgets are not rendered as Teams messages.

For explicit replies, search the node catalog under **Events / Chat / Teams**:

| Node | Connect |
|------|---------|
| **Send Teams Message** | The Chat Event's **Local Session** or **Global Session** to **Session**, plus Markdown text |
| **Send Teams Card** | The same **Session**, plus an Adaptive Card JSON object |
| **Update Teams Message** | The same **Session**, the **Message ID** from a send node, and replacement text |

These nodes reply within the originating execution and conversation. The
current bot processes text messages; attachments are not passed into the flow.

Use [Single Choice](/nodes/events/chat/interaction/interaction-single-choice/),
[Multiple Choice](/nodes/events/chat/interaction/interaction-multiple-choice/), or
[Form](/nodes/events/chat/interaction/interaction-form/) for approvals and input
that continue the flow. Flow-Like renders an Adaptive Card and routes its answer
back to the waiting node. Manually supplied cards do not create waiting
interactions automatically.

By default, only the person who started the flow can answer. Under **Approvals
and actions**, you can instead list up to 100 allowed Entra user object IDs.
Responders must answer in the original conversation of the configured Teams
organization.

### People from other organizations

In a [shared channel](https://learn.microsoft.com/en-us/microsoftteams/platform/concepts/build-and-test/shared-channels),
people from other organizations can message the bot. Microsoft labels every
message with the tenant that hosts the channel, so Flow-Like accepts it when
that host tenant is the configured organization. The Chat Event's user ID is
`<organization tenant ID>:<Entra object ID>`: for an external person, the prefix
is the host organization's tenant, while the object ID comes from the person's
home tenant. Approver lists match object IDs, so add an external person's object
ID to let them answer. Flow-Like does not look up a sender's home tenant.

A reply sent with **Send Teams Message** is added to the conversation history
that the next message receives, like the final **Push Response** text.

**An Interaction keeps the execution running while it waits.** Set its timeout
within the executor's remaining lifetime and connect its **Timeout** output.
On Lambda, the invocation remains active and its execution limit still applies.
Saved card state does not support durable approval waits across worker exits.

## Operate the integration

1. Apply migration `20260928130000_teams_bots` using your deployment's migration
   process. PostgreSQL and Aurora DSQL have separate versions under
   `packages/api/prisma/migrations/` and `packages/api/prisma/migrations-dsql/`.
   They create `TeamsBotConnection` and `TeamsBotState`.
2. Enable `supported_sinks.teams` in the hub configuration. Without it, bot
   setup and the messaging endpoint answer `403`; disconnecting still works.
3. Store `TEAMS_PUBLIC_BASE_URL` as the public HTTPS API origin in the API's
   configured secret store. It falls back to `API_BASE_URL` when absent;
   `/api/v1` is appended when absent. Keep this address stable and allow
   Microsoft requests to reach the generated messaging endpoint.
4. Configure and retain the server's existing `SINK_TOKEN_ENCRYPTION_KEY`.
   Teams credentials and stored conversation/action state use this encryption
   key. Replacing it without migrating stored data prevents decryption.
5. Verify the execution backend and run reply channel, then test a message and
   an Interaction card in Teams. The current integration targets Microsoft's
   public cloud endpoints.

Conversation, delivery, and approval state expires on its own schedule.
Long-running API servers delete expired rows every 10 minutes. Serverless
deployments delete them in the scheduled `cache_cleanup` maintenance job.

The API reads its `TEAMS_*` settings through its existing secret store.
On AWS, they live under the API's `SECRET_PREFIX` in SSM Parameter Store, so
they do not consume Lambda environment space. Azure uses Key Vault, with
underscores in key names replaced by hyphens. GCP uses Secret Manager with
the configured prefix. Grant the API identity access to these values.

For Docker Compose, the environment provider reads the Teams variables from
the deployment's `.env` file. For Kubernetes, pass them to the API through
Helm's `api.env` or `api.envFrom`; keep the provisioning secret in a Kubernetes
Secret. Keep the existing encryption key when updating or restarting the API.

Cloud deployments also store `TEAMS_MANAGED_ENABLED` as `true` or `false`.
When it is `false`, the API skips provisioning-secret reads, so customer-owned
bots work without access to managed credentials. When it is `true`, all six
provisioning settings below are required. Manual environment setups can omit
this marker; the API then checks whether the six settings are present.

### Enable Flow-Like-managed provisioning

To make **Flow-Like managed** available, configure all six settings below.
Customer registrations only need the public URL and the existing server
configuration.

| Variable | Value |
|----------|-------|
| `TEAMS_MANAGED_TENANT_ID` | Entra tenant that owns the managed bot identities |
| `TEAMS_PROVISIONER_CLIENT_ID` | Provisioning application's client ID |
| `TEAMS_PROVISIONER_CLIENT_SECRET` | Provisioning application's secret value |
| `TEAMS_PROVISIONER_OBJECT_ID` | Provisioning service principal's object ID, assigned as owner of created identities |
| `TEAMS_AZURE_SUBSCRIPTION_ID` | Subscription containing the bot resources |
| `TEAMS_AZURE_RESOURCE_GROUP` | Existing resource group for those resources |

`TEAMS_MANAGED_MAX_BOTS_PER_APP` optionally limits how many Flow-Like-managed
bots one app can hold (1–10000, default `10`). Disconnected bots don't count.
Each app can also start at most 10 managed setups or renames per hour.

Grant the provisioning application Microsoft Graph
[`Application.ReadWrite.OwnedBy`](https://learn.microsoft.com/en-us/graph/permissions-reference#applicationreadwriteownedby)
with administrator consent to manage its applications, service principals, and
credentials.

Give that same identity Azure access to create, update, and delete Bot Service
resources and configure their channels in the resource group. Required
operations include `Microsoft.BotService/botServices/write`,
`Microsoft.BotService/botServices/delete`, and
`Microsoft.BotService/botServices/channels/write`. A suitable Bot Service role
or resource-group Contributor assignment supplies this access; Microsoft
describes [Bot Service resource permissions](https://learn.microsoft.com/en-us/azure/role-based-access-control/permissions/ai-machine-learning#microsoftbotservice).
Ensure the subscription has the `Microsoft.BotService` provider registered.

Managed bot secrets expire after 180 days. Flow-Like renews them automatically
when the bot requests a token with fewer than 14 days remaining. For an idle bot
or an earlier renewal, use **Connection maintenance → Renew bot secret**.
Disconnecting a managed bot removes its Microsoft registration and then
permanently deletes the Entra application from the directory's deleted items,
which needs no permission beyond `Application.ReadWrite.OwnedBy`. If that purge
fails, Microsoft deletes the application after 30 days. Disconnecting a customer
bot leaves that registration in the customer's account. Remove the installed
Teams app separately.

Deleting a Teams event or its app never waits for Microsoft. If the managed
registration cannot be removed, Flow-Like still deletes the bot and records an
`event.teams.orphaned` audit entry. The entry and the API error log include the
Entra object ID, Azure resource ID, and the application's `flow-like-teams:`
recovery tag, so an administrator can remove the registration.

## Troubleshoot

- **Teams Bot is missing:** confirm the app is online, the board permits Remote
  execution, and the hub advertises Teams support.
- **Managed setup fails:** check all six provisioning variables, Graph consent,
  and Azure resource-group access. Microsoft registration changes can take time
  to propagate; refresh status and retry setup.
- **No reply:** confirm the Event is active, the Microsoft registration uses the
  displayed endpoint, and the organization tenant ID matches the conversation.
  Inspect the Event run for workflow or credential errors.
- **A card rejects a response:** check the assigned user IDs and conversation,
  then confirm that the Interaction and execution are still waiting.
- **Delivery is uncertain:** inspect Teams before sending again. A network
  timeout can happen after Microsoft accepted a message, so Flow-Like sends
  each message at most once and reports the attempt as failed.

The send nodes and the executor use these response codes:

| Code | Meaning | Retry |
|------|---------|-------|
| `200` | Delivered. Repeating the request ID returns the original message ID. | — |
| `400`, `403`, `413`, `422` | Invalid message, ended execution, oversized message, Teams rejected it, or an earlier attempt is unconfirmed | No |
| `409` | The same request is still being delivered | Yes, with backoff |
| `429`, `5xx` | Teams throttled the bot or failed temporarily | Yes, with backoff |
