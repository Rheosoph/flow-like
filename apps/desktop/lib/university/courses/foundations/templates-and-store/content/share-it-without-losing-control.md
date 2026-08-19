The kit works, and the team next door has noticed. "Can we get access?" Sharing in Flow-Like is three separate knobs — who can *see* the app (visibility), who can *work in* it (invites), and what leaves with a *copy* (fork policy). Turn them in that order.

> **Predict first:** Your app is currently **Private**. Can you invite the neighboring team right now — or is there a step in between?

One prerequisite before any knob turns: sharing needs an **online** app. An offline app can't change visibility at all — but it isn't stranded. From its configuration, select **Create an online copy**: Flow-Like uploads a new copy to your account with secret variable defaults and known token fields stripped, and leaves the local original untouched.

## 1 · Climb the visibility ladder

Open the app's Dashboard → Details tab → **Visibility Status**. Here's that card for an app currently at Prototype — the current level on top, the available transitions below, each with a one-line description, plus two footnotes: offline apps cannot change visibility, and public transitions require central review (1–3 days):

@AppVisibilitySettings

The five rungs:

| Level | Meaning |
| --- | --- |
| **Offline** | Local only, no syncing |
| **Private** | Synced for your account only |
| **Prototype** | Development phase — invite collaborators |
| **Public Request** | Visible in the store; people request to join |
| **Public** | Visible in the store; everyone can join |

So your prediction: Private is *not* enough — it means your account only. **Prototype is the minimum level for sharing with others.** And the ladder has rules on the way down, too: dropping from Prototype back to Private removes all collaborators, and they lose access immediately. Moving up to either public rung submits the app for central review, which takes one to three business days — more on that in the next lesson.

## 2 · Invite the neighbors

Once the app is at Prototype or higher, **Team** appears in its navigation. The page is titled **Access & Relationships**. Here it is for the Customer Support Copilot app — tabs for Team Members, Invite & Access, API Keys, Join Requests, and Connections, with the Invite & Access tab showing a **Direct Invite** card (Invite User) and an **Invite Links** card (Create Link — this fresh app has no links yet and 0 members):

@ShareApps

Direct invites target a specific Flow-Like user; invite links let anyone with the link join. What members can *do* is governed by roles — the owner assigns them and sets a default for new members. Role and permission design is its own discipline; the App Governance course owns that depth.

## 3 · Decide what a fork contains

You control copies, too. In the app's configuration, an **Allow Forking** card decides whether forking is possible at all — and when it is, the fork policy editor decides what a fork ships:

- Toggles for **Flows** (boards, nodes, and their pinned versions), **Files** (everything in the app's storage), **Widgets**, **Templates**, and **Roles**.
- A three-way choice for **Databases**: tables *and* data, tables only (recreated empty), or no database at all.

Two facts to anchor. First, the person forking has no say — the fork dialog only displays the result of your policy. Second, roles can't be truly withheld: a fork always gets its own Owner, Admin, and User roles, even when your custom roles don't travel. And a fork is a copy, full stop — your later edits never flow into it, and its edits never flow back.

> **Watch out:** "Tables only" recreates schema without data — flows in the fork that expect rows will find none until the new owner fills them. Pick the database mode to match what a stranger should legitimately hold.

## Recap

- Visibility, invites, fork policy — see, work, copy. Prototype is the minimum for sharing.
- Prototype → Private evicts all collaborators immediately; public rungs need central review.
- The owner's fork policy decides fork contents; forks always get their own Owner, Admin, and User roles.
