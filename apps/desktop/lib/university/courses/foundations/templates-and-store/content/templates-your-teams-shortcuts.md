Wednesday. The triage flow finally survives every weird edge case the last client threw at it, and a new client starts tomorrow. Priya suggests copying the whole app. But the app drags storage, events, and team settings along with it — you only want the *graph*. This is exactly what Flow Templates are for: reusable, versioned snapshots of individual flows.

> **Predict first:** You're about to template the triage flow. Do you snapshot it first and template the snapshot — or template the editable draft directly? What could go wrong with each?

## 1 · Freeze what works

Before sharing a blueprint, give it a version. Open the flow in Studio and open **Manage Board** from the toolbar. The dialog below shows the board "Customer Support Automation": its name and description, stage, log level, execution mode, and a **Version** selector currently reading **Latest (1.0.0)** — with the **Create Version** menu open, offering Major, Minor, and Patch:

@BoardVersions

Choosing one saves the current draft as an immutable snapshot and moves the editable draft to the next version number. The convention is the usual `major.minor.patch`: Major when callers may need migration, Minor for compatible additions, Patch for small corrections. Flow-Like doesn't infer which change you made — pick the bump that matches how the flow is consumed.

## 2 · Template it

Now open the app's **Templates** workspace and select **Create Template**. You'll:

1. Enter a name and description.
2. Choose the source flow.
3. Choose a numbered flow version — or **Latest**.
4. Create the template.

That third step hides this lesson's trap. A numbered version captures that immutable snapshot — no surprises. **Latest captures the current draft at the moment the template is created — and then stops.** The template does not keep following later edits. Next month, when the draft has ten improvements the template never saw, "but I picked Latest!" will feel very unfair. Latest means "freeze what's on the canvas right now", not "stay subscribed to my drafts".

## 3 · Grow the template, don't replace it

Open a template to review its description, source information, and available versions. When the source flow improves, you don't overwrite anything — you **import a newer flow snapshot as another template version**. Older versions stay available, so consumers deliberately choose the blueprint they want. Test a source flow before adding it as a template version: the metadata helps people evaluate, but the executable content is the captured graph.

Know what travels. A template carries the flow graph snapshot plus its own name, description, and versions — and nothing else. It is not a backup of the source app: runtime variables, credentials, events, pages, storage, and Data Studio data all stay behind. A colleague spinning up a new client app from your triage template still configures their own credentials and events.

| Share this | Use |
| --- | --- |
| **Flow Template** | One reusable graph and its versioned snapshots |
| **App** | A complete project boundary — flows, events, pages, storage, data, access |

## Recap

- Version first (Manage Board → Create Version), then template the proven snapshot.
- **Latest** freezes the draft at creation time — templates never follow later edits.
- Templates carry the graph, not the app: variables, events, and storage stay behind.
