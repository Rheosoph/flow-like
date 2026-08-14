Enough spectating. In this lesson you rebuild the support board's spine — listen, draft, send — in an app of your own, using the reference screenshots as your blueprint. Keep @StudioCanvas open in your head as the target shape: event → drafting step → review → send, white spine on top, dashed data underneath.

Open one of your own apps (or create a scratch one) and add a flow. Any app works; you need nothing pre-installed beyond the standard catalog.

## 1 · Listen — plant the entry

Every flow starts at its event. Right-click the canvas, pick **Event** from the catalog's quick actions, and choose an event you can actually trigger from your setup — a chat or simple manual event is perfect for practice; the reference board listens for incoming support mail. Give it a home on the left simply because your future self reads left to right. The engine, as you know by now, doesn't care.

## 2 · Draft — build from the pin outward

Here's the professional move, and the reason this lesson exists. Don't hunt the catalog blind — start from the value you already have.

@DrawPin

Grab the event's output pin — the customer's message, a String — and drag into open canvas. That thin wire trailing across the board in the shot is the gesture mid-flight. Release on empty space, and:

@TypedCatalogSuggestions

The catalog opens *pre-filtered*. Notice the differences from lesson 1's right-click version: a **Context Sensitive** toggle sits checked in the header, a **Create Variable from Pin** shortcut has appeared, and the "mail" search now returns four nodes instead of five — Watch Inbox is gone, because it can't connect to the String pin you're holding. Every suggestion is guaranteed compatible, and choosing one wires it automatically.

> **Predict first:** the wire you dropped carried a String. If you'd dragged an execution diamond instead, what kind of suggestions would you get?

Only nodes that can join the execution path — the filter follows the pin. Use this to assemble your drafting step: from the message pin, add whatever text or formatting node your catalog offers, then a response-producing step. Prompt assembly — joining fixed instructions with the customer's text — is deterministic and cheap: pure node, fed into the drafting step's input. The drafting step itself (a model call in the reference board) costs money and can fail: white path, no debate. That's lesson 3 earning its keep.

## 3 · Send — close both stories

Now finish like the reference board does: an output step (Send Reply there; use any node that shows or sends a result in your app) at the end of the white spine. Then walk both stories deliberately:

1. **Execution:** event → draft → send, one unbroken white path. Trace it start to finish.
2. **Data:** work *backward* from each input. Send's body ← draft's output. Draft's input ← your prompt assembly ← the event's message.

Before you press play, predict what a missing piece would do. Forget a data wire into the drafting step? The spine still runs — execution doesn't wait for data — and the unwired input falls back to its default, most likely drafting a lovely reply about an empty string. Forget an execution wire into send? Perfect draft, sent never. Neither mistake stops the other story; that independence is the number-one thing this build should teach your hands.

Then run it, with a test message worthy of the scenario ("my kettle says NO"). Watch the spine light up.

> **Watch out:** if your drafting step needs a provider credential, set it up the lesson-4 way — Secret + Runtime Configured, value in Runtime Variables — before your first run, not after your first paste-into-a-node regret.

## Recap

- Build the white spine first, then satisfy each data input working backward from the consumer.
- Drag from a pin and drop on empty canvas: the context-sensitive catalog offers only compatible nodes and wires them for you.
- Missing data doesn't stop execution, and missing execution isn't fixed by data — the stories fail independently.

You have a running flow that only you can trigger. Next: putting a real surface in front of it.
