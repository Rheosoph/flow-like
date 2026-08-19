Your flow works when you press Run. Now a scheduler needs to press it at 3 a.m. — who's going to type the password?

That question decides everything in this lesson. UI events always have a human present: someone to fill a form, read an error, retry. The support app's next surfaces — a partner webhook and an hourly reconciliation — run with nobody watching. Every configuration choice has to answer "and what happens when no one's there?"

## 1 · API: one guarded door

The partner portal wants to submit tickets programmatically. That's an **API** event: one configured HTTP endpoint — method, path, exposure, expected input and output, size limits, error codes — running locally or remotely. Pair it with a Generic Event node when the request payload must map into the flow, or a Simple Event node when triggering is enough.

Two disciplines make it production-grade:

- **Authorize before side effects.** Check the caller before creating a ticket, not after.
- **Expect retries.** Clients and proxies resend requests. An idempotency key on ticket creation means a retry finds the existing ticket instead of minting a twin.

And never return internal exception text to a partner — safe error out, details into logs.

## 2 · Cron: schedule plus overlap policy

The hourly reconciliation is **Cron** on a Simple Event node, local or remote. The schedule is the easy half. The grown-up half is the policy sheet:

- What's the time basis, the expected duration, the missed-run behavior?
- What happens when the previous run is *still going* at the next tick — skip, queue, or run concurrently? Concurrency is only legal if the flow can't double-process a period.
- Give each period a deterministic identity in data, so catch-up and retries converge on one completion record instead of stacking duplicates.

Remote schedules also need server-available credentials — remember, local Secrets don't travel.

## 3 · Daemon: the long-runner, not the escape hatch

A **Daemon** supervises a long-running *local* flow and can restart it after failure with bounded delays. Use it for persistent local integrations that can't be modeled as finite periodic work — a connection that must stay open, a watcher that must never sleep.

What a Daemon is *not*: a way to dodge writing a schedule. "Run it as a Daemon so we never miss a tick" buys you a resource-consuming process that still needs health signals, checkpoints, and reconnection logic. Finite hourly work is clearer, cheaper, and safer as Cron.

## 4 · REST and MCP, one paragraph each

**REST** turns flows into a remote, multi-endpoint, authenticated service surface. **MCP** exposes them as a Model Context Protocol server for tool-capable clients. Both are remote-only, broader contracts than a single API event, and deserve access policy, versioning, and monitoring. Internal consumers can reach them through connected-app infrastructure without public exposure; going public needs a threat and abuse review first.

## 5 · Prove it ran — then prove it worked

@RunsAndLogs

This view replaces the watching human. The board shows the support flow's three stages — *1 · Listen for requests*, *2 · Draft with AI*, *3 · Approve and send* — with the **Runs** panel on the right listing a completed "Incoming Support Request" run (Latest, 1.85 s) and the log console below it filtering by severity: two info entries, each with its duration and the node that emitted it. Every run leaves this trace, whether a button, a webhook, or a schedule triggered it.

Your pre-release drill for any unattended event: invoke it through the real configured path with a safe payload or temporary schedule, confirm the run targets the intended flow version, inspect inputs, outputs, duration, and terminal status here — then go look at the *actual outcome*. The database row. The emitted response. The completion record.

> **Watch out:** a green terminal status is not a business outcome. The reconciliation can "succeed" while writing nothing. Runs and Logs plus the durable record — always both.

## Recap

- API = one endpoint with authorization-before-side-effects and idempotent retries.
- Cron = schedule + explicit overlap policy + deterministic period identity; Daemon = supervised local long-runner, never a scheduling dodge.
- Verify unattended events through the real path, in Runs and Logs, *and* in the durable outcome.
