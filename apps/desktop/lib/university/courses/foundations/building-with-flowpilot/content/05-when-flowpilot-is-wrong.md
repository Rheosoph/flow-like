> **Predict first:** A triage run finishes with a green check, but the customer's reply arrived empty. Which one is lying — the run, or your assumptions?

Neither. A green check means the flow *completed*; it says nothing about whether each step produced what you intended. This is the junior-colleague moment the whole course has been preparing you for: FlowPilot's work is good, fast, and occasionally wrong — and the difference between a mess and a five-minute fix is whether you debug with evidence. This lesson covers catching problems before they land, and running the repair loop when they land anyway.

## 1 · Catch it before Apply

Your first defense is the review stage you already know. Board edits arrive compiled and validated, and the FlowScript workspace shows the generated source, its status, and compiler diagnostics — read them before you click Apply. Validation proves the change is well-formed; only you can tell whether it matches intent. Dismissing a wrong review costs nothing: the board stays exactly as it was. And if you spot the problem after you've already moved on, remember the standing rules — a review that no longer matches the live board goes stale rather than stomping your work, and any deletion of existing items waits for your explicit confirmation.

## 2 · Debug with run context

When a bad change does land — or a good change meets bad data — the flight recorder has the story.

@RunsAndLogs

The Runs sidebar on the right lists the latest execution of **Incoming Support Request** — Latest, 1.85 s, green check. The log panel under the board attributes each message to the node that produced it: "Received onboarding request" (120.00ms) from the event node, then "Drafted a helpful reply and queued human review" (730.00ms · Token In: 184 · Token Out: 96) from a node called **Draft Helpful Reply** inside the drafting layer. Level filters (Debug, Info, Warning, Error, Fatal) and a search box narrow the noise. That's evidence, not vibes: what ran, in what order, how long it took, what it said it did.

Now hand FlowPilot the case file. The board panel can receive run-log context, and FlowPilot uses run context and execution logs to investigate failures:

```text
Why did this run fail? Use the attached log context and suggest a fix.
```

That one line beats ten minutes of describing symptoms from memory — the log already contains the timings, the node names, and the last thing each step reported.

## 3 · Repair, verify, repeat

The repair loop is the build loop with evidence attached:

- **Say what's wrong, specifically.** Reply to the diff like you learned in lesson 3: "Send Reply's Body is empty — trace where the reply text is lost between Human Review and Send Reply."
- **Let FlowPilot repair, not just regenerate.** It can generate *and repair* FlowScript against the current board, so the fix arrives as a normal reviewable change — not a rebuild.
- **Verify with runs, not confidence.** FlowPilot can run safe verification steps and inspect their logs when runtime tools are available. And you can select a historic run and re-run it with the same payload — the exact input that failed becomes your test case.

**Watch out:** an applied fix is not a verified fix. Close the loop: re-run the failing payload and read the log line that used to be wrong. If it now says what you expect, *then* the bug is dead.

Quick recap:

- Review catches wrong-intent changes before they land; diagnostics catch malformed ones for you.
- Debug with the run attached — node-attributed logs are the evidence FlowPilot works best with.
- Prove a repair by re-running the same payload, not by trusting the green check.
