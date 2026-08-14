1.85 seconds. That's how long a run of the support board takes, start to finish. In that time the app received a request, drafted a reply with AI, and queued it for human review — and wrote down everything it did. Before you dissect Friday's crash, learn to read the recorder on a healthy flight.

> **Predict first:** does a run record only what went wrong, or does every node get to tell its story?

Every node gets to tell its story. A run isn't an error report — it's a full flight recording, and errors are just its loudest lines.

## The runs sidebar

@RunsAndLogs

That's the support board with the **Runs** sidebar open on the right. Above the list sit a time-window selector and two filter dropdowns. The list shows one run: **Incoming Support Request · Latest · 1.85 s**, with a green check, from 16 days ago. Four facts before you've clicked anything: which event started the run, which flow version it executed (Latest — the editable draft), how long it took, and whether it succeeded.

Every board keeps its previous runs like this. When something breaks on a Friday, you don't start from the board — you start from this list, because the failing run is already in it.

## Reading log lines

Select a run and its log opens in the panel at the bottom of the canvas. Across the top of that panel: five level chips — **Debug, Info, Warning, Error, Fatal** — and a search box, for cutting a noisy log down to the lines you care about.

The run in the screenshot recorded two Info lines:

- **"Received onboarding request"** — 120.00 ms, written by the **Incoming Support Request** node.
- **"Drafted a helpful reply and queued human review"** — 730.00 ms, with **Token Out: 96, Token In: 184**, written by a node called **Draft Helpful Reply**.

Look at what each line carries beyond its message: the node that wrote it, how long that step took, and — for AI steps — token usage. That's three diagnoses for free. A message turns into a *location* (the node name), durations expose the slow step (730 ms dwarfs 120 ms — the AI draft dominates this run), and token counts tell you what the model actually consumed and produced.

One more detail worth noticing: **Draft Helpful Reply** doesn't appear on the canvas above. It lives *inside* the collapsed "Prepare Support Reply" layer — and it logs under its own name anyway. The recorder sees through layers. Hold that thought for the next lesson.

## Re-run: reproduce for free

Here's the feature that turns the runs list from an archive into a debugging tool: you can select a historic run and **re-run it with the same payload** — to debug an error, observe total execution time, or test a change you've made to the flow.

That is step 1 of the loop, industrialized. Friday's failing customer message is not something you reconstruct from memory or ask the customer to resend. It's saved, attached to the failed run, one click from replaying. Reproduce stops being an investigation skill and becomes a button.

**Watch out:** if the log panel looks suspiciously empty, check the level chips before concluding the run wrote nothing — filtering to Error hides the Info story around the failure. Which lines get written *at all* is decided by the board's log level; that's Lesson "Log deliberately".

Recap:

- The runs sidebar lists each run with its event, version, duration, and status.
- Log lines name their node and duration — location, timing, and token evidence in one place.
- Re-running a historic run replays its exact payload: reproduction as a button.
