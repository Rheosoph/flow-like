Monday's team chat, condensed: "is the deploy done?" — nine times, nine slightly different phrasings, nine interruptions. Priya built a bot for it. It answered flawlessly for a week — and then went silent every evening at six.

> **Predict first:** the bot's flow was fine and its token was valid. Why did it stop answering outside office hours?

## 1 · The chore

Answer the recurring pings — deploy status, wifi password, "where's the deck" — in the chat where people already ask, without a human context-switching every time.

## 2 · The trigger

In the flow, the entry point is a **Chat Event** node — the same entry contract the built-in chat interface uses. In the Events workspace, you then connect **Discord** or **Telegram** to that node; both are event types the Chat Event supports.

**Telegram:** create the bot with BotFather, paste the issued token into the event configuration, done. Delivery comes in two styles — *long polling*, which needs a continuously running process (your desktop app, or a self-hosted worker), and a *hosted webhook*, which needs a public HTTPS URL and runs without your machine. Telegram permits one webhook per bot: delete the webhook before using the same token for polling again.

**Discord:** also two styles. The *Gateway* connects with a bot token and the intents you enable — message content is a privileged intent that must also be switched on in Discord's developer portal. The *interactions webhook* serves slash commands to a hosted endpoint, verified by the application's public key.

And there's the evening mystery solved: the bot used polling, and polling ran inside the desktop app. Laptop closed at six, bot asleep at six. For an always-on bot, use hosted webhook delivery.

## 3 · The flow

1. **Chat Event** — receives the incoming message context.
2. Match the command — a transform step checking for the configured prefix, like `/status`. Both sink types support a command prefix and mention rules, so the bot only wakes for messages aimed at it.
3. Look up the answer — for `/status`, read the deploy-status file from storage; for `/report`, reuse the numbers from last lesson's recipe. Recipes compound.
4. **Send Message** — post the answer back. Telegram and Discord each bring their own Send Message node, plus richer siblings for files and images.

One honest caveat: a webhook delivery and a polling adapter don't hand your flow identical payload shapes. If one flow serves both styles, check the shape before reaching into nested fields.

## 4 · Guardrails

- **Keep the allowlist narrow.** Both sinks can restrict which chats they process. A bot that can trigger flows is a doorway into your app — scope it to the ops channel first, widen deliberately.
- **The token is a credential.** It grants control of the bot. Keep it out of screenshots, repos, and pasted support messages, and rotate it if it leaks.
- **Loop insurance comes built in.** The Discord adapter ignores messages authored by bots — so your bot and a colleague's bot can share a channel without answering each other forever. Don't undo that safety with a webhook that echoes into the same channel it listens to.

## 5 · Keep it

The bot lives in the same Events workspace as everything else:

@AppEvents

That's the support app's Events list — "2 of 2 events", each with a green active dot: a Quick Action named "Triage selected request" on `/triage`, and a Chat UI event named "Support assistant" on `/chat`, both pointing at their flow. Your bot joins this list as one more event on one more flow — one page tells you what's live, what type it is, and where it points. When the bot misbehaves, you start here, not in the chat scrollback.

**Recap**

- Chat Event is the entry node; Discord and Telegram are event types connected to it.
- Polling needs a continuously running process; hosted webhooks stay on when the laptop doesn't.
- Narrow allowlists, guarded tokens — a bot is a doorway, treat it like one.
