Partners drop CSV files into Brightbeam's `partner-uploads/` storage folder. Last month, one file got processed twice — duplicate rows all the way into the dashboard — and another was never processed at all. Nobody noticed either until a partner did.

> **Predict first:** for a folder-watching recipe, what should mark a file as "done" — deleting it, renaming it, a database row, or something else entirely?

## 1 · The chore

Whenever a new file lands in the drop folder, transform it and write the result — reliably, exactly once per file, without a human scanning the folder for "anything new since Thursday."

## 2 · The trigger

There's no push notification for "a file appeared" here — and pretending otherwise is how files get missed. The honest pattern is a **sweep**: the schedule from the report recipe, every 15 minutes, paired with real change detection so each sweep is cheap and exact.

## 3 · The flow

The heart of this recipe is a node pair made for the job:

1. **Storage Dir** — resolves the app's storage as a path; point at `partner-uploads/`.
2. **Diff Directory** — compares the folder against a *manifest* file and emits three arrays: **Added**, **Updated**, and **Deleted**. The manifest is just a path you choose — any name, doesn't need to exist yet — and the node hands you a *session* pin describing what it saw. Change detection has two modes: *Auto* trusts the store's ETags (fast), *Checksum* always hashes contents — the right choice when a backend's ETags are weak, such as local disk timestamps.
3. **For Each** over Added — loop the new files.
4. **Read to String** → transform → **Write String** — process each CSV and write the result; derive the output name from the input name, so reprocessing overwrites instead of duplicating.
5. **Write Directory Manifest** — feed the session in and *commit*: the next diff only reports changes made after this point.

That's the answer to the prediction: neither deleting nor renaming — a **manifest**. The files stay untouched (partners own that folder), and "done" lives in a ledger the diff consults. Delete-as-done destroys the audit trail; rename-as-done rewrites files you don't own.

## 4 · Guardrails

- **Commit after success — this ordering is the whole safety story.** A file that fails halfway is *not yet committed*, so the next sweep diffs it as still-new and retries it for free. Invert the order — commit first, then process — and a failed file vanishes from every future diff: processed-never, reported-done. Write Directory Manifest can also commit *selected* paths, so one bad file doesn't hold the batch hostage: commit the successes, let the failure come back around.
- **Sweeps can overlap.** The report recipe's warning, now with teeth: a 25-minute batch under a 15-minute schedule means two sweeps diffing the same folder before either commits — both will see the same Added files. The manifest can't referee a race; idempotent processing makes the race harmless, because both runs write the same output to the same name.
- **Dry-run by hand first.** Wire the same flow to a Quick Action and click it yourself before any schedule exists:

@QuickActionEvent

That's a Quick Action event from the support app: status Active, execution Local, type Quick Action, pointing at a flow's node with version Latest — a manual trigger wrapped around a flow. Give the watcher one of these, drop two test files in the folder, click, and read the run before a schedule ever does it unattended.

## 5 · Keep it

Decide what Deleted means for you — partners remove files sometimes. Usually the answer is "log it, don't panic." And watch the first week of sweeps in the run history: green-and-boring, with Added counts that match what partners say they sent.

**Recap**

- Diff Directory + Write Directory Manifest turn "scan everything" into "only what changed."
- Commit after success; failures stay uncommitted and retry on the next sweep.
- Overlapping sweeps are survivable only when processing is idempotent.
