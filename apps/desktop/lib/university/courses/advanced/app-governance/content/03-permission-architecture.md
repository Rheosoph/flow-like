Rosa joins the Copilot team to run production: fire the triage Flow when a batch needs a rerun, trigger the nightly Event when the schedule misses, and read logs when something smells wrong. Marek's first suggestion: "Just give her Admin, it's one checkbox." That checkbox would let Rosa rewrite every Flow, every role, and the App's configuration — to do three read-and-run tasks.

> **Predict first:** how many distinct permissions does Rosa's job actually need?

This lesson gives you the vocabulary to answer precisely: elevation, ladders, and flags.

## 1 · Elevation is not a role

The permission model has two **elevation** flags, Owner and Admin. Permission checks treat either one as satisfying every ordinary permission — which is exactly why elevation deserves a tiny membership and periodic review, and why "just give her Admin" is never a role design. It also means you can't infer capability from a role's name: "analyst" and "operator" are labels. Always evaluate the *effective flags* after elevation and ladder levels are applied.

## 2 · Rosa's three ladders

Most permissions come in **ladders**: ordered levels where each level contains the ones below it, so impossible combinations (edit a Flow you can't view) can't happen. Rosa's job touches exactly three:

- **Workflows:** None → View (`ReadBoards`) → Run (adds `ExecuteBoards`) → Edit (adds `WriteBoards`)
- **Events:** None → Browse (`ListEvents`, `ReadEvents`) → Trigger (adds `ExecuteEvents`) → Manage (adds `WriteEvents`)
- **Observability:** None → Logs (`ReadLogs`) → Full (adds `ReadAnalytics`)

Rosa gets **Workflows: Run**, **Events: Trigger**, **Observability: Full**. That's the answer to the prediction: seven flags across three ladders — and not one of them spells "edit."

@RightsAndRoles

That's the New Role dialog, mid-creation for a role named "Support Specialist." Count the permission groups: eight, each with a checked-of-total counter — System 0/2, Team & Access 0/3, Files & Data 0/5, Workflows 0/3, Events 0/4, Observability 0/2, Configuration 0/2, Content 0/7. The **System** group at the top is the Owner/Admin elevation pair from section 1; the other seven rows are the seven ladders. (The Attributes section below holds custom key-value tags for filtering and policy rules — it grants nothing.)

## 3 · All seven ladders, for reference

| Ladder | Levels |
| --- | --- |
| Team & Access | None → View (`ReadTeam`, `ReadRoles`) → Use API (adds `InvokeApi`) |
| Files & Data | None → Read (`ReadFiles`, `ReadDatabase`) → Write (adds `WriteFiles`, `WriteDatabase`, `WriteMeta`) |
| Workflows | None → View → Run → Edit |
| Events | None → Browse → Trigger → Manage |
| Observability | None → Logs → Full |
| Configuration | None → View (`ReadConfig`) → Edit (adds `WriteConfig`) |
| Content | None → View → Edit → Publish (adds `WriteRoutes`) |

You don't memorize this table; you return to it every time you design a role. The dependency rule is the part to internalize: whoever can trigger an Event can browse it, whoever can edit a Flow can view and run it — within one ladder, never across ladders.

## 4 · The database nuance

The Files & Data ladder bundles files and tables into one safe shape. But raw flags exist for narrower jobs: `ReadDatabase` and `WriteDatabase` grant database-only access without any file access. Note the asymmetry — `ReadFiles` also *implies* database read for backward compatibility, and `WriteFiles` implies database write, so the file flags are always the broader grant.

For the Copilot's analysts, who need the ticket tables but have no business browsing app files, `ReadDatabase` alone is the narrowest fit. When a role needs a raw custom set like this, record why it deviates from the ladder and retest it after every permission-model change.

**Watch out:** when a role is missing one capability, the fix is one ladder level — not Admin. Elevation is how a role for three tasks becomes a blast radius for all of them.

## Recap

- Owner/Admin elevation satisfies every check — keep it rare, named, and reviewed.
- Seven ladders turn raw flags into ordered levels; each level contains the ones below it, within its own ladder.
- Database-only flags are narrower than `ReadFiles`/`WriteFiles` — reach for them when the job is tables, not files.
