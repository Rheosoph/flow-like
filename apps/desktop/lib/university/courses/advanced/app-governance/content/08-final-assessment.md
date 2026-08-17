This is where the whole course happens at once. The Customer Support Copilot is leaving the safety of Prototype: the team wants it operated remotely, opened to a real support organization, and — before the quarter ends — listed publicly. You're signing off on the design. Six proposals landed on your desk, written by six different people; the assessment asks you to judge each one.

## The situation

- **The App:** ticket triage and drafted replies. One Hybrid Flow ("Customer Support Automation"), a chat Page for the team, plus a nightly triage schedule and a CRM sync API Event — both meant to run with no desktop open anywhere.
- **The data:** a ticket database, shared reply templates in storage, per-user drafts. The CRM is an external API with a token.
- **The people:** analysts who inspect outcomes and tables; operators who run and diagnose production; builders who change Flows; a two-person release group that alone touches production entry points; a steady stream of new support agents joining by invite link.
- **The state:** visibility is Prototype. The board's draft moves daily. A numbered version 1.4.2 exists and production is pinned to it. The CRM token currently lives as a Secret on one builder's desktop. Publication evidence is half-assembled.

## Your review packet

One proposal per challenge:

1. A connectivity choice for the App, argued from the team's operating needs.
2. A role card for the operators, mapped to ladder levels.
3. A data grant for the analysts, argued as "the narrowest that works."
4. A credential plan for the unattended remote Events.
5. A release plan for a compatible fix that's ready while production stays pinned.
6. A publication plan for the move to a public target.

## How to review

For each proposal, name the boundary it touches — connectivity, permission, data, secret, version, or exposure — and ask what the least-privilege or most recoverable option looks like. Distrust any design that was only tested as an Owner, and anything that "works" because a value happens to be reachable. Your lesson artifacts — the inventory, the role card, the release steps, the governance record — are fair reference material. Each answer's explanation tells you not just what's right, but what the tempting wrong option would have cost.

@BoardVersions

The release surface, one last time: the Manage Board dialog with the Version selector on **Latest (1.0.0)** and the Create Version menu offering Major, Minor, and Patch. Somewhere between that dialog and the store listing sit all six of your decisions. You've built everything you need — go sign off.
