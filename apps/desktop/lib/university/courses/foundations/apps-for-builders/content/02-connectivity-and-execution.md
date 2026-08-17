Friday demo: you run the Customer Support Copilot from your laptop, drafts appear, everyone applauds. Friday night: you close the lid. Monday morning: the nightly case summary never ran, and nobody could open the app in a browser all weekend. Nothing "broke" — you just answered three different questions with one assumption.

> **Predict first:** which three separate decisions did that demo quietly conflate?

Here they are: **connectivity** (where the App is stored and who can reach it), **execution mode** (where a Flow run happens), and **Event location** (which runner answers an invocation). Flow-Like keeps them separate on purpose. Treat them separately and the Monday surprise never happens.

## 1 · Connectivity — where the App lives

This is the radio button you saw in @CreateAppDialog. An **offline App** stores its data on the current device, needs no sign-in, and never appears in the web app — right for personal experiments, device-bound automation, and work that must not leave one machine. No roles, no invitations, no browser access, no publication.

An **online App** lives on the configured Flow-Like backend: authenticated web access, multiple devices, collaboration, server-side Events, publication workflows. Online does not mean public — visibility and roles stay deliberate settings you control.

Write your non-negotiable constraint first, then pick. "The summary must run while every laptop is closed" forces online. "This automation drives my desktop and its data stays here" points offline.

## 2 · Execution mode — where a run happens

A Flow set to **Local** runs from Flow-Like Desktop and can't be invoked remotely. **Remote** runs through the backend, even when you press play in Desktop. **Hybrid** picks per invocation: started from your Desktop, the run happens locally; started from the web or a remote caller, it happens remotely.

The word *picks* is doing careful work there. Hybrid does not split one run across machines. Every invocation executes on exactly one host, start to finish — if your run touches ten nodes, all ten execute in the same place.

## 3 · Capabilities pin the host

Some nodes need the device itself: browser control, desktop input, screen inspection, local file paths. Flow-Like analyzes your whole graph — nested layers included — and if even one node is local-only, the entire run must execute locally. There is no "offload the small local part."

Two symmetric traps live here. A local run is not an offline sandbox: it can happily call cloud APIs, because "local" names the host, nothing more. And an offline App can never run server-side, even if every node in it is remote-compatible — offline means there is no backend copy to invoke.

Events add the last piece: where the type permits, an Event can be **Local** (needs an available Desktop runner) or **Remote** (answers while every laptop is closed — provided its Flow and every node inside support remote execution).

## 4 · Move online without carrying assumptions

The Copilot started offline. To take it to the team, you don't flip a switch — you **create an online copy**. The local original stays put; a new, secret-stripped copy uploads: known token fields and secret variable defaults are removed, and device Runtime Variables never travel with the bundle.

Then review the copy like the deployment it is: every Flow, credential, Event location, storage path, role, and version target. Local paths and device-only nodes may need redesign, and server credentials come from the deployment's supported mechanism — not from secrets that lived on your laptop. (Whether callers follow your editable draft or a frozen snapshot is the versioning question — lesson 5 owns it.)

> **Watch out:** the most expensive misreading of this lesson is "online means remote." Connectivity is about storage and reach; execution mode is about the run's host. An online App can run Flows entirely on your Desktop all day.

## Recap

- Three separate dials: connectivity (storage and reach), execution mode (run host), Event location (which runner answers).
- Every invocation runs on exactly one host; a single local-only node anywhere in the graph pins the whole run to Desktop.
- Going online is a reviewed copy, not a mutation — secrets are stripped and must be re-provisioned server-side.
