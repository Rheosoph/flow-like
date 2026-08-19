It's Monday, 8:55 a.m. Priya — the other half of your two-person ops team at Brightbeam — opens her checklist: sort the support inbox, assemble the weekly metrics mail, answer the "is the deploy done?" pings in chat, process the partner CSV uploads, and copy new signups into the CRM. Ninety minutes of clicking. Every week. Forever.

This course deletes that list one recipe at a time — and every recipe follows the same four-beat arc.

> **Predict first:** which of the five chores would you automate first? Hold that thought — by the end of this lesson you'll have a rule for picking, and it's probably not "the most annoying one."

## 1 · The arc

Every automation worth keeping has the same shape:

**Trigger → transform → act → verify.**

- **Trigger** — something starts the run: a schedule fires, a mail arrives, a chat message lands, a request hits an endpoint.
- **Transform** — the flow turns raw input into a decision or a payload: classify the mail, build the report, parse the command.
- **Act** — the one step with a side effect: move the message, send the mail, write the file, call the API.
- **Verify** — prove the outcome exists: check the run, check the record, check the file.

@CourseBanner

That's the whole course in one picture: a mail card, a clock card, and a chat card — three different triggers — all wiring into the same kind of flow, and the flow ending in a green check. The trigger changes from recipe to recipe. The arc doesn't.

## 2 · See it on a board

Here's the arc on a real board — a support pipeline built in Flow-Like Studio:

@FlowLikeStudio

Three tinted comment lanes name the beats across the top: "1 · Listen for requests", "2 · Draft with AI", "3 · Approve and send". Underneath, the node chain runs Incoming Support Request → Prepare Support Reply → Human Review → Send Reply. The solid white line between them is execution order; the dashed line below it carries data — the request text riding along until it becomes the reply body. Anyone opening this board reads the arc before reading a single node.

Steal one more thing from this board: **Send Reply is the only node that touches the outside world.** Trigger and transform steps can rerun harmlessly all day. The act step is where mistakes become emails. Keep one act step per recipe when you can, and put it late.

Events — the machinery that connects schedules, chats, and endpoints to boards — have their own course. Here we treat them as ingredients: when a recipe needs a trigger, you'll configure it in the Events workspace and move on.

## 3 · Two habits

Two habits separate automations you keep from automations you disable in week two.

**Runs happen twice. Plan for it.** Schedules overlap, webhooks retry, teammates double-click. An *idempotent* recipe produces its outcome once no matter how many times it runs — the second run recognizes the work is already done and adds nothing. Every recipe in this course carries its own dedupe trick: a "seen" flag, a file named after the week, a change manifest, a dedupe key. You'll meet each one where it earns its keep.

**Dry-run first, verify the outcome.** Before a recipe touches anything real, run it by hand against safe targets — a draft instead of a send, a copy instead of a move — and read the run log. A green status only proves the run finished. The finish line is the *durable outcome*: the moved mail, the written file, the CRM record you can point at.

Try it right now, no product required: take your own most annoying chore and write its four beats on one line. If you can't name the trigger or the act, it's not ready to be a recipe — that's the diagnosis, not a failure.

And your prediction? Automate first the chore with the clearest trigger and the most reversible act. For Brightbeam that's the inbox: mail arriving is a clean trigger, and filing a mail into the wrong folder is a two-second fix. Sending the wrong report to the whole company is not. The inbox is next lesson.

> **Watch out:** a recipe that isn't safe to run twice isn't finished — it's an incident with a schedule.

**Recap**

- Every recipe: trigger → transform → act → verify — one act step, placed late.
- Idempotency is non-negotiable: the second run must add nothing.
- Verify the durable outcome, not the green status.
