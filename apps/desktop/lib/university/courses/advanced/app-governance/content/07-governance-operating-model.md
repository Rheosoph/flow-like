Three weeks after launch, an auditor asks one question: "Who approved the change that went live on the 14th, and what did they see when they approved it?" The team knows the answer — it's somewhere in a chat thread, between lunch plans and a GIF. That's not an operating model. This lesson assembles the one-page record that answers in thirty seconds.

> **Predict first:** which artifacts from lessons 1–6 already answer half of the auditor's question?

## 1 · Four loops

Everything you've built runs on four recurring loops, each with an accountable owner, required evidence, and an escalation path:

- **Access:** request → approve → provision → test → review → revoke. (Lessons 3–4.)
- **Change:** classify → build on Latest → test → version → approve → pin → verify. (Lesson 6.)
- **Exposure:** inventory surfaces → assemble evidence → request → respond to review → monitor. (Lessons 2 and 6.)
- **Incident:** detect → contain → preserve evidence → restore a known version → communicate → fix the control.

One person may run several loops on a small team — fine, as long as the combination is visible rather than accidental.

## 2 · Classify by consequence, not diff size

Before implementing any change, name its class: **Low** (documentation, presentation; no permission, data, route, or runtime effect), **Standard** (tested compatible fix or feature with known rollback), **High** (anything touching permissions, public routes, data schemas, credentials, execution location, major versions, or publication), **Emergency** (time-critical containment, reviewed retrospectively).

The trap is judging by diff size. Flipping one checkbox — the default role from Viewer to Operator — is a one-line change and a High classification: it rewrites the exposure boundary for every future invitation.

## 3 · The Copilot's governance record, filled in

@AppAnatomy

Structure the record along the five cards from lesson 1 — Flows, Experiences, Data, Reuse, Delivery — then add who's on the hook for each. Here's the Copilot's, filled in:

| Section | Customer Support Copilot |
| --- | --- |
| Boundary | Online; Hybrid triage Flow; chat Page (team); nightly remote schedule; CRM sync API Event; ticket DB + reply templates |
| Roles | Default: Viewer. Copilot Operator (Run/Trigger/Full), Builder (Edit/Manage/data write), release group (Content Publish + routes). Quarterly membership review — next 15 Nov |
| Credentials | `CRM_API_TOKEN` — owner Priya, server-side for the schedule, desktop Secret for local debugging, 90-day rotation. `SUPPORT_API_URL` per environment |
| Release | Develop on Latest; staging Event tests each new patch; production pinned to 1.4.3; rollback target 1.4.2; approver Sara |
| Publication | Prototype today; Public Request planned; evidence package linked; AI Act assessment current (feature enabled) |
| Monitoring | Alerts on run failures and denied-request spikes; Rosa is first responder |
| Incident | Deactivate Event → revoke suspect role → re-pin known-good → preserve logs → communicate. Owner: Marek |

That table *is* the answer to the auditor — and to the prediction: the lesson-1 inventory, the lesson-4 role card, and the lesson-6 release steps were already three of its seven rows.

Now reproduce it for your own app: same seven rows, real names in every one. Then run the cheapest governance review that exists: ask each named person to demonstrate one permitted and one denied path. If either demo surprises anyone, you found the gap before an incident did.

## 4 · Cadence

- Quarterly, and after job changes: elevated and write-role membership.
- Every release: public routes, Events, and pinned versions.
- On schedule: credential rotation, plus a rollback drill *before* you need one.
- On material change — purpose, data, audience, model use: refresh publication and assessment evidence.

**Watch out:** approval that lives only in chat is approval that doesn't exist. If the record can't name the version, the approver, and the evidence, the change didn't happen — govern accordingly.

## Recap

- Four loops — access, change, exposure, incident — each with an owner and required evidence.
- Classification follows consequence: permission and exposure changes are High at any diff size.
- The one-page record turns lessons 1–6 into something an auditor — and 2 a.m. you — can actually use.
