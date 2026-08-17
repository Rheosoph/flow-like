Here's a support-desk mystery. Send Reply has a beautiful, fully-formed Body value sitting on its data pin. The customer waits. Nothing sends. No error, no log, nothing. Why?

Because data never schedules anything. If Send Reply's white execution input is disconnected, it will hold that perfect value until the end of time. This lesson is about the two scheduling rules that explain every "why didn't it run?" and every "why did that run?" you'll ever hit.

## 1 · Standard nodes run when control arrives

A **standard node** (and a layer of standard nodes) runs exactly when the white path reaches its execution input. Its data pins say *what* to process, never *when*. On the support board, that's the whole top row: the event fires, then Prepare Support Reply, then Human Review, then Send Reply — one deliberate step after another.

This explicitness is the point. Sending mail, calling a paid model, writing a record — anything with a side effect, a cost, or a failure mode belongs on the white path where its position in the order is visible and reviewable.

## 2 · Pure nodes evaluate when someone asks

@TypedConnections

Now look at the bottom of that shot. Customer Message feeds Format Generic Value through a dashed wire — and Format Generic Value's own Value output connects to **nothing**.

> **Predict first:** during a run of this flow, how many times does Format Generic Value evaluate?

Zero. A **pure node** has no execution pins; it evaluates when a downstream consumer needs its output, and the engine walks the data wires backward to get it. No consumer, no demand, no evaluation. Values are *pulled* by consumers, never *pushed* by producers — Customer Message producing something new doesn't "fire" the formatter, and neither does the run starting.

The corollary: a pure chain can be beautifully typed, carefully arranged, and completely irrelevant. Wiring its final output into a consumer is what brings it to life.

## 3 · Choose the right side for each step

Sort the support flow's steps with one question: *would I care if this ran twice, at a weird time, or not at all?*

- Sending Riko her reply — care a lot. White path.
- Calling the drafting model — costs money, can fail. White path.
- Trimming whitespace off her message — couldn't care less. Pure.
- Joining instructions and question into a prompt string — deterministic, cheap. Pure.

Putting a trim on the execution path isn't illegal, it's noise: it buries the consequential steps in ceremony. Putting a model call in a pure node is worse — you've hidden cost and failure inside "just a calculation" that evaluates whenever some consumer happens to ask.

When behavior surprises you, read the graph twice: once along the white wires from the event (*could control reach it?*), once backward along the dashed wires from the failing input (*could the value get there?*). One of those two traces contains your answer — resist rewiring anything until you know which.

> **Watch out:** don't assume a pure node evaluates once per run. It evaluates on demand — that can be zero times or several, depending on who asks. Anything that must happen exactly once belongs on the white path.

## Recap

- Standard nodes run when execution reaches them; their data pins never trigger them.
- Pure nodes evaluate on downstream demand — unconsumed means never.
- Side effects, cost, and failure live on the white path; deterministic transformations live in pure nodes.

Next: what happens when several corners of the board need the same value — the board's memory.
