Thursday, 6:04 a.m.: the run fails. The portal took nine seconds to render and your five-second delay gave up waiting. Friday, 6:03 a.m.: the run goes green — and half the spreadsheet holds Wednesday's numbers, because the flow read the table before the portal refreshed it. Two failures in two days. The loud one cost a re-run. The silent one fed stale data into a purchasing decision.

> **Predict first:** which run is more dangerous — the one that failed, or the one that "succeeded"?

## Wait for signals, not seconds

A fixed delay is wrong twice: too short on the portal's slow days, wastefully long on the fast ones. Waits should watch for the thing you actually need — **Wait For Selector** for the status table, **Wait For Template** for the Export dialog, **Wait For Network Idle** when a page loads data in bursts and no single element marks "done." Reserve **Delay** for deliberate pacing, like throttling your clicks to be polite to a fragile portal. Never use it as a readiness signal.

## Bound it, then retry what's safe

Every consequential operation deserves a ceiling: **With Timeout** turns "hangs forever" into a failure you can route. For flaky steps, **Retry Loop** retries with a maximum attempt count, an initial delay, and a backoff strategy — Constant, Linear, or Exponential — so a struggling portal gets breathing room instead of a hammering.

But retry *what*, exactly? Re-reading the status table is safe to repeat; so is waiting again for a template. Clicking a button that *does* something — submit, confirm, acknowledge — is not, unless you can first verify whether the earlier click already landed. The docs' rule, worth engraving: avoid retrying a destructive or externally visible action unless it's idempotent or you can check whether it already succeeded.

## Assert, so change fails loudly

Kestrel will redesign the portal eventually, and they will not email you first. Your defense is asserting landmarks. Before trusting the status table, verify something you know: the table's header text, the PO column label. On the visual side, **Assert Template Exists** and **Assert Color At Position** do the same job for desktop surfaces. If the landmark is gone, the layout changed, and the flow should stop — loudly. A wrong-looking page that fails the run at 6 a.m. is a gift; a wrong-looking page that gets extracted anyway is Friday's silent corruption.

For the failure paths, the RPA family gives you structure: **Try Catch** to route errors, **Error Recovery** and **Diagnose Failure** to react to them, **Save Checkpoint** and **Take Snapshot** to record how far the flow got and what the screen looked like, **Log Action** to leave an audit trail a colleague can follow.

## Watch the run

@RunsAndLogs

That's the board after an execution: the Runs panel on the right lists the latest run — 1.85 seconds, green check — and the log panel below filters by level (Debug, Info, Warning, Error, Fatal) and pins every message to the node that wrote it, with per-step timings. That attribution is how you tell "the wait timed out" apart from "the extraction returned nothing" at a glance. The Debugging course goes deeper; for the 6 a.m. run it's enough that every morning leaves evidence you can read at nine.

And when a target does drift, climb the fallback ladder deliberately — the same order the overview card's Target panel lists: stable selector or accessibility element first, then an alternate selector or stored fingerprint, then a template match, then a coordinate, with LLM-assisted resolution last. Each rung is less deterministic than the one above it. Fall, don't jump.

## Recap

- Wait for signals, never seconds — and give every operation a timeout.
- Retry only what's safe to repeat; verify before repeating anything consequential.
- Assert landmarks so change fails loudly, and let Runs and Logs keep the evidence.
