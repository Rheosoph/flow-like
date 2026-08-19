> **Predict first:** You type "Add a step that drafts a reply with AI" and hit Enter. Does the board change at that instant?

It doesn't — and that pause is the most important thing about FlowPilot. Between your sentence and any node appearing, your junior colleague plans the change, compiles it, validates it, and hands it to you for review. This lesson walks that loop once, end to end, on the triage app.

@FlowPilotLoop

The infographic states the contract: *you say what, FlowPilot builds how, you stay in charge.* Four stages — 1 · Describe intent, 2 · Plan, 3 · Apply to the board, 4 · Review & run — with a dashed feedback arrow running from Review straight back to Describe intent, captioned "Not quite right? Say so — the loop continues with full context." And the fine print at the bottom is the guarantee this whole course leans on: every change lands as a reviewable edit, and nothing is applied behind your back.

## 1 · Say it

Stage 1 is plain language in chat — the infographic's example is literally "Triage my support inbox". For our app, the opening ask is one sentence:

```text
Create a board that listens for incoming support requests, drafts a reply
with AI, and waits for human approval before anything is sent.
```

No node names, no wiring instructions. Intent.

## 2 · See it

Stages 2 and 3 belong to FlowPilot: it scopes the nodes, wiring, and data, then applies real nodes and real wires. Board edits are compiled and validated before they are offered for application, and the **FlowScript** workspace shows the generated source, its status, and compiler diagnostics. Here's what our opening ask produced:

@FlowLikeStudio

Read it left to right along the white execution wire: an **Incoming Support Request** event node hands its Request to a collapsed layer called **Prepare Support Reply** (Message in, Reply out), which feeds a second collapsed layer, **Human Review**, before **Send Reply** finally gets a Body. Three comment labels — "1 · Listen for requests", "2 · Draft with AI", "3 · Approve and send" — mark the sections, and two gray notes record design intent: one says two implementation steps are grouped into one reusable layer, the other says Human Review is a prototype of a future review step. Below the chain, a pure pair — **Customer Message** feeding **Format Generic Value** over a dashed data wire — sits off the execution path entirely.

That's a working skeleton. You said one sentence; FlowPilot built four stations and labeled its own homework.

## 3 · Steer it

Stage 4 is yours, and it's non-negotiable. The mechanics:

- With **Auto mode off**, FlowPilot asks before side-effecting tool calls and keeps generated changes in a review step. You inspect the diff, then select **Apply** — or dismiss it and the board stays untouched.
- A review that no longer matches the live board (because you kept working) is marked **stale** instead of being applied over your newer work.
- Deleting existing board items always requires your explicit confirmation. Always — this is the one gate even Auto mode never waives.
- With **Auto mode on**, FlowPilot runs tools and applies completed reviews without asking each time, including destructive actions (deletions excepted). Use it only when you're comfortable reviewing after the fact.

Why so strict? Because compiled-and-validated means the change is *well-formed*, not that it's *what you meant*. Validation is FlowPilot's job; intent is yours.

**Watch out:** Apply is not the finish line. Stage 4 is called Review *& run* — run the flow and read what happened before you call the ask done.

Quick recap:

- Describe intent → plan → apply → review: every change lands as a reviewable edit.
- Stale reviews never overwrite newer manual work, and deletions always need your confirmation.
- Auto mode trades the per-step gate for speed — review moves after the apply; it never disappears.
