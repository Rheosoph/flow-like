Friday, 16:58. A security reviewer signs off on the Customer Support Copilot's triage Flow: logic clean, no credential leaks, nice work. Monday, 09:12: support data turns up where it shouldn't. The Flow was fine. The Event that exposed it to the web was never in the review — and neither was the weekend invite link that quietly made forty strangers "team members."

> **Predict first:** a Flow passes line-by-line review. Name two ways its App can still leak data.

Here's the resolution to both the vignette and your prediction: the Flow is not the unit that ships. The App is. By the end of this lesson you'll draw the Copilot's full boundary, choose its connectivity deliberately, and explain why "online" never means "runs remotely."

## 1 · Everything inside the boundary

@AppAnatomy

This is the shape you're governing. One App card — Customer Support Copilot, "a project boundary for everything needed to build and ship an experience" — fans out into five groups: **Flows** (the typed visual workflow logic), **Experiences** (Events, Pages, routes, and Chat — everything that exposes logic to a caller), **Data** (storage and Data Studio), **Reuse** (Widgets and Flow templates), and **Delivery** (team access, releases, and sharing).

Review one card alone and you miss the rest: the Event that exposes a Flow, the role that can invoke it, the credential it expects, the mutable draft an entry point may follow. Govern the five cards as one operating unit — one blast radius.

## 2 · Offline or online is a storage decision

An **offline App** lives in Flow-Like Desktop, works without signing in, and moves between machines only by explicit export and import. Pick it for personal experiments, attached hardware, or data that must never leave one machine. It has no web access, no team roles, no publication workflow.

An **online App** syncs through the configured Flow-Like backend and unlocks authenticated web access, team roles, server-side Events, and publication. The Copilot needs all four, so it goes online.

One trap hides in the migration: creating an online copy from an offline App produces a new, **secret-stripped** copy. Your local runtime values stay on your device; the copy starts unconfigured. Review its Flows, storage, credentials, roles, and Events before treating it as production-ready — it's a new App, not a moved one.

## 3 · Where a Flow runs is a separate decision

Online is where the App lives. Execution is where a run happens, chosen per Flow:

- **Local** runs on Desktop — invisible to a web-only caller.
- **Remote** runs on the configured execution backend.
- **Hybrid** runs locally when Desktop calls it and remotely when web or server-side paths do. It never splits one graph across machines.

A local-only node, device input, file path, or attached machine forces local execution — discover that before a web-triggered run fails, not after. Events add one more pair of choices: a local or remote sink, and following Latest or a pinned numbered version. Hold onto those two words; they carry lesson 6.

## 4 · The Copilot's inventory, filled in

Here's the governance inventory for the Copilot — the artifact every later lesson builds on:

| Row | Customer Support Copilot |
| --- | --- |
| Purpose & owners | Ticket triage and drafted replies; product owner Sara, technical owner Marek, data owner Priya |
| Connectivity | Online — team roles, web access, nightly server-side Event, planned publication |
| Flows | "Customer Support Automation" (triage + reply drafting), Hybrid |
| Events & routes | Chat page (team), nightly triage schedule (remote, pin planned), CRM sync API Event (remote, pinned) |
| Data | Ticket database, shared reply templates in storage, per-user drafts |
| Runtime config | `SUPPORT_API_URL` (runtime), `CRM_API_TOKEN` (secret) — lesson 5's whole story |
| Reuse & surfaces | Reply widget, triage Flow template |
| Access | Roles and a default role — lessons 3 and 4 |
| Operations | Monitoring, incident, change, rollback, and publication owners — lesson 7 |

Now reproduce it: open one of your own apps and fill the same nine rows. Any row you can't fill is a governance gap you just found for free.

**Watch out:** the most common boundary mistake isn't a missing row — it's assuming "online App" means every run is remote. Connectivity and execution are separate decisions, always.

## Recap

- The App — Flows, Experiences, Data, Reuse, Delivery — is the unit you govern, not the Flow.
- Offline vs online is a storage-and-collaboration choice; Local/Remote/Hybrid is a per-Flow execution choice.
- An online copy of an offline App is a new, secret-stripped App that needs its own review.
