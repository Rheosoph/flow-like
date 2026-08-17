Tuesday afternoon. You're mid-refactor on the Customer Support Automation board — half the nodes rewired, nothing tested — when the support lead pings: "chat just started replying in German?" Of course it did. The `/chat` Event follows **Latest**, and Latest is whatever your draft looks like *right now*. Your work-in-progress is live. This lesson makes sure you're never in that sentence again.

## 1 · Freeze what works

A **Flow version** is an immutable snapshot of your draft. Open the flow, choose **Manage Board**, and you get this dialog:

@FlowVersions

Alongside the board's name, description, stage, and log level, the **Version** selector reads `Latest (1.0.0)`, and next to **Save** sits **Create Version** — clicking it asks exactly one question: version type **Major**, **Minor**, or **Patch**. (The dialog also confirms a lesson-2 fact in passing: this board's execution mode is Local, because "offline projects only support local execution.")

The bump is your call — Flow-Like doesn't guess semantic impact:

- **Major**: existing callers may need migration.
- **Minor**: compatible new behavior.
- **Patch**: a compatible correction.

The snapshot is read-only forever. Latest stays the editable line you keep working on.

## 2 · Pin the consumers that need stability

Every Event chooses what it follows: **Latest** — the live draft, right for your own development entry point — or a **numbered version**, frozen until you deliberately repoint it, right for anything users or external systems depend on. Page-target Events pick a version the same way, and Data Studio governed actions can bind to a published version too.

So the Tuesday fix is three moves: create a version from the last known-good state, pin `/chat` and `/triage` to it, and refactor Latest in peace.

One label fools almost everyone: the App also has a free-form **Version** field in its settings. It snapshots nothing — not Flows, not Events, not storage. Treat it as human-readable release notes ("v2.1 — new triage form") and record in the changelog which Flow versions that label corresponds to. A Flow Template can snapshot a selected version for reuse elsewhere, but a Template is a blueprint, not a release control.

## 3 · Release deliberately, roll back calmly

A release is a short, boring ritual — boring is the point:

1. Test Latest end to end through the real routes (`/chat`, `/triage`), not just in Studio.
2. Inspect the matching runs and logs, not only the visible reply.
3. Create the numbered version with the honest bump.
4. Point production Events at it; verify route, permissions, location, credentials, and data in the target environment.
5. Record what shipped, and keep the previous tested version around.

Before you ship, decide what would make you roll back: an error threshold, a missing outcome, a latency ceiling. Then rollback is mechanical: repoint the pinned Events to the previous version, debug on Latest, cut a patch version, retest, repin. No archaeology at 2 a.m.

Permissions belong in the release review too: an online App supports roles and invitations, and each role should get only what its users and runners need.

> **Watch out:** don't change everything at once. A release that bundles new logic, a data migration, new credentials, and a route change can't be diagnosed when it misbehaves. Stage what you can.

## Recap

- Numbered versions are immutable snapshots; Latest is the editable draft — every Event chooses which one it follows.
- Pin every production-facing Event to a tested version; the App's version label documents, it never snapshots.
- Decide the rollback trigger before shipping, and keep the previous tested version one repoint away.
