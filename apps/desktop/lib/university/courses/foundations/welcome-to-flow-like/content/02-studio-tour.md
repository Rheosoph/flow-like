Here it is — the board this whole course reads: the Customer Support Copilot's main Flow, exactly as Studio shows it.

@StudioCanvas

Three tinted comment blocks name the plan across the top: **1 · Listen for requests**, **2 · Draft with AI**, **3 · Approve and send**. Under them runs a chain of four nodes — *Incoming Support Request → Prepare Support Reply → Human Review → Send Reply* — linked by solid white wires. A dashed pink wire runs in parallel just below the chain, and at the bottom of the canvas two amber nodes (*Customer Message → Format Generic Value*) sit connected to nothing above them. A toolbar floats top-center, an "Offline" badge sits top-right, zoom controls bottom-left, and a minimap bottom-right.

> **Predict first:** Two wires leave *Prepare Support Reply* — a solid white one and a dashed pink one. If you deleted the pink one, would *Send Reply* still fire on the next run?

## 1 · Two kinds of wires, two stories

Every board tells two stories at once.

The **solid white wires** are the control story. They connect the diamond-shaped **execution pins** on each node's shoulders and answer one question: who runs, and in what order. Reading them here: the request arrives, a reply is prepared, a human reviews, the reply goes out.

The **dashed colored wires** are the data story. They connect the round, colored **data pins** and carry typed values: *Request* flows into *Message*, and the drafted *Reply* travels across to *Body*. The color encodes the value's type.

So, the prediction: delete the pink wire and *Send Reply* still fires — the white wire alone decides that. It just fires without the drafted reply, because the value that should have arrived at *Body* no longer does. Control and data are separate systems, and most confusing boards become readable the moment you untangle the two.

Flow-Like guards both systems while you author: an execution pin won't connect to a data pin, and incompatible data types refuse to wire together. A whole class of mistakes dies before the first run.

## 2 · Node roles, by header

- **Incoming Support Request** wears an orange-red header and has a round play button on its left edge: an **event node**, the graph's entry point.
- **Prepare Support Reply** and **Human Review** share that orange-red header style but are **layers** — grouped sub-steps behind one named boundary. The grey sticky note above the first says it plainly: "Two implementation steps are grouped into one reusable layer."
- **Send Reply**, with the envelope icon and a neutral header, is a **standard node**: it sits on the execution path and does one job.
- The two amber-gold nodes at the bottom, **Customer Message** and **Format Generic Value**, are **pure nodes** — data pins only, not a diamond in sight. They star in the next lesson's prediction.
- The tinted blocks and grey notes are **comments**. They never execute.

## 3 · The Node Catalog

Where do new nodes come from? Right-click any empty patch of canvas.

@NodeCatalog

The catalog opens under a red **Actions** header with three quick entries — Comment, Event, Placeholder — and a search box. In the screenshot, typing "mail" surfaces *Add Mail Attachment*, *Copy Mail Message*, *Send Email*, *Parse Mailbox*, and *Watch Inbox*. Later you'll learn the power move: dragging a wire into empty space opens this same catalog pre-filtered to nodes compatible with that pin.

## Try it in any app of your own

Open any Flow you have — a scratch one is fine, and if your Library is still empty, everything here works the moment you create your app in lesson 5:

1. Pan and zoom, then find your way back with the minimap.
2. Select one node and read its pins: diamonds for execution, colored circles for data.
3. Follow one white wire and say out loud which node runs first.
4. Right-click the canvas, search "mail", then press Esc without adding anything.

Nothing in this list edits your graph.

## Recap

- Solid white wires carry execution order between diamond pins; dashed colored wires carry typed data between round pins.
- Headers reveal roles: orange-red events and layers, neutral standard nodes, amber pure nodes, comments that never run.
- Studio blocks incompatible connections while you author — the type system is your first safety net.
