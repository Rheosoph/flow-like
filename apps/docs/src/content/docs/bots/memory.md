---
title: Memory and workspace
description: How a bot remembers facts across chats, how its history stays manageable, and where its files, tables and mail live.
---

Every bot has one continuous history with you, its own memory of facts, and a private workspace for files and data.

## Memory

With **Memory** on and an embedding model chosen, a bot remembers what matters from your chats and brings it back when it's relevant. If either is missing, the bot's **Memory** tab says so and offers **Turn on memory** or a model picker.

### How facts get saved

- **When you ask,** or when the bot decides something is worth keeping, it saves a fact itself.
- **In the background,** once at least six new messages have piled up, the bot reviews them and saves what it learned.

Every fact is tied to evidence: the message it came from and a short verbatim quote. Facts are typed as profile, preference, event, decision or knowledge, and can carry a validity period, an expiry, places on a map and where they came from (said by a person, inferred by the bot or read from external content).

When the bot has just read untrusted content, such as a web page, a mail or a tool result, it can't quietly save facts from it. Saving needs your confirmation, so a crafted page can't plant false memories.

### How recall works

Before the bot answers, it searches its memory with both keywords and meaning, favors recent events and respects time cues like "last week". Outdated facts that were replaced only come back when you ask how things used to be.

### Long histories

A bot keeps one continuous history with you instead of separate chats. As the history grows, older parts are summarized into segments, while the last few exchanges stay word for word. A long pause starts a new segment. You can still search and read the full history, and so can the bot.

Deleting a message also removes or re-checks the facts that relied on it, and rewrites the summaries that contained it.

### The Memory tab

Open a bot's **Memory** tab to review what it knows:

- **Timeline** lists facts by time, with search and a category filter.
- **Map** shows facts that carry places.
- **Forget this fact** removes a single fact.
- **Explore chat history** opens the full history.

While memory is off, you can still review and forget facts, but nothing new is saved or recalled.

In rooms, each bot keeps a separate memory per room. You decide whether it may recall your 1:1 memory there and whether the room shares what it learns. See [Memory in rooms](/bots/rooms/#memory-in-rooms).

## Workspace

Each bot has a hidden **workspace**: a private app only you can open, used for its files and tables. It's created the first time the bot needs it and deleted with the bot.

- **Files** holds what the bot writes or receives: documents it creates, code results, webhook payloads, mail attachments and large web responses.
- **Data** holds its tables, which you can open in Data Studio like any app's.
- Its storage counts toward your plan, but it never takes up a project slot, and you can't share or publish it.

Bots that live only on your computer while signed out keep their workspace on that computer, and it moves with them when you move them to your account.

## Mail

With **Email** on, a bot's **Mail** tab shows its address and its mail grouped by thread. Mail it receives or sends also appears in your 1:1 chat with the bot, so it works with mail using its full history and memory. Where the server can't write into that chat, for example because it's end-to-end encrypted, a thread gets its own room instead. See [Email](/bots/triggers/#email).

## Notes for long tasks

For work that spans several turns, like a study plan or a research job, a bot keeps **notes** in the conversation. A note's body is its hidden working state, such as the plan, sources found or your level, and only the bot and you can read it. Its title and status show as a chip on the replies that touched it. Notes need no ability and are separate from long-term memory.
