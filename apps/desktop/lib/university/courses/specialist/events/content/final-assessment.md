Sprint's over. You're the release owner for the **Customer Support Copilot**, and three surfaces ship this week:

1. **Intake form** — visitors open `/support/new`, submit subject, description, and priority, and get validation feedback. The flow team edits drafts daily, but production behavior must stay predictable.
2. **Operator refresh** — an operator inside the app manually refreshes a cached dashboard.
3. **Hourly reconciliation** — runs on company infrastructure every hour and writes an auditable completion record. Nobody watches it.

## Current state, straight from the team channel

- The intake flow has a **Generic Event** node with `subject`, `description`, and `priority` output pins; the `/support/new` event pins **version 3**.
- The refresh flow exists, but Create Event isn't offering the type you expected for it.
- The reconciliation was built on a developer's laptop. Its API token is a local **Secret** runtime variable, and the Cron event is configured **Local** — on that laptop.
- A support landing Page is designed and renders in the builder, but nobody can open it.
- Yesterday's dry run logged two problems: a double-clicked form submission created **two tickets**, and the reconciliation **logged success while the completion record never appeared**.

The routing model, for reference while you decide:

@RouteEventArchitecture

Each question below hands you one release decision from this state. Everything you need is in the five lessons — and in the symptoms above. Ship it clean.
