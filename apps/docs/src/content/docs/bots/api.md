---
title: API and SDKs
description: Manage bots, conversations and messages from code with the REST API, the Node.js, Python and Rust SDKs, webhooks and the Ask Bot flow node.
---

Bots are headless: everything they do runs on their host, and every app is just a window onto it. You can drive the same bots and conversations from code.

## Ways in

| Use | Best for |
|---|---|
| [Webhooks](/bots/triggers/#webhooks) | Waking a bot from another service with a secret URL. |
| [Ask Bot node](/bots/triggers/#from-your-flows) | Asking a bot from a Flow-Like flow and using its reply. |
| SDKs | Managing bots, secrets, conversations and messages from your own code. |
| REST API | Everything else. Documented in the OpenAPI reference of your hub. |

All bot and conversation endpoints need a signed-in user or a personal access token. App API keys and running flows can't manage bots.

## SDKs

The [Node.js](/dev/sdks/nodejs/), [Python](/dev/sdks/python/) and [Rust](/dev/sdks/rust/) SDKs cover bots and conversations:

- **Bots:** list, create, read, update and delete bots, attach apps and manage secrets.
- **Conversations:** create 1:1 chats and rooms, add members, send and read messages, react, mark as read, and publish end-to-end encryption keys.

Sending a message to a bot is enough to wake it. Its host answers in the same conversation, so read the conversation's new entries to get the reply.

## REST endpoints

Paths are relative to your hub's API, for example `https://api.flow-like.com/api/v1`.

### Bots

| Method | Path | Purpose |
|---|---|---|
| `GET`, `POST` | `/bots` | List or create your bots. |
| `GET`, `PATCH`, `DELETE` | `/bots/{id}` | Read, update or delete a bot. |
| `POST` | `/bots/{id}/clone` | Duplicate a bot. |
| `POST` | `/bots/{id}/ask` | Wake a bot with a message and optionally wait up to 100 seconds for the reply. |
| `GET` | `/bots/{id}/apps` | The apps attached to a bot. |
| `PUT`, `DELETE` | `/bots/{id}/apps/{app_id}` | Attach or detach an app. |
| `GET`, `PUT` | `/bots/{id}/apps/{app_id}/restrictions` | What the bot may do in that app. |
| `GET` | `/bots/{id}/secrets` | Secret names and descriptions. Values are never returned. |
| `PUT`, `DELETE` | `/bots/{id}/secrets/{name}` | Store or delete a secret. |
| `GET`, `POST` | `/bots/{id}/tasks` | List or create scheduled tasks. |
| `PATCH`, `DELETE` | `/bots/{id}/tasks/{task_id}` | Change, pause or delete a task. |
| `POST` | `/bots/{id}/tasks/{task_id}/fire` | Run a task now. |
| `GET`, `POST` | `/bots/{id}/webhooks` | List or create webhooks. |
| `PATCH`, `DELETE` | `/bots/{id}/webhooks/{hook_id}` | Change or delete a webhook; `POST .../rotate` issues a new secret. |
| `POST` | `/bots/hooks/{hook_id}` | The public webhook URL. |
| `POST` | `/bots/{id}/turns/{turn_id}/approval` | Answer an approval card. |
| `GET`, `DELETE` | `/bots/{id}/tool-grants` | List and revoke standing approvals. |
| `GET` | `/bots/{id}/memory/facts` | What the bot remembers. |
| `DELETE` | `/bots/{id}/memory/facts/{fact_id}` | Forget one fact. `DELETE /bots/{id}/memory` forgets everything. |
| `GET`, `POST` | `/bots/{id}/skills` | List or add skills; approve or disable them under `.../skills/{skill_id}`. |
| `GET` | `/bots/{id}/usage` | Usage and limits, overall and per room. |
| `GET` | `/bots/presence` | Which hosts are online. |

### Conversations

| Method | Path | Purpose |
|---|---|---|
| `GET`, `POST` | `/inbox/conversations` | List or create 1:1 chats and rooms. |
| `GET`, `PATCH`, `DELETE` | `/inbox/conversations/{id}` | Read, rename or leave a conversation. |
| `GET`, `POST` | `/inbox/conversations/{id}/entries` | Read or send messages. |
| `POST` | `/inbox/conversations/{id}/read` | Mark as read. |
| `POST`, `DELETE` | `/inbox/conversations/{id}/members/...` | Add or remove members. |
| `PATCH` | `/inbox/conversations/{id}/members/{member}/limits` | Set a bot's room limits. |
| `GET` | `/inbox/conversations/{id}/turns` | What the bots in a conversation are doing. |
| `GET`, `POST`, `DELETE` | `/inbox/conversations/{id}/links` | Invite links. |
| `GET` | `/inbox/invitations` | Your pending invitations; accept or decline them. |
| `GET`, `PUT`, `DELETE` | `/inbox/blocks/...` | Blocked people and bots. |
| `GET`, `PUT` | `/inbox/keys` | End-to-end encryption keys. |

### Ask a bot and wait

```bash
curl -X POST "https://api.flow-like.com/api/v1/bots/$BOT_ID/ask" \
  -H "Authorization: Bearer $FLOW_LIKE_PAT" \
  -H "Content-Type: application/json" \
  -d '{"message": "Summarize yesterday'\''s failed runs", "wait_seconds": 60, "request_id": "daily-2026-11-09"}'
```

The message lands in your 1:1 chat with the bot, marked **Event: From a flow**. A repeated `request_id` returns the same turn instead of starting a new one. Only the bot's owner can ask it this way.

## Desktop

The desktop app runs its own bot host. Bots you create while signed out are stored on that computer and are moved to your account with an explicit click, never silently. The same host answers your account's desktop bots while the app runs.
