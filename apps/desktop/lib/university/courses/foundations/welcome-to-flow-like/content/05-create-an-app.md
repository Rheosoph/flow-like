You've read someone else's board for three lessons. Time to own one. In the next two minutes you'll create the Customer Support Copilot as a real project on your machine — no account, no setup, no cost.

## 1 · Open the dialog

From Home, click the **Create a new app** chip (or use the create action in **My Apps**).

@CreateAppDialog

A dialog appears — titled, yes, **Create Flow**. Read the rest of it: the subtitle says "Create a new project with all embedding models from your current profile", the field is **Project Name** (filled with "Customer Support Copilot" in the screenshot), and the confirm button says **Create Project**. The title is the one word of this dialog to take least literally: it creates the whole App around your first Flow. Naming is hard.

Below the name sits the real decision, **Connectivity**: *Online — Sync with cloud (Login required)* or *Offline — Local only*. In the screenshot Offline is selected and Online is greyed out — nobody is signed in, and offline needs no account.

## 2 · Name for the outcome

Name the project after what it does for people, not how it's built today. **Customer Support Copilot** stays accurate if you swap the model or the data source. **Three Nodes Test** does not. Future-you, scanning a Library of twelve apps, will be grateful.

## 3 · Offline or online — decide with a reason

**Offline** stores the App on this device, works signed out, and is the right default for experiments and personal automation. **Online** uses a Flow-Like backend and buys you browser access, multi-device use, collaboration, roles, and publication — at the price of an authenticated account.

Two facts save later pain. First, there is no in-place switch: an offline App isn't quietly converted. When the day comes, Flow-Like creates an **online copy** — your local source stays intact, known secret fields are stripped from the copy, and you review its credentials, Events, and access before the team touches it. Second, **online is not public**: publication is a separate, deliberate step.

## 4 · What your new App owns

@AppAnatomy

The anatomy diagram from lesson 1 is now *your* project's floor plan: five capability cards hang off the App — **Flows** (typed visual workflow logic), **Experiences** (Events, Pages, routes, and Chat), **Data** (Storage and Data Studio), **Reuse** (Widgets and Flow templates), and **Delivery** (team access, releases, and sharing). One App can hold many Flows; split them by responsibility — "answers requests" versus "files follow-ups" — not because a canvas got visually crowded.

## Do it now

1. Open the dialog, name the project **Customer Support Copilot**, keep **Offline**.
2. Click **Create Project**, then open the project's first Flow. An empty canvas, all yours.
3. Right-click the canvas and search "mail" — note that *Send Email* is waiting for the day you build your own *Send Reply*. Press Esc.
4. In the App's sidebar, find where Events and Storage live. Look, don't touch.

That's a real project boundary, and you made it in under a minute. Every later course in this track builds inside a boundary exactly like it.

## Recap

- The Create Flow dialog creates an App: name it for the outcome, then choose connectivity deliberately.
- Offline = this device, no sign-in. Online = browser, team, publication. Conversion happens by copy, never in place.
- The App owns Flows, Experiences, Data, Reuse, and Delivery — graphs live in Flows, everything else surrounds them.
