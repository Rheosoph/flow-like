You send a test request through the new surface and a reply comes back. Lovely. Now answer one question: *can you prove which nodes ran, with what data, for how long?* "It seemed to work" survives exactly until Monday morning. Evidence survives arguments.

@RunsAndLogs

This is the support board wearing its evidence. On the right, the **Runs** panel lists every execution — here one entry: *Incoming Support Request*, on Latest, finished in **1.85 s** with a green check, 16 days ago, with filters for time window and status above it. Along the bottom, the **log panel** for the selected run: severity filters (Debug through Fatal), a search box, and two entries —

- *"Received onboarding request"* — 120 ms — attributed to **Incoming Support Request**
- *"Drafted a helpful reply and queued human review"* — 730 ms, **184 tokens in, 96 out** — attributed to **Draft Helpful Reply**

Look at that second attribution. Draft Helpful Reply isn't visible on the outer board at all — it lives *inside* the collapsed Prepare Support Reply layer. Logs attach to the node that actually did the work, however deep it's folded. You just verified the layer's internals executed without opening the layer. Notice also what the numbers alone tell you: which step is slow (the model call, by 6×) and what it consumed.

## 1 · Test like a skeptic

Write the expected outcome *before* sending each message, and cover more than sunshine:

1. **Happy path** — a short, clear request ("kettle won't join Wi-Fi").
2. **Boundary** — a very long, oddly formatted rant.
3. **Safety** — something the flow should refuse or route to a human, if you've built that.
4. **Failure** — a deliberately broken configuration, when you can trigger one safely.

Synthetic text only. Test inputs end up in logs, and logs get read by teammates, so no real customer data and — pointing back at lesson 4's sermon — never a real credential.

## 2 · From symptom to node, in order

When an answer is wrong, resist the itch to rewire. Walk the ladder:

1. Confirm which flow *and which version* the Event actually invoked.
2. Find the matching run in the Runs panel.
3. Read the white story in the logs: which nodes ran, in what order, how long?
4. Trace the data story into the first node whose output is wrong.
5. Only now, edit — the smallest plausible change.

> **Predict first:** where does "move the drafting node earlier on the canvas" appear on that ladder?

Nowhere, ever. Position isn't scheduling — the rule from lesson 1 holds under debugging pressure too, which is precisely when people forget it.

## 3 · Rerun the same payload

After your one small fix, don't type a fresh test message first — **rerun the historic payload** from the Runs panel. Same input, one changed node: any difference in output is your fix (or your fresh bug), cleanly isolated. A new message would change two things at once and tell you nothing.

One honest caveat: if the flow calls an external model, reruns aren't perfectly deterministic — treat the rerun as a strong comparison point, not a regression proof. When it passes, run the broader test set, and only then cut the numbered version lesson 7 told you to pin.

> **Watch out:** logs are written by nodes, and you control what explicit logging emits. Log stage and shape ("draft was empty"), not payload contents you'd regret archiving.

## Recap

- Every run leaves evidence: entry, duration, status, and per-node logs — attributed even inside collapsed layers.
- Diagnose in order: version → run → white story → data story → smallest edit.
- Rerun the same payload after a fix to compare like with like; then widen the test set.

One module left. The reference board is about to arrive broken, and you get to be the one who knows why.
