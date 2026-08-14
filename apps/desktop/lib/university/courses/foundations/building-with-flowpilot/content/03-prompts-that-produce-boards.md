"Make my support app better." Send that to FlowPilot and it will do *something*. The odds that it's your something are low — not because the model is weak, but because the sentence carries no target, no data, and no way to check the result. This lesson is prompt craft for boards: scope one change, name the inputs and outputs, and iterate on the diff instead of arguing with paragraph one.

## 1 · Scope one change

The formula from the docs is short: give FlowPilot **an outcome and enough context to verify it**. Three real asks that earn their keep on the triage project:

```text
Explain this workflow and point out where errors are handled. Do not change it.
```

Read-only recon. FlowPilot describes the board without touching it — always a safe first move on a board you didn't build.

```text
Build a webhook workflow that validates the payload, stores it,
and returns a useful error response.
```

One entry point, three verifiable behaviors. Notice what's absent: no node names dictated, no wiring micromanaged. FlowPilot finds appropriate nodes in the live catalog itself.

```text
Why did this run fail? Use the attached log context and suggest a fix.
```

The debugging ask — evidence attached. Lesson 5 is built around it.

## 2 · Name inputs and outputs: the makeover

Back to the bad prompt. "Make my support app better" — better how? Faster? Kinder replies? Fewer escalations? FlowPilot has to guess all three. Here's the makeover, scoped to one requirement of our triage app:

```text
On this board, after the Prepare Support Reply layer, add a step that
flags messages mentioning refunds and routes them into Human Review
instead of straight on to Send Reply. Keep Send Reply unchanged.
```

Why this works: it anchors to named nodes ("after Prepare Support Reply"), names the data it acts on (messages mentioning refunds), names the destination (Human Review), and states a don't-touch constraint (Send Reply). Nothing to guess, everything to verify — you've effectively written the review checklist inside the prompt.

@FlowLikeStudio

You can read the same discipline off the board itself. The three comment labels — "1 · Listen for requests", "2 · Draft with AI", "3 · Approve and send" — each name one scoped job, and the gray note on Human Review ("prototype a future review step before implementing its internals") is exactly the kind of stated constraint a good ask carries. Boards built from scoped prompts stay this legible; boards built from "make it better" don't.

## 3 · Iterate on the diff

The first result won't always be perfect, and that's fine — the loop continues with full context. The skill is *replying to what you see in the review* instead of rewriting your original prompt from scratch:

- "The refund check runs before the draft is written — move it after Prepare Support Reply."
- "Flagged messages should land in Human Review, not skip it."

Selection is targeting. Inside the board editor, FlowPilot receives the current board, the layer you're in, and your **selected nodes** — so select the two nodes in question before you ask, and "these two" resolves itself. Small, named, verifiable corrections converge in one or two rounds; vague restarts begin the guessing game again.

Quick recap:

- One scoped change per ask, anchored to named nodes and named data.
- Say what must *not* change — that's half the review done in advance.
- Refine by replying to the diff with the relevant nodes selected, not by re-litigating prompt one.
