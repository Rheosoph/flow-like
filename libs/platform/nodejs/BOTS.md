# Bots and inboxes

Manage your bots and talk to them (and to people) without the app. Every method lives on
`FlowLikeClient`; models use the API's snake_case field names.

**Authenticate as a person.** These routes accept a personal access token (`pat_…`) and refuse
API keys and running flows with 403.

```ts
import { FlowLikeClient, inboxEntryText } from "@flow-like/sdk";

const sdk = new FlowLikeClient({ baseUrl: "https://api.flow-like.com", pat: "pat_…" });

const bot = await sdk.createBot({
	name: "Researcher",
	instructions: "Answer briefly and cite sources.",
	capabilities: ["internet"],
});

// Direct conversations are deduplicated: this returns the existing one when you have it.
const conversation = await sdk.createConversation({ kind: "direct", members: [bot.id] });
const sent = await sdk.sendInboxMessage(conversation.id, "Summarize today's AI news");

// Everything newer than your message, newest first.
const page = await sdk.listInboxEntries(conversation.id, { after: sent.cursor });
for (const entry of page.entries) console.log(entry.sender_id, inboxEntryText(entry));
```

A message in a bot's conversation is what makes the bot's host run a turn and append its reply
(server worker or desktop host). Until a host runs for the bot, nothing answers.

## Bots

| Method | Route |
|---|---|
| `listBots()` | `GET /bots` — Assistant and Flux first (set up on the first call) |
| `getBot(botId)` | `GET /bots/{id}` |
| `createBot(request)` | `POST /bots`; `appearance` defaults to `{ kind: "pip" }` |
| `updateBot(botId, changes)` | `PATCH /bots/{id}`; omitted fields stay, `null` clears |
| `deleteBot(botId)` | `DELETE /bots/{id}`; built-in bots answer 409 |
| `listBotApps(botId)` / `attachBotApp(botId, appId)` / `detachBotApp(botId, appId)` | `/bots/{id}/apps[/{app}]` |
| `listBotSecrets(botId)` / `setBotSecret(botId, name, { value } \| { sealed })` / `deleteBotSecret(botId, name)` | `/bots/{id}/secrets[/{name}]` |

Secrets are write-only: lists return names and metadata, never values.

```ts
await sdk.setBotSecret(bot.id, "CRM_TOKEN", { value: "…", description: "CRM API" });
await sdk.updateBot(bot.id, { capabilities: ["internet", "selected_apps"], description: null });
await sdk.attachBotApp(bot.id, appId);
```

## Conversations and entries

Every member has its own copy of each event (an `InboxEntry`); copies share `message_id`.
Pages are newest first and positioned by the opaque `cursor` of an entry.

| Method | Notes |
|---|---|
| `listConversations({ as? })` / `getConversation(id, { as? })` | `as`: read one of your bots' inboxes (read-only) |
| `createConversation({ kind, members, title?, encryption? })` | `direct` takes one member, `group` up to 49 |
| `renameConversation(id, title \| null)` | groups only |
| `addConversationMembers(id, members)` / `removeConversationMember(id, principalId)` | |
| `listInboxEntries(id, { as?, before?, after?, limit? })` | returns `{ entries, next_before }` |
| `iterateInboxEntries(id, { as?, before?, after?, pageSize? })` | async iterator over every entry, page by page |
| `sendInboxMessage(id, text, { replyTo?, messageId? })` | plain envelope `{ v: 1, enc: "none", body }` |
| `reactToInboxMessage(id, target, emoji, { op?, messageId? })` | `target` is a `message_id` |
| `editInboxMessage(id, target, text)` / `deleteInboxMessage(id, target)` | your own messages only |
| `sendInboxEntry(id, { kind, envelope? \| envelopes?, message_id? })` | any event, incl. sealed `envelopes` |
| `markConversationRead(id, upTo?)` | `upTo` is a cursor; the marker never moves back |
| `getInboxKeys(principalIds)` / `registerInboxKey(request)` / `revokeInboxKey(keyId, principalId?)` | public keys for end-to-end encryption |

```ts
// Page older history.
for await (const entry of sdk.iterateInboxEntries(conversation.id)) { /* newest → oldest */ }

// Poll for news; a non-null next_before means more arrived than one page: reload the window.
const news = await sdk.listInboxEntries(conversation.id, { after: newestCursor });
if (news.next_before) { /* page with iterateInboxEntries(id, { after: newestCursor }) */ }
```

**Retries are safe.** Sends without a `messageId` get a fresh one (`newInboxMessageId()`); pass the
same `messageId` again to retry and the server returns the first result instead of sending twice.

## Errors

Failures throw `FlowLikeError` (`AuthError` for 401/403, `NotFoundError` for 404) with
`statusCode` and the parsed `body`; the server's code is `error.body.error.code`, e.g.
`BOT_LIMIT_REACHED`, `BOT_CAPABILITY_LOCATION`, `BOT_SYSTEM_UNDELETABLE`, `INBOX_MEMBER_NOT_ALLOWED`,
`INBOX_MESSAGE_ID_TAKEN`. Someone else's bot or a conversation you are not in reads as 404.
