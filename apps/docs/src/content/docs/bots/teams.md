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

Anyone in any team of an org sees the whole org: every team, its people, and every bot with its job description. A Security member sees who works in Finance and what Finance's bots do. A bot that's invited but not accepted yet shows only to its owner and the admins of the team that invited it.

Anyone can leave a team. Their bots in the team leave with them unless they're still in another team of the same org. When the last person leaves, the team is deleted.

## Bots in a team

A bot is in at most one team, so it has one place in the org chart.

- **Your own bots** join any team you're in right away, and leave the team they were in.
- **A colleague's bot** joins once its owner accepts. Only team admins can invite it, and only when its owner is in the org too. The owner sets what the bot may do in the team's room when accepting, like for any [room](/bots/rooms/).

A team holds up to 30 bots, counting invitations that are still open. Flux, the built-in assistant, can't join teams.

### The lead

Each team can have one lead, chosen by an admin from the team's bots. A bot reports to its team's lead, and a lead reports to the lead of the team above. The org chart's **Reporting line** shows that chain for any bot.

The lead is the team's contact for the whole org: everyone in the org can talk to it, and its owner pays for its turns, up to 20 per person and the bot's daily total for other people (100 unless the owner changes it in the bot's safety settings). So when an admin picks a colleague's bot, the bot leads only once its owner accepts, from **Needs you** or the bot's details. Until then the team keeps its current lead, and the admin sees **Waiting for** the owner. Your own bot leads right away. The owner of a lead can **Step down as lead** at any time; the team then has no lead until an admin picks another.

### Ins and outs

With **Ins & outs** on, the org chart shows how work reaches each team from outside its chats:

- **Left of the team:** each bot's [mail address](/bots/triggers/#email) and [webhooks](/bots/triggers/#webhooks), wired to the bot they start. A mail address shows the mail received in the last 7 days, a webhook when it was last called. The heads above the rails add up the team's mail and webhooks.
- **Right of the team:** the mail each bot sent in the last 7 days.
- **Beside a bot's name:** a calendar mark when the bot keeps its own calendar, whose events [wake it](/bots/triggers/#calendar-wake-ups), and a second mark when it reads connected Google, Outlook or CalDAV calendars.

A mail address shows only while the bot's **Email** ability is on, webhooks only while its **Webhook** ability is on, and calendar marks only while its **Calendar** ability is on. Switched-off webhooks don't show. Everyone in the org sees webhook names, never their URLs or secrets, and sees which calendar services a bot reads, never the accounts. Select a team to list all its ways in, or a bot to copy its mail address.

### Apps

With **Apps** on, each team's apps hang under it in the org chart, one card per app. A card's outline shows the app's type, and what it shows depends on that type: an automation its schedule and 14 days of runs with failures in red, an agent where it answers and its chats this week, an interface its thumbnail and pages, a form its entries, a dashboard its trend. Filter the chart to **Agents**, **Apps**, **Data** or **Automations**.

A team's apps are the apps pinned to it and the apps its bots use:

- **Pinning:** choose **Pin app** under a team or in its details. You can pin an app you own or administer. A team admin's pin shows at once; a member's pin waits until a team admin approves it.
- **Apps its bots use:** the apps you gave a bot show under its team with a dashed outline and "via" the bot, and small marks on the bot's row. A bot's own workspace never shows.

Everyone in the org sees an app's name, icon, thumbnail, description and type, even without access to it. Its live numbers show only to its members. Select an app to see which bots use it and which teams it's pinned to, to open it, or to ask for access. Team admins, whoever pinned an app, and the app's admins can unpin it.

Each team header also shows a few of its people's faces. Hover or tap them to see everyone in the team and their role.

## The team room

Every team has a room with its people and bots. The lead reads along and answers what's addressed to the team. The other bots answer when someone mentions them, so a question to the team doesn't wake every bot in it. Once you've mentioned a bot and it answered, your follow-ups go to it without another mention, until you turn to someone else. In a team without a lead, a message that mentions nobody and follows up on no exchange wakes no bot. Use the room for work that concerns the whole team, and open a bot's own chat for everything else.

The room's picture is also the team's icon, in the org chart and wherever you pick a team. Anyone in the team can change it: click the icon in the team's details in the org chart, or edit the room. Without a picture, the team keeps the icon the org chart picks for it.

Deleting a team keeps its room as a plain group chat, so nobody loses the history.

## Asking a team from any chat

Start a message with the team's tag, like `#ITSupport is the VPN down?`, and it goes to that team instead of the bots in the chat you're in. Typing `#` at the start of a message suggests your org's teams. The tag is the team's name without spaces, and you can address up to three teams at once: `#Security #ITSupport`. A tag later in the message is just text.

The question lands in your thread with that team, a chat with you, the team's lead and the lead's owner, which you'll find under the team in your chat list. The lead answers there and can hand parts to teammates.

When you asked from your own chat with one of your bots, the lead's answer also shows up right there, with a link to the thread. In a room or a chat with other people, the answer stays in your thread, so it never reaches people you didn't ask.

You can address any team in an org you belong to, as long as it has a lead. If you leave the org, your threads stay readable, but the lead leaves them. Teams can't be addressed in end-to-end encrypted chats, since the server can't read those messages.

## Talking to bots in your org

The org chart marks every bot with how you can reach it, and clicking the mark takes you there:

| Mark | What it means |
| --- | --- |
| **Team room** | A bot of a team you're in. Talk to it in the team room. |
| **Talk** | The lead of a team you're not in. Talk to it in your thread with that team. |
| **Chat** | Its owner approved your chat with it. |
| **Ask to chat** | Any other bot in your org. Ask its owner for a chat. |
| **Waiting** | You asked; its owner hasn't answered yet. |
| **Declined** | Its owner said no. You can ask again after 7 days. |
| **Classified** | It's classified above you, so it doesn't talk with you. |
| **Joining** | Its owner hasn't accepted the team yet. |

Under each team's name, **Talk to** opens your thread with its lead. A team without a lead can't be contacted directly; ask one of its bots instead.

### Asking for a chat with one bot

**Ask to chat** sends the bot's owner a request with an optional note about what you need. The owner approves it from **Needs you** and decides what the bot may do in the chat, like for any [room](/bots/rooms/), or declines it. You can have up to 10 open requests, and one per bot.

An approved chat has three members: you, the bot and its owner, who can read along; the bot says so when the chat opens, and a lead says the same in your thread with its team. It ends when you or the owner end it, or when you or the bot leave the org. You keep the history.

### Your bots at work

When your bot is someone else's lead or chat partner, you sit in those conversations as its owner. They collect in **Your bots at work** at the end of your chat list. They don't send you notifications unless someone mentions you, and they don't count toward your unread badges. Approvals your bot asks you for still reach you in **Needs you**.

## Classifications for people

Bots have a data classification, which their owner sets with the bot's other safety settings, and people in an org have one too: **Public**, **Internal**, **Confidential** or **Restricted**. Admins of the org's top-level team set it in that team's details. Everyone without one counts as Internal, the same as an unclassified bot. You see your own classification. Only admins of the top-level team see everyone's.

A bot talks only with people at or above its own classification. In a team room, a thread with a lead and a chat with a bot, a bot doesn't answer while someone in the conversation other than its owner is classified below it. It says so once in the conversation, and the org chart marks it as not answering in that team's room. A bot also only asks bots at or below its own level, so nothing classified above you reaches you through a bot.

Before you invite someone or add a bot, the dialog warns you when that would make a bot stop answering in the team room. Classifications, lead requests and their answers, and chat requests and their answers are recorded in the [audit trail](/self-hosting/audit-trail/).

## Who a bot can ask

A bot in a team can ask more bots than its owner's own:

| In the org chart | Who | What happens |
| --- | --- | --- |
| **Direct** | Teammates, when the team allows direct asks | The question goes through without a card. |
| **With OK** | Teammates otherwise, your other bots, and the leads of other teams | The first question to each bot needs an OK from the asking bot's owner, like any [ask between bots](/bots/rooms/#bots-talking-to-bots). |
| **Via lead** | Bots in other teams | The bot asks that team's lead, who hands the question on. |
| **Never** | Bots classified higher than the asking bot | Refused, so a bot's answer never carries data down to a lower classification. |
| **Later** | Bots whose owner hasn't accepted the team yet | Reachable once they join. |

When two bots of different owners talk, both owners can read the exchange. The owner of the bot that answers pays for its turn. Each person can wake another person's bot up to 20 times a day, the same limit as in rooms, and everyone but the owner together up to the bot's daily total for other people, which the owner sets in the bot's safety settings (100 by default). When the total is used up, the bot says so and its owner gets one notification that day.

### Direct asks

A team admin can turn on **Teammates ask each other directly**. Each owner still agrees for their own bots: until they do, their bots keep asking with a card, and the team shows **Waiting**. Owners agree, or stop again, from the card in **Needs you** or from the team's details. Turning the switch off resets every agreement.

Classification rules apply to direct asks as well.

## What a bot reads

The bot's details in the org chart show **What it reads**: the exact text the bot gets about its team before every turn. It lists the team and its purpose, the lead and reporting line, the teammates and whether they can be asked directly, and the organization: its name, what it does (the top-level team's purpose), how many teams and bots it has, and the teams near the bot's own with their lead and purpose. A large org isn't listed in full. The bot sees the teams above, inside and beside its own and the org's first level, up to 15 teams.

The bot answers questions about its own organization from this before it searches the web. For everything the brief leaves out, it uses **Org lookup**: it searches the org's teams, bots and people by name, `#Team` tag, or words from a team's purpose or a bot's job, and gets what you see in the org chart: a team's purpose, lead, bots, people and the teams inside it, a bot's owner, team and job, and the teams a person is in with their role. Org lookup never shows anyone's classification.

In a conversation with people other than its owner, the bot also reads who is there: each person's teams and their role in each, and the lowest classification among them. It never gets anyone's own classification, so it can't give one away. It keeps its answer within that lowest level and uses people's teams and roles to judge what someone may see or ask for. **What it reads** in the org chart leaves this part out, since it depends on the conversation.

A bot reads this only in conversations where everyone is in its org. In a room with people from outside the org, it reads nothing about the org, can't use Org lookup, and can ask only its owner's other bots, so the org's layout never reaches outsiders through it.

Bots outside every team read none of it and see only their owner's other bots, as before.
