17:09. You've traced the null body to its producer and you know the fix. Your cursor is hovering over the node. Twenty-one minutes left — plenty. Stop.

> **Predict first:** if this fix makes things *worse*, what exactly do you fall back to? Does saving the board create a restore point?

No. Saving updates the draft — the same draft you're about to edit. If your fix is wrong, "undo" is you, tired, at 17:25, trying to remember what the board looked like at 17:08. Flow-Like has a better answer: versions.

## Snapshot before surgery

@BoardVersions

The **Manage Board** dialog, opened from the Studio toolbar. Below the board's name ("Customer Support Automation"), description, stage, log level, and execution mode sits the **Version** selector, showing **Latest (1.0.0)**, and next to Save, a **Create Version** button — pressed here, revealing a **Version Type** menu: **Major, Minor, Patch**.

Create Version does one precise thing: it saves the current draft as an **immutable, numbered snapshot** and moves your editable draft to the next version number. Existing snapshots are never overwritten. The Version selector then distinguishes:

- **Latest** — the editable draft, where all your fixing happens.
- **A numbered version** — a read-only snapshot. Open one to inspect its graph or its execution history; return to Latest before editing.

So the 17:09 routine is: open Manage Board, Create Version, *then* fix. The broken-but-known state is now frozen where no fix, however misguided, can touch it.

## Major, minor, or patch?

The Version Type menu is a promise to the flow's consumers, and Flow-Like deliberately doesn't guess it for you:

- **Major** — existing callers or behavior may need migration.
- **Minor** — compatible behavior or capability is added.
- **Patch** — a compatible correction or small adjustment.

Friday's fix — a null body corrected, nothing about how anyone calls the flow changed — is the textbook **Patch**.

## Pin, fix, repoint

Versions earn their keep at the app's entry points. An Event can target **Latest**, following every draft edit as it happens, or a **pinned numbered version**, running that exact snapshot until someone deliberately changes the pin. Production-facing Events should be pinned — you do not want Tuesday's half-finished draft experiment answering real customers.

That makes the full Friday sequence:

1. **Create Version** — freeze the current state (your rollback target).
2. Fix one thing on **Latest**.
3. Verify — fresh run, same input.
4. Create a **Patch** version of the verified fix.
5. Repoint the Event to the new version and test it.

And if the new version misbehaves in the wild? **Rollback is repointing**: re-select the previous tested version on the Event, keep debugging on Latest, and ship the next patch when it's actually ready. You never edit a numbered version in place — immutability is precisely what makes a version worth pinning.

One nearby trap: an App also has a free-form "Version" field in its details. That's release *metadata* — a label for people browsing the app. Editing it snapshots nothing.

**Watch out:** a pinned Event doesn't receive your fix when you save the draft, or even when you create the version. Until you repoint the Event, production runs the old snapshot — feature, not bug, but only if you remember step 5.

Recap:

- Create Version freezes the draft as an immutable snapshot; the draft moves to the next number.
- Choose Major/Minor/Patch by what consumers must do about the change — Flow-Like won't infer it.
- Pin production Events; roll back by repointing, never by editing a numbered version.
