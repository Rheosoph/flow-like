A customer named Riko has emailed support: her smart kettle refuses to join the Wi-Fi, and she is "this close" to going back to a stovetop. Somewhere between her email and a helpful reply sits the board you'll live on for this whole course — the **Customer Support Automation** flow.

@StudioCanvas

That's the Studio canvas: one flow, drawn as a graph. Along the top row, four connected nodes carry Riko's request from left to right — Incoming Support Request, Prepare Support Reply, Human Review, Send Reply. Above them float three tinted comments — "1 · Listen for requests", "2 · Draft with AI", "3 · Approve and send". At the bottom, a detached pair (Customer Message and Format Generic Value) minds its own business.

> **Predict first:** when Riko's request arrives, which node moves first? Pick one before reading on.

## 1 · Find the entry

Every run enters through an **event node** — the red one, Incoming Support Request, with the round play badge to its left. Not the leftmost node. Not the comment labeled "1". Events are where the outside world pokes the flow, and execution starts there every single time.

One level up: this flow lives inside an **App**. The app also holds Events, pages, storage, and other flows — none of which appear on this canvas. If you expect the board to show everything the app can do, you'll go looking for things that were never nodes.

## 2 · Follow the two stories

Look closely at the wires. The **solid white** ones link diamond-shaped pins and tell you what runs next: event → Prepare Support Reply → Human Review → Send Reply. The **dashed pink** ones link round pins and carry values: Request into Message, Reply onward into Body. Two wire systems, one canvas. You'll spend lesson 2 on them, but you can already read the whole plot: listen, draft, approve, send.

The comments? Pure documentation. They label the three phases for humans and never execute. Same for node placement — this board reads left to right because a human tidied it, not because the engine sweeps the canvas like a scanner. Drag Send Reply to the far left and save: at runtime, nothing changes. Wires decide, coordinates don't.

## 3 · Open the catalog

Time to touch something. Open any app of your own (a scratch flow is fine), right-click empty canvas, and the **Node Catalog** appears — every operation you can add. Type "mail".

@NodeCatalog

That's the same moment on the reference board: an Actions panel with quick entries for Comment, Event, and Placeholder, a search box with "mail" typed in, and five matching nodes below it — Add Mail Attachment, Copy Mail Message, Send Email, Parse Mailbox, Watch Inbox. Searching the catalog is how boards get built; you'll use the fancier pin-drag variant in lesson 6. Press Escape to close it without changing anything. Congratulations — two minutes in, you've already driven the tool.

> **Watch out:** the one habit to unlearn from day one — canvas position is layout, never scheduling. Moving a node left does not make it run earlier. Every debugging disaster in this course traces back to someone forgetting that.

## Recap

- A run starts at a red event node and follows wires, not screen positions.
- The canvas shows one flow; the app around it holds events, pages, and data you won't see here.
- Right-click opens the Node Catalog; search it by what you want to do.

Next: the parts up close — node families, pins, and why some wires refuse to connect.
