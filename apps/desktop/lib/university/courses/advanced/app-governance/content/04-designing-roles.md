Friday, 16:58: someone just granted Everyone edit rights. Not maliciously — the support-agent onboarding link had its default role set to Editor "so nobody gets blocked over the weekend." Forty agents joined. Every one of them could now rewrite the live triage Flow.

> **Predict first:** which single setting turned one convenient shortcut into forty editors?

Lesson 3 gave you ladders and flags. This lesson turns them into roles that stay correct after the people change — and defuses the setting you just predicted.

## 1 · Start from a template, then justify every change

Flow-Like ships four starting shapes:

- **Viewer** reads team, files/data, workflows, Events, logs, configuration, and content. Changes nothing.
- **Operator** adds run and trigger rights plus full observability on top of view-level access.
- **Editor** writes files/data, edits workflows and Events, sees full observability — and can publish content and manage routes.
- **Administrator** is elevation: every ordinary check passes.

Templates are starting shapes, not policy. The one that bites teams is Editor: it bundles **Content Publish** and `WriteRoutes` — release authority — with development rights. If your operating model separates building from releasing, cut a narrower Builder role instead of handing Editor around unchanged.

## 2 · The Copilot Operator, filled in

Before you design your own, here's a completed role card for the Copilot — the worked artifact for this lesson:

| Field | Copilot Operator |
| --- | --- |
| Purpose | Run production triage and diagnose failures; never change logic or schedules |
| Ladder levels | Workflows: Run · Events: Trigger · Observability: Full · Team & Access: View · Files & Data: Read |
| Withheld | Workflows Edit, Events Manage, Configuration Edit, Content Edit/Publish, all elevation |
| Owner & review | Sara owns the role; membership reviewed quarterly |
| Allowed tests | Run the triage Flow; trigger the nightly Event manually |
| Denied tests | Save any Flow edit; change the schedule's settings |

@RightsAndRoles

Building it takes two minutes in the New Role dialog you met in lesson 3 (shown there creating a "Support Specialist"): name, a description that states the job — "Runs and diagnoses production; no editing" — then tick exactly the levels on the card and nothing else.

Now yours: pick one real responsibility in your own app and write the same six-row card *before* opening the dialog. If you can't fill "Denied tests," you haven't decided what the role must not do — which means the App has decided for you.

## 3 · The default role is the blast radius

@SharingAccess

Back to the Invite & Access tab from lesson 2 — Direct Invite for named people, Invite Links for populations. Every invite link carries a default role, and that default is the blast radius of every mistaken invitation: the forwarded link, the typo'd address, the URL that ends up in a public doc. Friday's forty editors were exactly this — the answer to the prediction was one dropdown on one link.

Set the default to Viewer or something even narrower. Then grant Operator, Builder, or anything with write access deliberately, per member or group, *after* they join. For each link, record owner, intended audience, expiry or review date, and approver — and revoke it when the onboarding campaign ends.

## 4 · Exceptions expire

One-off needs don't belong in shared roles. A permanent flag added to forty people's role for one person's temporary task never gets removed. Prefer a separate time-bounded role with a named approver, purpose, and expiry. For incidents, use a break-glass role: small eligible group, explicit activation evidence, monitoring while active, removal and review immediately after. Emergency convenience must never become the daily path.

**Watch out:** testing a role while signed in as Owner proves nothing — elevation satisfies every check, so every missing permission is masked. Run the card's allowed *and* denied tests from a real non-elevated account.

## Recap

- Templates are starting shapes; the diff between template and job is your actual role design.
- The default role is the blast radius of every mistaken invitation — Viewer or narrower, always.
- Exceptions get owners and expiry dates; break-glass access ends with the incident.
