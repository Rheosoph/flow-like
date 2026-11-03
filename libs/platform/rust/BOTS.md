# Bots and inboxes

Manage your bots and talk to them (and to people) without the app. Every method lives on
`flow_like_platform::Client`; models mirror the API's JSON. String enums (`BotCapability`,
`InboxEntryKind`, …) carry an `Other(String)` variant, so values a newer server adds never break
deserialization.

**Authenticate as a person.** These routes accept a personal access token (`pat_…`) and refuse
API keys and running flows with 403.

```rust
use flow_like_platform::{
    Auth, BotCapability, Client, CreateBot, CreateConversation, InboxEntriesQuery,
};

let client = Client::new("https://api.flow-like.com", Auth::from_token("pat_…")?)?;

let bot = client
    .create_bot(&CreateBot {
        instructions: Some("Answer briefly and cite sources.".into()),
        capabilities: vec![BotCapability::Internet],
        ..CreateBot::new("Researcher")
    })
    .await?;

// Direct conversations are deduplicated: this returns the existing one when you have it.
let conversation = client
    .create_conversation(&CreateConversation::direct(&bot.id))
    .await?;
let sent = client
    .send_inbox_message(&conversation.id, "Summarize today's AI news")
    .await?;

// Everything newer than your message, newest first.
let page = client
    .list_inbox_entries(
        &conversation.id,
        &InboxEntriesQuery { after: Some(sent.cursor.clone()), ..Default::default() },
    )
    .await?;
for entry in &page.entries {
    println!("{}: {:?}", entry.sender_id, entry.text());
}
```

A message in a bot's conversation is what makes the bot's host run a turn and append its reply
(server worker or desktop host). Until a host runs for the bot, nothing answers.

## Bots

| Method | Route |
|---|---|
| `list_bots()` | `GET /bots` — Assistant and Flux first (set up on the first call) |
| `get_bot(bot_id)` | `GET /bots/{id}` |
| `create_bot(&CreateBot)` | `POST /bots`; `CreateBot::new` sets `appearance` to `{"kind": "pip"}` |
| `update_bot(bot_id, &UpdateBot)` | `PATCH /bots/{id}`; `None` leaves a field, `Some(None)` clears it |
| `delete_bot(bot_id)` | `DELETE /bots/{id}`; built-in bots answer 409 |
| `list_bot_apps` / `attach_bot_app(bot_id, app_id)` / `detach_bot_app(bot_id, app_id)` | `/bots/{id}/apps[/{app}]` |
| `list_bot_secrets` / `set_bot_secret(bot_id, name, &SetBotSecret)` / `delete_bot_secret` | `/bots/{id}/secrets[/{name}]` |

Secrets are write-only: lists return names and metadata, never values, and `SetBotSecret`'s
`Debug` output redacts the value.

```rust
use flow_like_platform::{SetBotSecret, UpdateBot};

client
    .set_bot_secret(&bot.id, "CRM_TOKEN", &SetBotSecret::value("…").description("CRM API"))
    .await?;
client
    .update_bot(&bot.id, &UpdateBot { description: Some(None), ..Default::default() })
    .await?;
```

## Conversations and entries

Every member has its own copy of each event (an `InboxEntry`); copies share `message_id`.
Pages are newest first and positioned by the opaque `cursor` of an entry. `entry.body()` is the
plain body (`None` when sealed), `entry.text()` the text of a message or edit.

| Method | Notes |
|---|---|
| `list_conversations(as_bot)` / `get_conversation(id, as_bot)` | `as_bot`: read one of your bots' inboxes (read-only) |
| `create_conversation(&CreateConversation)` | `CreateConversation::direct(member)` / `::group(title, members)` |
| `rename_conversation(id, title)` | groups only; `None` removes the title |
| `add_conversation_members(id, members)` / `remove_conversation_member(id, principal_id)` | |
| `list_inbox_entries(id, &InboxEntriesQuery)` | `InboxEntriesPage { entries, next_before }` |
| `stream_inbox_entries(id, InboxEntriesQuery)` | `'static` stream over every entry, page by page |
| `send_inbox_message(id, text)` | plain envelope `{"v": 1, "enc": "none", "body": …}` |
| `react_to_inbox_message(id, target, emoji, ReactionOp)` | `target` is a `message_id` |
| `edit_inbox_message(id, target, text)` / `delete_inbox_message(id, target)` | your own messages only |
| `send_inbox_entry(id, &SendInboxEntry)` | any event: `::message`, `::reply`, `::reaction`, `::edit`, `::delete`, `::sealed` |
| `mark_conversation_read(id, up_to)` | `up_to` is a cursor; the marker never moves back |
| `get_inbox_keys(ids)` / `register_inbox_key(&RegisterInboxKey)` / `revoke_inbox_key(key_id, principal_id)` | public keys for end-to-end encryption |

```rust
use futures_util::TryStreamExt;

// Page older history, newest → oldest.
let mut history = client.stream_inbox_entries(&conversation.id, InboxEntriesQuery::default());
while let Some(entry) = history.try_next().await? { /* … */ }

// Poll for news; a set next_before means more arrived than one page: reload the window.
let news = client
    .list_inbox_entries(&conversation.id, &InboxEntriesQuery { after: Some(newest), ..Default::default() })
    .await?;
```

**Retries are safe.** Sends without a message id get a fresh one (`new_inbox_message_id()`);
fix one with `SendInboxEntry::message(text).with_message_id(id)` and send it again to retry: the
server returns the first result instead of sending twice.

## Errors

Failures are `Error::Api { status, body, .. }` (`is_auth()` for 401/403, `is_not_found()` for
404); the server's code is `body["error"]["code"]`, e.g. `BOT_LIMIT_REACHED`,
`BOT_CAPABILITY_LOCATION`, `BOT_SYSTEM_UNDELETABLE`, `INBOX_MEMBER_NOT_ALLOWED`,
`INBOX_MESSAGE_ID_TAKEN`. Someone else's bot or a conversation you are not in reads as 404.
