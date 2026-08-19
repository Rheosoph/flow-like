The pull works — because Orbit's API token is sitting in the Token pin's default value, in plain sight. Now play the tape forward: the app syncs, three teammates get the flow, the token rides along in every copy. Friday you rotate it; Monday three flows break and the old credential lives on in places you can't reach. This lesson gets the token out of the flow before any of that happens.

> **Predict first:** You mark `CRM_API_TOKEN` as Secret and Runtime Configured, save your value, and a teammate opens the same app on their laptop. What do they see on the Runtime Variables screen for that variable?

## 1 · Split the value from the flow

Open the flow in Studio and open its variables panel. Create `CRM_API_TOKEN`, then enable two settings on it: **Runtime Configured** and **Secret**. Add `SUPPORT_API_URL` too, Runtime Configured only — it's not sensitive, it just differs between environments.

The split works like this: the flow definition keeps the variable's name, type, and settings. The *value* lives in Flow-Like's local application storage on the device that configured it, keyed by app and variable. It is never written back into the flow and never synced with the app. Your teammate gets the variable — and an empty slot where your value would be. That's the answer to the prediction, and it's the point: each person and machine supplies its own.

## 2 · Configure it once per device

Open the app and select **Runtime Variables**.

@RuntimeVariables

The screen shows the *Customer Support Automation* flow fully configured — 2 of 2. `CRM_API_TOKEN` carries a **Secret** badge and displays only masked dots; `SUPPORT_API_URL` carries a **Runtime** badge and shows its environment-specific URL in the clear. The security notice at the bottom states the contract in one sentence: values are stored locally on your device and never uploaded to the server, and for remote execution only non-secret runtime variables are sent.

You don't even have to configure ahead of time. When an interactive run needs a value nobody has saved, Flow-Like opens the **Configure Runtime Variables** dialog before execution — save it and the pending run continues.

## 3 · Use it in the flow

Variables read through generated **Get** nodes — search the catalog for `Get CRM_API_TOKEN` or drag the variable from the panel onto the canvas. Feed it into **Set Bearer Auth**'s Token pin from last lesson, and the request carries `Authorization: Bearer …` without the flow definition ever containing the credential. `SUPPORT_API_URL` feeds Make Request's URL the same way, so your staging laptop and the production machine call different endpoints from the same flow.

The execution rules are worth memorizing, because lesson 4 will lean on them hard:

- A **local run** receives runtime-configured values *and* secrets.
- A **remote run** receives only non-secret runtime-configured values; secrets are filtered out before the request ever leaves your device.
- A saved runtime value takes precedence over anything the flow definition carries.

> **Watch out:** A remotely executed flow never sees a secret that exists only on someone's laptop. If an unattended or web-triggered flow needs a credential, provision it through a server-side mechanism for that deployment — don't design the flow to depend on your device being awake.

Name your variables like you'll be reading them at 3 a.m. — `CRM_API_TOKEN`, `SUPPORT_API_URL` — and give each a description that tells the runner what to provide without including the value. Keep credentials out of regular defaults entirely; that's the mess we just cleaned up.

## Recap

- Runtime Configured splits value from definition: the flow syncs, the value stays in local application storage, per app and device.
- Secret adds masking and exclusion from remote execution payloads — combine both for every credential.
- Read values with generated Get nodes; a saved runtime value beats whatever the flow carries.
