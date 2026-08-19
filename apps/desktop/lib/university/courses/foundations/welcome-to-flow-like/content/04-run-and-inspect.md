You press Run. A toast flashes green: success. And yet no reply ever reaches the customer. Who's lying? Nobody — "success" only claims the graph finished without crashing. Whether it did the *right thing* is a different question, and the answer lives in the run's evidence. This lesson teaches you to read it.

## 1 · Run on purpose, not on hope

Start a run from the event node — the play button you spotted on *Incoming Support Request*. If the event asks for input, give it a small synthetic payload: a made-up support question whose correct answer you already know. Never a real customer record, never a password or token. And decide what you expect *before* you run; evidence can only surprise you if you had an expectation.

Then open **Runs** from the Studio toolbar.

## 2 · Read the evidence

@RunHistory

The Runs panel lists executions on the right: here, one run of *Incoming Support Request* on **Latest**, finished with a green check in **1.85 s**, sixteen days ago, with a time-window filter and two "All" dropdowns above it. Selecting the run opens the log panel below the canvas, with level chips — Debug, Info, Warning, Error, Fatal — and a search box. Two info entries are visible:

- "Received onboarding request" — 120.00 ms — attributed to *Incoming Support Request*.
- "Drafted a helpful reply and queued human review" — 730.00 ms — Token Out: 96, Token In: 184 — attributed to *Draft Helpful Reply*.

Three habits worth stealing from ten seconds of reading:

- **Every entry names its node.** You never have to guess which step spoke.
- **Durations are per step.** 730 ms of this 1.85 s run went into drafting, and the token counts prove a model call really happened.
- **Logs see through layers.** *Draft Helpful Reply* appears nowhere on the top-level board — it lives *inside* the *Prepare Support Reply* layer. The run shows you the machinery the canvas politely hides.

## 3 · Work backward from the first surprise

When the outcome is wrong, resist the urge to rearrange nodes. Evidence first, in this order:

1. Confirm you're looking at the right run — expected payload, and did it use **Latest** or a pinned version?
2. Trace the white path: did execution reach the branch you expected?
3. Check the data inputs of the earliest suspicious node.
4. Read the logs top-down and find the **first** entry that surprises you.
5. Change one thing, then re-run with the *same* payload.

A downstream error is usually the visible consequence of an earlier missing value — the first surprise, not the loudest one, marks the real bug. Re-running with an identical payload holds the input constant, so any change in outcome belongs to your fix.

## 4 · Where did it run?

Location is part of the evidence too. A **local** run executes on your desktop. A **remote** run executes on a configured backend. A **hybrid** Flow can do either — the *same* graph runs locally when you invoke it from Desktop and remotely when a compatible online path calls it. What hybrid never does is split one run across machines.

Some nodes are **local-only** because they need your device: browser control, clipboard, screen inspection, local files. Before a run starts, Flow-Like analyzes the entire graph — including inside every layer — and if any node needs local capabilities, the whole run must execute locally.

> **Watch out:** a green check means "finished", not "correct". A run that succeeds in 1.85 seconds can still send the wrong words to the right customer.

## Recap

- Decide the expected outcome, run with a small synthetic payload, then open the run — not the graph — first.
- Logs attribute every message to its node, time every step, and see inside layers.
- Hybrid means local *or* remote per invocation, never split; one local-only node makes the whole run local.
