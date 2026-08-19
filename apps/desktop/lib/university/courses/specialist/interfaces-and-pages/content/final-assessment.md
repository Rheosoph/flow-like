Ship day. Nothing new to learn — this assessment hands you the support lead's launch checklist and the app as it stands, and asks you to make the calls. Every question combines at least two things you built across this course.

## The state of the app

@AppEvents

Current wiring, straight from the Events workspace: two active UI Events — the **Triage selected request** Quick Action at `/triage` and the **Support assistant** Chat UI at `/chat` — both entering the **Customer Support Automation** flow. The Pages tab lists three pages: **Support Operations Dashboard**, **Customer Intake**, and **Escalation Console**. The **Support Health Card** widget exists with one exposed property, *Card title*, and its definition contains an **Escalate** button.

## The launch checklist

From the support lead, verbatim:

1. "Opening the app should land me on the operations dashboard. Not a menu. The dashboard."
2. "The health card goes on the dashboard **and** the escalation console — but the dashboard one says *Queue health* and the console one says *Escalation health*."
3. "The card's Escalate button must actually escalate — pressed on the console, it runs the escalation logic in Customer Support Automation."
4. "Design wants the chat in Aurora Glass but with our purple. Legal wants the AI notice kept, word for word."
5. "You'll keep improving the flow all week. Nothing the team uses may change before Friday's release."

## The beta report

Two findings from Thursday's testers:

- "Customer Intake shows up on the Pages tab, but nobody can open it. There's no address that reaches it."
- One teammate is convinced the dashboard and the chat are "different systems" and wants a second engineer assigned to "the chat codebase."

## Ground rules

Answer from Flow-Like's actual model — routes, pages, widgets, chat configuration, events, and versions — not from what a generic web framework would do. Where two approaches both "work," pick the one that survives next month's changes: no forked copies, no hardcoded targets, no draft surprises in production.

Take the checklist item by item. The challenges below are the launch review.
