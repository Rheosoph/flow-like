A teammate leans over your shoulder, looks at the support board, and asks: "So *Customer Message* is where it starts, right? It's the customer's message." A fair guess — and wrong, twice over. This lesson gives you a four-pass reading method that settles questions like that in seconds, using the same board as last time.

@StudioCanvas

You're looking at the copilot's main Flow again: the four-node chain under the numbered comment blocks, the dashed data wire running beneath it, and the two amber nodes at the bottom.

> **Predict first:** When does *Format Generic Value* — the amber node at the bottom right — actually run?

## Pass 1 · Trigger: find the entry

A run enters the graph at an **event node**. Here that's *Incoming Support Request*: orange-red header, play button on its left edge. *Customer Message*, despite its name, can't start anything — look at its edges. No diamond pins means no place for execution to begin.

## Pass 2 · Transform: follow the white wires

From the event, the white wires visit *Prepare Support Reply*, then *Human Review*, then *Send Reply*. That order is a fact about wires, not geography — drag *Send Reply* to the far left of the canvas and it still runs last. Never infer execution order from placement.

## Pass 3 · Data: trace the dashed wires

Now the quieter story. *Request* feeds *Message* at the first layer. Then look closely at the long dashed wire: the drafted *Reply* leaves *Prepare Support Reply* and travels **straight past Human Review** into *Send Reply*'s *Body*. Human Review gates *when* sending happens; the reply text itself never passes through it. That's a real design detail the layout almost hides — exactly the kind of thing pass 3 exists to catch.

The amber nodes are **pure nodes**: round data pins only, never on the white path. The runtime evaluates a pure node when — and only when — a downstream consumer needs its output. Which resolves the prediction: *Format Generic Value* runs when something asks for its *Value*, and on this board nothing does yet, so it doesn't run at all. A pure node is a data dependency, not a step.

## Pass 4 · Outcome: name the result

What makes a run worth having? Here, *Send Reply* — the envelope — delivers the approved answer. Now compress all four passes into one sentence:

> When **an incoming support request** arrives, the flow **drafts a reply and routes it through human review**, using **the request message**, then **sends the reply to the customer**.

If you can write that sentence for a board, you understand the board.

## Comments and layers are the map, not the territory

The numbered blocks (*1 · Listen for requests*, *2 · Draft with AI*, *3 · Approve and send*) label subgoals, and the grey notes explain intent — one says "Two implementation steps are grouped into one reusable layer," telling you *Prepare Support Reply* is a **layer** hiding nested steps behind its pins. Read the top level first; open a layer only when you need its internals. And trust comments the way you trust any documentation: verify their claims against the wires.

> **Watch out:** numbered Flow versions are immutable. If Studio won't let you edit, you're viewing a version — switch to **Latest** first.

Try the four passes on any Flow of your own and write its sentence. Twenty seconds per pass is plenty.

## Recap

- Read in order: trigger → white wires → dashed wires → outcome, then compress it into one sentence.
- Execution order comes from wires, never from left-to-right placement.
- Pure nodes run on demand when their value is needed — or never, if nothing asks.
