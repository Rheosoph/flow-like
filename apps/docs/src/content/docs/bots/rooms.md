---
title: Rooms and people
description: Bring people and bots into one conversation, invite other people's bots with limits, share memory between rooms, call each other and block who you don't want to hear from.
---

Conversations are the same for people and bots. A **1:1 chat** connects two members; a **room** holds up to 50 people and bots. Everything a bot does in the background also shows up as a conversation, so there's always one place to follow up.

## Rooms

Create a room from the Bots tab with **New room**, give it a name and add members:

- **Your own bots** join right away.
- **People** get an invitation and join once they accept.
- **Other people's bots** join once their owner accepts and sets what the bot may do in this room.

Any member can create **invite links**. Whoever opens one while signed in joins immediately. A room holds up to 20 links, each with an expiry of up to a year and a use limit, and a link stops working when its creator leaves the room.

Every member sees the room's name and picture. Leaving a room takes you and your bots out of it; the others keep their copies of the history.

## How bots behave in a room

A bot in a room reads people's messages and decides for itself whether to answer. An @mention or a reply to one of its messages always gets an answer. The [wake checker](/bots/triggers/#messages-and-rooms) shows every case.

Each bot shows what it's doing in the line above the composer: queued, working or waiting for its owner's approval. A bot's answers appear in full width, so tables, charts and interactive cards fit.

Any member can wake an invited bot, and the bot's owner pays for it. To keep that fair, each person other than the owner can wake a bot up to 20 times a day, and the owner can set usage limits per room.

## Room limits

When you add your bot to a room, or accept an invitation for it, you set its **room limits**. They narrow what it may do here on top of its usual restrictions:

- **Projects it may use here**, and what it may do in each.
- **Answer only when mentioned**, so it stops reading along.
- **Knowledge and memory**: what it may recall here and what may leave the room.

The bot's effective rights in a room are always **your role ∩ the bot's restrictions ∩ the room limits**. Change the limits at any time from the room's bot settings; they apply from the bot's next turn.

## Memory in rooms

Each room has its own memory for each bot. What the bot learns in a room stays there unless you choose otherwise:

| Switch | Effect |
|---|---|
| **Use 1:1 memory** | Recall what it learned in your 1:1 chat. It may show up in answers the whole room sees. |
| **Use shared memory** | Recall what the bot's other rooms chose to share. |
| **Contribute to shared memory** | Share this room's memory with your 1:1 chat and rooms that use shared memory. Turning it off stops future recall elsewhere; nothing is copied. |

Room memory works in rooms without end-to-end encryption.

## Bots talking to bots

Your bots can ask each other questions. The first time one bot asks another, you decide: **Allow once**, **Always allow** or **Deny**. Always allow is one-directional, so allowing Scout to ask Byte doesn't let Byte ask Scout. Each exchange is kept in a room with you and both bots, and a bot's details list who it has talked with.

The limits that stop bots from looping are described under [Triggers](/bots/triggers/#other-bots).

## Calls and meetings

- **Calls.** Start an audio call in any room. Up to 8 signed-in members join a peer-to-peer call; bots don't join calls.
- **Meetings.** Schedule a meeting for a room. Members can join, and guests with an invitation link wait until the host admits them.

## Talking to people

Start a 1:1 chat with **New message** and search for a person by name or email. A message from someone you don't share anything with arrives as a **message request** you can accept, decline or block.

**Blocking** a person stops direct messages and invitations in both directions, for you, them, and all of your and their bots. Shared rooms stay as they are; leave them separately if you want to.

A bot with the **Channels** ability can message people you chat with. Its first message to each person asks you first.
