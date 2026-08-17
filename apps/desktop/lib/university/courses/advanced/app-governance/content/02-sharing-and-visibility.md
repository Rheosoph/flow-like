"Just make it Public so the team can find it." It's Tuesday, the Copilot's triage finally works, and someone in standup wants feedback from three colleagues. Public would put the App in the store for everyone on the platform. What the team actually needs is one step up from Private — and knowing the difference is this whole lesson.

> **Predict first:** which of these needs central review — inviting a collaborator, or listing the App in the store?

## 1 · The visibility ladder

@VisibilitySettings

This is the Copilot's Visibility Status page mid-project. The current state is **Prototype** — the UI calls it "Development phase, invite collaborators," and that's exactly what it is: the App stays team-only, but you can now invite people in. Around it sit the three transitions: back to **Private** ("Synced for your account only"), forward to **Public Request** ("Visible, people can request to join"), or all the way to **Public** ("Everyone can join, visible in store").

Two footnotes at the bottom of that page do a lot of governance work. *Offline apps cannot change visibility status* — visibility is an online concept, which is one more reason lesson 1 put the Copilot online. And *public transitions require central review (1–3 days)* — you don't flip to Public; you request it. Lesson 6 covers what reviewers want to see.

That resolves the prediction: inviting a collaborator is a Prototype-level action you control today; a store listing is a reviewed transition you request and wait for.

## 2 · Visibility is reach; roles are capability

Visibility never grants or limits what a member can do. A Prototype App still needs deliberate membership and least-privilege roles. A Public App still needs server-side authorization on every protected operation. And in the other direction: granting someone a role doesn't publish anything.

Keep the two axes separate in your head — *who can find or join the App* (visibility) versus *what a member can do inside it* (roles). Most sharing incidents are a confusion of exactly these two.

## 3 · The team area

@SharingAccess

Here's where members actually arrive: the Copilot's Team area, on the **Access & Relationships** page ("People and connected apps — in one place", currently 0 members). Tabs run across the top — Team Members, **Invite & Access**, API Keys, Join Requests, Connections. The open Invite & Access tab offers two paths in: **Direct Invite** with an Invite User button, and **Invite Links** with a Create Link button (none exist yet).

A direct invite targets one named person. An invite link is a standing access path — treat it like infrastructure: it has an owner, an intended audience, an expiry or rotation plan, and, most consequentially, a default role that every redeemer receives. That default role gets its own reckoning in lesson 4.

## 4 · Widen one dimension at a time

The Copilot's staged path to production:

1. Stay **Private** while building; finish the lesson-1 inventory.
2. Create least-privilege roles *before* inviting anyone (that's the next module).
3. Move to **Prototype** when collaboration starts; invite a small test group and test as a recipient, not as the owner.
4. Pin production-facing Events to tested versions before any public step (lesson 6).
5. Request the public target with evidence ready; monitor the review on the request.

Change visibility, team size, default role, and execution paths one at a time. Widen three dimensions in one afternoon and Friday's incident report can't tell you which one broke.

**Watch out:** owners testing only as themselves is the classic silent failure — elevation masks every missing permission. The first Prototype test belongs to a non-owner account.

## Recap

- Private → Prototype → Public Request → Public: each step widens reach, never capability.
- Prototype is the development-phase state: team-only, invitations open.
- Public transitions are reviewed requests with evidence — not toggles.
