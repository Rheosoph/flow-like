---
title: Teams
description: Organize your bots and your colleagues' bots into nested teams with leads, give each team a room, and control which bots may ask each other.
---

With a handful of bots, a flat list works. Once you have many, or your colleagues bring theirs, **teams** keep them findable: Finance, IT Support, Security and so on, each with its people, its bots and one bot that leads it. Teams nest, so a company can be one **org**: a top-level team with the teams below it.

Bots know about their teams. Before every turn, a bot in a team reads who its teammates are, whom it reports to and how the org is laid out, so it can hand a question to the right colleague instead of guessing.

## Two views

The Bots tab has two views, switched at the top:

- **Chats** is for daily work. Your conversations are grouped by team, each team's room first, then its bots. Above them, **Needs you** collects invitations and requests that wait for your answer. Filter by **Unread** or **Pinned** when the list gets long.
- **Org chart** is for organizing. It shows every team of your orgs as a tree with its lead, its bots and their job descriptions. Drag a bot onto another team to move it, select a bot or team to see and change its details, and turn on **Who can ask** to see which bots a selected bot can reach.

## Creating teams

Choose **New team** in the org chart and give it a name and, optionally, a purpose. Every bot in the team reads the purpose, so a sentence like "Keep our systems patched and watch for threats. Incidents go to Sentinel first." shapes how they work together.

A team at the top level starts a new org. To put a team inside another one, you need to be an admin of that other team. Teams nest up to five levels below the top-level team, and an org holds up to 100 teams.

You become the new team's admin, and the team gets its own room.

## People

A team has **admins** and **members**. Admins invite people, change roles, add and remove bots, pick the lead, and rename, move or delete the team. Members add their own bots and use the team's room.

Invite people from the team's details in the org chart with **Invite people**. People already in your org are suggested first; anyone else you find by name or email. Invited people join once they accept. A team holds up to 20 people, and keeps at least one admin while it has other people in it.

Anyone in any team of an org sees the whole org: every team, its people, and every bot with its job description. A Security member sees who works in Finance and what Finance's bots do.

Anyone can leave a team. Their bots in the team leave with them unless they're still in another team of the same org. When the last person leaves, the team is deleted.

## Bots in a team

A bot is in at most one team, so it has one place in the org chart.

- **Your own bots** join any team you're in right away, and leave the team they were in.
- **A colleague's bot** joins once its owner accepts. Only team admins can invite it, and only when its owner is in the org too. The owner sets what the bot may do in the team's room when accepting, like for any [room](/bots/rooms/).

A team holds up to 30 bots, counting invitations that are still open. Flux, the built-in assistant, can't join teams.

### The lead

Each team can have one lead, chosen by an admin from the team's bots. A bot reports to its team's lead, and a lead reports to the lead of the team above. The org chart's **Reporting line** shows that chain for any bot.

## The team room

Every team has a room with its people and bots. The lead reads along and answers what's addressed to the team. The other bots answer when someone mentions them, so a question to the team doesn't wake every bot in it. Use the room for work that concerns the whole team, and open a bot's own chat for everything else.

Deleting a team keeps its room as a plain group chat, so nobody loses the history.

## Who a bot can ask

A bot in a team can ask more bots than its owner's own:

| In the org chart | Who | What happens |
| --- | --- | --- |
| **Direct** | Teammates, when the team allows direct asks | The question goes through without a card. |
| **With OK** | Teammates otherwise, your other bots, and the leads of other teams | The first question to each bot needs an OK from the asking bot's owner, like any [ask between bots](/bots/rooms/#bots-talking-to-bots). |
| **Via lead** | Bots in other teams | The bot asks that team's lead, who hands the question on. |
| **Never** | Bots classified higher than the asking bot | Refused, so a bot's answer never carries data down to a lower classification. |
| **Later** | Bots whose owner hasn't accepted the team yet | Reachable once they join. |

When two bots of different owners talk, both owners can read the exchange. The owner of the bot that answers pays for its turn, and each person can wake another person's bot up to 20 times a day, the same limit as in rooms.

### Direct asks

A team admin can turn on **Teammates ask each other directly**. Each owner still agrees for their own bots: until they do, their bots keep asking with a card, and the team shows **Waiting**. Owners agree, or stop again, from the card in **Needs you** or from the team's details. Turning the switch off resets every agreement.

Classification rules apply to direct asks as well.

## What a bot reads

The bot's details in the org chart show **What it reads**: the exact text the bot gets about its team before every turn. It lists the team and its purpose, the lead and reporting line, the teammates and whether they can be asked directly, and an outline of the org with each team's lead.

Bots outside every team read none of it and see only their owner's other bots, as before.
