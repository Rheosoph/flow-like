2:14 p.m. — Orbit starts answering `429 Too Many Requests`. Your triage flow doesn't notice: it parses the error body as if it were a customer record, and forty tickets get blank context stapled to them before anyone looks up. Nobody wrote a bug this afternoon. The flow just trusted every response equally, and today Orbit had a bad day.

> **Predict first:** Three responses come back from Orbit: `404`, `429`, `503`. Which of them deserves an automatic retry?

## 1 · Sort the failures

Last lesson's rule pays off now: everything non-2xx arrives on the Error path with the Response attached. Wire **Get Status Code** there and branch, because the status tells you *whose* problem it is:

- **4xx — your request is wrong.** The same request will fail the same way forever; retrying is noise. Handle it: a `404` for an unknown email means "no context for this customer", not "try harder". `401` means your credential is bad — a configuration problem, not a timing one.
- **429 — you're the problem, specifically your pace.** The request is fine; slow down and try again later.
- **5xx — their problem.** `503` during a deploy usually clears in seconds. This is the retry case.
- **Transport failure** — no response at all; the node itself errors.

That's the prediction resolved: retry the `503` (and the `429`, after a real pause). The `404` never.

## 2 · Retry politely

A retry loop in flow terms: keep a bounded attempt counter, put a **Delay** node between attempts, and widen the wait each time — one second, then two, then four. Cap the attempts; a flow that retries forever is an outage with extra steps. For calls that hang rather than fail, the **Timeout** node executes downstream work with a time limit and branches on whether it completed.

Rate limits change how you shape whole flows, not just single calls. Pagination is the classic trap: looping "next page" requests at full speed is exactly how you manufacture a `429` from a healthy API. Put the same Delay between pages that you'd put between retries, and let one run's pace leave room for everyone else's.

## 3 · Retry safely

Here's the question that separates careful integrations from cleanup jobs: *what happens if the request succeeded but you retried anyway?* A timeout doesn't tell you the CRM failed — it tells you that you didn't hear back. Maybe the note was written.

Retried GETs are harmless — reading twice changes nothing. Retried writes duplicate. The resolution push from our scenario POSTs a note to Orbit; retry it blindly and a slow deploy turns into two identical notes on the customer's record. Design writes so a retry converges: check whether the note already exists before creating it, or send a client-generated identifier the API can deduplicate when it supports one. When neither is possible, don't auto-retry the write — route the failure to a human instead.

## 4 · Watch it fail

You can't tune any of this blind.

@RunsAndLogs

The screenshot shows the flow board with the log panel open beneath it: severity filters (Debug, Info, Warning, Error, Fatal), log entries with per-step durations, and the node that produced each line — while the Runs panel on the right lists a completed *Incoming Support Request* run with its total time. When a retry loop fires at 2 a.m., this is where you reconstruct what happened. Log the status code and which request it was; never log the token.

## Recap

- Branch on status before anything else: 4xx is yours to fix, 429 is pace, 5xx is theirs to outlast.
- Retries are bounded, spaced with Delay, and backed off — for pagination loops as much as for failures.
- Only idempotent writes may auto-retry; give creates a check or a dedupe key, or hand the failure to a human.
