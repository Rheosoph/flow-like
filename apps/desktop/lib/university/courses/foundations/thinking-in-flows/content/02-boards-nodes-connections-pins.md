Click a node and the board talks back. In the shot below, Incoming Support Request is selected: an orange outline wraps it and a small toolbar pops up above it — edit, comment, info, deactivate, pin, copy, delete.

@NodeTypes

> **Predict first:** count the different node styles on this board. How many distinct jobs can you spot before reading the list?

## 1 · Node families

Four families share the support board:

- **Event nodes** — Incoming Support Request, red header, link icon. The entry point; lesson 1's "who moves first" answer.
- **Standard nodes** — Send Reply, with its envelope icon and diamond execution pins. Standard nodes do consequential work and run when the white path reaches them.
- **Pure nodes** — the amber pair at the bottom, Customer Message and Format Generic Value. No diamond pins at all. They compute values on demand; lesson 3 is theirs.
- **Layer nodes** — Prepare Support Reply and Human Review, marked with a lightning-bolt badge. Each hides a nested graph behind a typed boundary; lesson 5 opens them up.

Plus the grey comment boxes, which annotate and never run. Color and icons are fast cues, but the pins are the truth: a node's pins tell you exactly what it needs, what it produces, and whether it takes part in execution.

## 2 · Two wire systems, one canvas

@TypedConnections

Trace both systems in that shot. The **solid white wires** connect diamond pins along the top row — they answer "what may run next?" and nothing else. The **dashed pink wires** connect round pins — Request to Message, Reply across to Body, and Message to Generic Value on the pure pair below. They answer "which value feeds this input?" and their color announces the type (pink is String).

Studio enforces the split while you author: execution only connects to execution, data only to data of a compatible type, and structured values must also agree on schema. This is why some connections simply refuse to happen — the board is rejecting a bug you haven't run yet.

Keep the two stories separate in your head, because they fail separately. A standard node can hold perfect data forever and never run (no execution in). A white path can arrive at a node whose data input is empty. Different symptoms, different wires to trace.

## 3 · Generic pins commit on contact

Now select the pure formatter:

@GenericPinTypes

Format Generic Value is selected here, and its pins look different: Generic Value in and Value out are a muted grey-blue, while the Customer Message pin feeding it is pink. Grey-blue marks a **Generic** pin — the node works with many types and hasn't fully committed. Connect a concrete type and the generic resolves to it; from then on, downstream connections must match what was locked in.

That's type inference, not a warning state. If a connection that used to be offered suddenly isn't, check whether an earlier wire already resolved the generic to something else.

> **Watch out:** an input pin and an output pin can carry the same value but they're never interchangeable — direction matters as much as type. Read the pin side before you blame the type.

## Recap

- Four node families: event, standard, pure, layer — pins are the reliable tell, diamonds mean execution.
- White solid wires schedule; dashed colored wires carry typed values; the two fail independently.
- Grey-blue generic pins adapt to the first concrete type you connect, then hold everyone downstream to it.

Next: the scheduling rules themselves — who runs, who merely evaluates, and when.
