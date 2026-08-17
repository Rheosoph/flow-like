17:24. The fix is in, and Latest runs green in Studio with Friday's payload. Six minutes to spare. Is the incident over?

> **Predict first:** what's the difference between "the board runs" and "the app works"?

The customer. No customer has ever clicked Run in Studio. They reach your flow through an **Event** — a mail trigger, a chat window, a form, a quick action, a schedule. Until the fix has survived that door, you've tested a different thing than the thing that broke.

## Test the real door

The Events documentation puts the rule in one line: activate the Event and **test the complete invocation path**. The Event layer carries configuration Studio runs never touch, and each piece can eat your fix on its own:

- **The version pin.** Your fix lives in the new patch version. Is the Event still pinned to the old one? Then production is still broken, green Studio runs and all.
- **The event node.** After repointing an Event, confirm the event node it targets still exists in that version, and test with representative payloads.
- **The payload.** The real trigger delivers the payload its surface produces — which is the payload your flow must survive, not the tidy one you typed by hand.

## Choose payloads like a scientist

Three runs, minimum, before you call it fixed:

1. **The payload that failed.** Re-run Friday's failing run from the runs list — it replays the recorded payload exactly. Green here proves the fix.
2. **A payload that used to succeed.** Re-run one of the healthy historic runs. Green here proves your fix didn't break what worked — the regression check.
3. **A fresh, representative payload through the activated Event.** Green here proves the whole door: event configuration, version pin, real payload shape.

The runs list makes the first two nearly free — that's the same re-run button from the flight-recorder lesson doing verification duty instead of reproduction duty.

## Where will it actually run?

One more door check, because a fix can change the answer. Some catalog nodes are **local-only** — browser automation, computer control, vision, RPA. They carry a monitor badge on the canvas and need the machine running Flow-Like Desktop. Before every run, Flow-Like inspects every node in the flow, *including nodes inside layers*. One local-only node anywhere marks the entire flow as requiring local execution — a single run is never divided between local and remote workers.

So if your fix quietly added a browser node inside the Prepare Support Reply layer, a Remote Event can no longer run this flow at all, and "Hybrid" won't save you — hybrid means the same flow may run fully locally *or* fully remotely per invocation, never half and half. The pre-run check sees through the layer even if you forgot you put the node there.

## Still one change at a time

Verification discipline is loop discipline. If run 1 fails, you're back to Isolate — with the new fact that your fix was wrong or incomplete. If run 2 fails, your fix broke something else: roll back to your snapshot and rethink. Either way, resist bundling "while I'm here" changes into the verification pass; every extra change turns a clean answer into a maybe.

**Watch out:** a green run proves the flow completed — it doesn't prove the reply landed in the customer's inbox. For side effects that matter, check the durable outcome too, not just the run status.

Recap:

- Studio green isn't done: activate the Event and test the complete invocation path.
- Verify with three payloads — the one that failed, one that worked, one fresh through the real Event.
- Local-only nodes (even inside layers) force local execution; a remote Event can't run that flow.
