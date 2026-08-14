The log says `✗ Send Reply — body is null`. Case closed — Send Reply is broken, replace it?

> **Predict first:** is the node named in the error actually the broken one?

Usually not. Send Reply is the *victim*: it was handed a null body and refused to send an empty email — which is exactly what you want it to do. The null was manufactured somewhere upstream. Finding that "somewhere" is a three-step read: trace → node → pin.

## The map

@FlowLikeStudio

That's the whole support board. Three comment labels name the stages — **1 · Listen for requests**, **2 · Draft with AI**, **3 · Approve and send** — and below them runs the execution chain: **Incoming Support Request** (the event node) connects through solid white wires to **Prepare Support Reply**, then **Human Review**, then **Send Reply**. Underneath the white wires, dashed wires carry the data: *Request* feeds *Message*, and a *Reply* value travels along the chain until it reaches Send Reply's *Body* input. At the bottom of the canvas sits a separate pair — **Customer Message** connected by a dashed wire to **Format Generic Value** — with no white wires at all.

Two kinds of wires, two different questions:

- **Solid white wires** answer *"in what order do nodes run?"*
- **Dashed wires** answer *"where did this value come from?"*

They are not the same question. A node's execution predecessor and its data supplier can be entirely different nodes. When a *value* is wrong, the white wires are the wrong map to stare at.

## Trace → node → pin

Apply it to Friday:

1. **Trace:** the run log names the failing node — Send Reply.
2. **Node:** find it on the canvas. Its white wires tell you it ran after Human Review. Interesting, but you're not chasing *order* — the flow ran fine until here.
3. **Pin:** find the input holding the bad value — *Body*. Now follow *Body*'s dashed wire backward to whoever produced that value, and read *that* node's log lines and outputs.

The bug lives at the producer end of the dashed wire, or further upstream of it. Repeat the read until you find the node where good data went in and bad data came out. That node is your suspect.

## Layers hide nodes, not evidence

Prepare Support Reply and Human Review aren't single nodes — they're collapsed **layers**. The canvas notes say it plainly: two implementation steps are grouped into one reusable layer, and the review step is a prototype awaiting its internals. If your dashed-wire trace points into a layer, open it and keep tracing — remember Draft Helpful Reply from the last lesson, logging by name from *inside* Prepare Support Reply. The run log never stops at a layer boundary, so your trace shouldn't either.

## Pure nodes: the quiet suppliers

Now the bottom pair. **Customer Message** and **Format Generic Value** have no white execution pins at all — they're **pure nodes**. A pure node runs automatically whenever a downstream node needs its output; it never appears on the white execution path. That has a sharp debugging consequence: if a pure node produces a wrong value, no amount of stepping along the execution chain will visit it. The *only* road that leads to a pure node is a dashed data wire — one more reason trace → node → pin follows data, not execution.

The rest of the cast, for completeness: event nodes (like Incoming Support Request) are the red entry points that start a run, standard nodes run only when their incoming execution wire fires, and comment labels like "2 · Draft with AI" never execute — they're annotation, not machinery.

**Watch out:** the node that errors is where the flow *stopped*, not necessarily where the bug *lives*. Replacing the victim leaves the culprit in place — and Monday's incident will introduce you to it again.

Recap:

- White wires order execution; dashed wires carry values — a wrong value means follow the dashed wire.
- Trace (log names the node) → node (find it) → pin (follow the bad input backward to its producer).
- Collapsed layers and pure nodes are still reachable: open the layer, follow the data.
