02:00. The Copilot's nightly triage Event fires on the backend, calls the CRM — and every request comes back unauthorized. The token is fine. It's saved on Rosa's laptop, marked Secret, exactly where she left it. That's the problem: secrets stay home.

> **Predict first:** the App is online and syncing. Why didn't the token sync too?

## 1 · Two variables, two behaviors

@RuntimeVariables

This is the Copilot's Runtime Variables page — "2 of 2 configured" for the Customer Support Automation board. `CRM_API_TOKEN` is tagged **Secret**: its value renders as masked dots, its description reads "Credential used to load customer account context." `SUPPORT_API_URL` is tagged **Runtime**: an environment-specific endpoint, readable in the clear. And the Security Notice at the bottom answers the prediction in the product's own words: runtime variables are stored locally on your device and never uploaded to the server — for remote execution, only non-secret runtime variables will be sent.

The model behind it: a Flow variable keeps its name, type, and settings inside the synchronized Flow definition. Its *value* lives in local application storage, per App and per device. **Runtime** means supplied per device and eligible to travel with a remote run. **Secret** means masked locally and excluded from every remote payload. Both together: per-device, masked, never sent.

One honest caveat: this separation prevents accidental synchronization; it is not a claim that local application data is encrypted at rest. Protect the operating-system account too.

## 2 · What travels where

A local interactive run can use everything saved on that device — runtime values and secrets alike. A remote run receives only the non-secret runtime values sent along for that run. It cannot reach a Secret that exists on Rosa's laptop, no matter how online the App is. That's the 02:00 incident, fully explained.

So an unattended remote Event needs a credential that lives where the run lives: provision it through the server-side mechanism appropriate to your deployment. The Copilot's fix — the CRM credential goes server-side for the nightly schedule; Rosa's desktop Secret stays for local debugging.

And a Hybrid reminder from lesson 1: desktop callers run locally, web and server callers run remotely — so check that *each* host has the nodes, file paths, network access, and credentials its runs will need.

## 3 · Keep values out of definitions

The Flow definition carries names and descriptions — `CRM_API_TOKEN`, "credential used to load customer account context" — never values. The classic leak is helpfulness: pasting the real token into the variable's *default* "so teammates aren't blocked." Defaults are part of the Flow definition. They synchronize to everyone with board read access and travel with remote runs. The masking you saw on the Runtime Variables page protects the locally saved value — not a default.

Round out the strategy with three habits: one credential per environment (never one token shared across dev and production), the narrowest scope the integration allows and short-lived credentials where supported, and a recorded owner, rotation period, and dependency list per credential — so the owner can rotate a value without touching the Flow version, and rotation doesn't cause a surprise outage.

**Watch out:** if a remote run suddenly "works" after someone made a credential reachable, ask *how* before celebrating. The two-minute audit: open the board and read every variable default, looking for anything shaped like a key.

## Recap

- Runtime values live per device; Secret values are masked and excluded from remote payloads.
- Unattended remote execution needs server-side credentials — a desktop Secret can't carry it.
- Names and descriptions belong in the Flow; values never do — defaults included.
