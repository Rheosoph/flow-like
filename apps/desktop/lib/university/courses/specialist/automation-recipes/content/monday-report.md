Every Monday at 8:40, Priya assembles the metrics mail by hand — numbers from a storage file, a quick summary, send to the team before the 9:30 standup. Rushed, occasionally wrong, always resented.

> **Predict first:** the schedule for the automated version is `0 9 * * 1`. When exactly does it fire?

## 1 · The chore

One email, every Monday at 09:00 Berlin time: last week's numbers plus a two-line summary. The content is the easy part. The hard part is *every Monday* — including the ones where the laptop stays in the bag.

## 2 · The trigger

The trigger is a **Cron event** pointed at a Simple Event node in the flow. A cron expression has five fields — minute, hour, day-of-month, month, day-of-week — so `0 9 * * 1` reads: minute 0, hour 9, any date, any month, weekday 1. Monday, 09:00. That's your prediction resolved. (A leading seconds field exists too, but hosted schedulers are minute-precision — stick to five fields when a schedule might run remotely.)

Two settings decide whether this recipe survives contact with reality:

- **Timezone.** Use the IANA name `Europe/Berlin`, never a fixed offset like `+01:00`. A fixed offset can't express daylight-saving transitions; the IANA name shifts with the clocks, and your 09:00 stays 09:00.
- **Execution location.** A *Local* schedule runs inside the desktop app — which means it runs only while the app is open. A *Remote* schedule runs on the hub, laptop-lid-proof. There's also *Hybrid*, which registers **both** schedulers — and can therefore fire twice for the same tick. For a report that must arrive unattended: Remote.

## 3 · The flow

Simple by design:

1. **Simple Event** — the entry node the Cron event targets.
2. **Storage Dir → Read to String** — the metrics live in a file in the app's storage; read it.
3. Format the summary — a transform step shaping the numbers into a short body.
4. **Send Mail** — SMTP Connect plus Send Mail, the same sending pair the triage recipe graduates into. The act step.

While you're building, **Notify User** is a handy stand-in act: it pings the user who executed the workflow — perfect feedback during manual dry-runs, before any mail goes anywhere.

## 4 · Guardrails

- **Give the run a period identity.** Before sending, write the report to storage named after its week — `report-2026-W33.md`. Rerun the same Monday and the file is overwritten, not duplicated; the week's identity makes repeats converge instead of multiply.
- **Missed ticks stay missed.** If the scheduler wasn't running at 09:00 — Local schedule, laptop closed — the occurrence is *not* replayed later. That's a feature to design around, not a bug to wait out: unattended schedules belong Remote.
- **Overlap isn't prevented for you.** Nothing stops tick two from starting while tick one still runs. This report is fast, so it won't bite here — but the habit of asking "what happens at the next tick?" pays off two lessons from now.
- **Dry-run with a one-time schedule.** Cron also accepts a single future date and time instead of an expression. Schedule the report once, two minutes from now, and watch it run for real — then switch to the weekly expression. The event editor also shows a next-run preview; read it before walking away.

## 5 · Keep it

Monday 9:05, you don't open your inbox to check on the recipe — you open Runs:

@RunsAndLogs

That's a flow's run history: the panel on the right lists the latest run with its duration and a green check, and the log pane below filters by severity — Debug through Fatal — showing each step's log line with its timing. The run list is your alibi: report sent, 1.85 seconds, here's the receipt. If Monday's entry is missing, you know *before* the standup does.

**Recap**

- Five fields, IANA timezone — `0 9 * * 1` in `Europe/Berlin` is Monday 09:00, DST included.
- Unattended means Remote; Local dies with the laptop lid, and missed ticks aren't replayed.
- Name the output after its period so a rerun overwrites instead of duplicating.
