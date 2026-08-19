Friday, 09:12. The support team goes live on Monday, and **Customer Support Copilot** — the triage app you and FlowPilot have been co-building all course — needs its final pass. This assessment is that pass. Everything you need is in the artifacts below and the five lessons behind you; the challenges make the calls.

## Artifact 1 · The board

@FlowLikeStudio

Current state: **Incoming Support Request** feeds the collapsed **Prepare Support Reply** layer (Message in, Reply out), which feeds the collapsed **Human Review** layer, which feeds **Send Reply** (Body). Comment labels mark the sections — 1 · Listen for requests, 2 · Draft with AI, 3 · Approve and send — and a gray note on Human Review reads "Prototype a future review step before implementing its internals." A pure **Customer Message** → **Format Generic Value** pair sits below the execution path.

## Artifact 2 · The run evidence

@RunsAndLogs

The latest run of Incoming Support Request: 1.85 s, green check. Two log lines — "Received onboarding request" (120.00ms), then "Drafted a helpful reply and queued human review" (730.00ms · Token In: 184 · Token Out: 96) from the Draft Helpful Reply node.

## The requirements

- **R1 — Refund routing.** Messages mentioning refunds must reach Human Review, never go straight to Send Reply.
- **R2 — Team dashboard.** A support operations page plus the workflow that loads its ticket data, reachable as a page event.
- **R3 — Weekly numbers.** Ops wants "open tickets mentioning refunds, by week" — ideally as a chart.
- **R4 — No surprise deletions.** Nothing on the board is removed without a human signing off.

## The cast and constraints

- **You** — desktop app, capable model configured, Auto mode currently on.
- **Sam** — support lead. Edits the board by hand, sometimes while FlowPilot reviews are pending.
- **Riley** — new builder. Browser only, free profile model.

## The symptom

- **S1** — One run this week finished green, but the customer's reply body arrived empty. No error was logged.

That's the brief. The challenges below are your Friday — make the calls you'd defend on Monday.
