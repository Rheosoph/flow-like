Monday, 09:12. Your team duplicated the Customer Support Automation board to build **Priority Support** — a VIP fast lane for customers whose kettles fail loudly on social media. Somewhere between the duplication and this morning's stand-up, the drafting layer got deleted. The copy now has a hole where its heart used to be, and you're the one who understands boards well enough to fill it.

The healthy original, for reference:

@FlowWithLayers

Event on the left, two collapsed layers and the mail node along the white spine, the pure pair idling at the bottom, three phase comments overhead. Keep that picture in mind — the copy looks like this, minus Prepare Support Reply.

## What you find on Priority Support

- A white execution wire leaves **Incoming Support Request** and ends in open canvas. **Human Review** still waits for execution that never arrives.
- The event's pink **Request** output has a dashed wire drawn toward the gap, connected to nothing.
- **Send Reply** is intact and still expects a reply value on **Body**, delivered through the review step.
- The pure pair — **Customer Message** feeding **Format Generic Value** — survived the duplication. Its final output is, as ever, connected to nothing.
- VIP replies must go out through the **mail provider**, whose API token is not yet configured anywhere on this copy. VIP runs happen locally on the on-call laptop.
- The original board's customer-facing Event — the one real customers hit today — must keep behaving exactly as it does now. Meanwhile a teammate wants to rework Prepare Support Reply's boundary pins *on the original* before the layer gets copied over.
- Both boards' drafts are being edited daily. The copy's Event currently points at Latest.

## Your job

Reason from the contracts, not from node names: what must be true of whatever fills the gap — on the execution side and on the data side? What does the idle pure pair do during a run, and what would change that? Where does the token live? Which checks does the boundary rework demand? Which Event points where while all of this churns? And when the first VIP test reply comes back empty — what do you do *before* touching the graph?

Every question below is one of those decisions. The evidence above is sufficient; nothing needs guessing. Take the run apart in your head the way you've done for eight lessons — then commit.
