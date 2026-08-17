Friday, 16:58. You're on call for the support app. A customer wrote in at 16:18; no reply ever went out. Your team is gone for the weekend, you have thirty minutes, and everything you need is in the evidence below. This lesson teaches nothing new — it hands you the pager.

## The system

The board is **Customer Support Automation**: an **Incoming Support Request** event node connects along the white execution chain to **Prepare Support Reply** (a collapsed layer whose nodes include Draft Helpful Reply), then **Human Review** (a collapsed layer), then **Send Reply** — a mail node whose *Body* input is fed by a dashed data wire from upstream. A pure pair, **Customer Message → Format Generic Value**, sits below the chain with no execution pins. Customers reach the flow through the app's support Event, which is **pinned to a numbered flow version**.

## The evidence

**Exhibit A — today's run, 16:41.** Status: `run failed · 1.85 s`. The log holds one substantive line:

```
✗ Send Reply — body is null
```

Almost nothing precedes it — no draft milestone, no review milestone, no intermediate values.

**Exhibit B — a healthy run, 16 days ago.**

@RunsAndLogs

The runs sidebar shows that run — Incoming Support Request, Latest, 1.85 s, green check — and its log tells a story in two Info lines: "Received onboarding request" (120.00 ms, Incoming Support Request) and "Drafted a helpful reply and queued human review" (730.00 ms, Token Out: 96, Token In: 184, Draft Helpful Reply). Node names, durations, token counts — the recorder was rich back then.

**Exhibit C — Manage Board, opened just now.** Version reads **Latest (1.0.0)**. The board's **Log Level** is set to **Error** — someone tidied it up two weeks ago, "for production."

**Exhibit D — the runs list.** Both runs are still selectable, today's failure and the healthy one, each with its recorded payload.

## The constraints

- The support Event stays pinned; other surfaces call this flow, and your fix must not change how any caller uses it.
- You get the fix wrong once and there's no time for a second rewrite — every irreversible step needs a fallback.
- The incident closes only when you can *prove* the customer's message gets a reply, not when a run happens to go green.

## Your task

Answer the questions below as the person holding the pager. Every answer is derivable from the exhibits plus what you've practiced: the debug loop, the flight recorder, trace → node → pin, deliberate logging, version snapshots, and testing through the real door. The clock reads 17:00. Go.
