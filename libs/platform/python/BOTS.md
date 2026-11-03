# Bots and inboxes

Manage your bots and talk to them (and to people) without the app. Every method lives on
`FlowLikeClient`, with an async twin prefixed `a` (`list_bots` / `alist_bots`). Models are
dataclasses with the API's field names and the full JSON in `raw`.

**Authenticate as a person.** These routes accept a personal access token (`pat_…`) and refuse
API keys and running flows with 403.

```python
from flow_like import FlowLikeClient

client = FlowLikeClient(base_url="https://api.flow-like.com", pat="pat_…")

bot = client.create_bot(
    "Researcher",
    instructions="Answer briefly and cite sources.",
    capabilities=["internet"],
)

# Direct conversations are deduplicated: this returns the existing one when you have it.
conversation = client.create_conversation("direct", [bot.id])
sent = client.send_inbox_message(conversation.id, "Summarize today's AI news")

# Everything newer than your message, newest first.
page = client.list_inbox_entries(conversation.id, after=sent.cursor)
for entry in page.entries:
    print(entry.sender_id, entry.text)
```

A message in a bot's conversation is what makes the bot's host run a turn and append its reply
(server worker or desktop host). Until a host runs for the bot, nothing answers.

## Bots

| Method | Route |
|---|---|
| `list_bots()` | `GET /bots` — Assistant and Flux first (set up on the first call) |
| `get_bot(bot_id)` | `GET /bots/{id}` |
| `create_bot(name, *, appearance=None, capabilities=None, …)` | `POST /bots`; `appearance` defaults to `{"kind": "pip"}` |
| `update_bot(bot_id, *, name=…, description=…, …)` | `PATCH /bots/{id}`; left-out arguments stay, `None` clears |
| `delete_bot(bot_id)` | `DELETE /bots/{id}`; built-in bots answer 409 |
| `list_bot_apps` / `attach_bot_app(bot_id, app_id)` / `detach_bot_app(bot_id, app_id)` | `/bots/{id}/apps[/{app}]` |
| `list_bot_secrets` / `set_bot_secret(bot_id, name, value, *, sealed=None, description=…)` / `delete_bot_secret` | `/bots/{id}/secrets[/{name}]` |

Secrets are write-only: lists return names and metadata, never values.

```python
client.set_bot_secret(bot.id, "CRM_TOKEN", "…", description="CRM API")
client.update_bot(bot.id, capabilities=["internet", "selected_apps"], description=None)
client.attach_bot_app(bot.id, app_id)
```

## Conversations and entries

Every member has its own copy of each event (an `InboxEntry`); copies share `message_id`.
Pages are newest first and positioned by the opaque `cursor` of an entry. `entry.body` is the
plain body (`None` when sealed), `entry.text` the text of a message or edit.

| Method | Notes |
|---|---|
| `list_conversations(*, as_bot=None)` / `get_conversation(id, *, as_bot=None)` | `as_bot`: read one of your bots' inboxes (read-only) |
| `create_conversation(kind, members, *, title=None, encryption=None)` | `direct` takes one member, `group` up to 49 |
| `rename_conversation(id, title)` | groups only; `None` removes the title |
| `add_conversation_members(id, members)` / `remove_conversation_member(id, principal_id)` | |
| `list_inbox_entries(id, *, as_bot, before, after, limit)` | `InboxEntriesPage(entries, next_before)` |
| `iter_inbox_entries(id, *, as_bot, before, after, page_size)` | iterator over every entry, page by page |
| `send_inbox_message(id, text, *, reply_to=None, message_id=None)` | plain envelope `{"v": 1, "enc": "none", "body": …}` |
| `react_to_inbox_message(id, target, emoji, *, op="add")` | `target` is a `message_id` |
| `edit_inbox_message(id, target, text)` / `delete_inbox_message(id, target)` | your own messages only |
| `send_inbox_entry(id, kind, *, envelope=None, envelopes=None, message_id=None)` | any event, incl. sealed `envelopes` |
| `mark_conversation_read(id, up_to=None)` | `up_to` is a cursor; the marker never moves back |
| `get_inbox_keys(ids)` / `register_inbox_key(key_id, algorithm, public_key, *, principal_id=None)` / `revoke_inbox_key(key_id, *, principal_id=None)` | public keys for end-to-end encryption |

```python
# Page older history.
for entry in client.iter_inbox_entries(conversation.id):
    ...  # newest → oldest

async for entry in await client.aiter_inbox_entries(conversation.id):
    ...

# Poll for news; a set next_before means more arrived than one page: reload the window.
news = client.list_inbox_entries(conversation.id, after=newest_cursor)
if news.next_before:
    entries = list(client.iter_inbox_entries(conversation.id, after=newest_cursor))
```

**Retries are safe.** Sends without a `message_id` get a fresh one (`new_inbox_message_id()`);
pass the same `message_id` again to retry and the server returns the first result instead of
sending twice.

## Errors

Failures raise `APIError` (`NotFoundError` for 404) with `status_code` and the raw
`response_body`; the server's code is `json.loads(error.response_body)["error"]["code"]`, e.g.
`BOT_LIMIT_REACHED`, `BOT_CAPABILITY_LOCATION`, `BOT_SYSTEM_UNDELETABLE`, `INBOX_MEMBER_NOT_ALLOWED`,
`INBOX_MESSAGE_ID_TAKEN`. Someone else's bot or a conversation you are not in reads as 404.
