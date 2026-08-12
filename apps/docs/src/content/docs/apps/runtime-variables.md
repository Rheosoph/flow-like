---
title: Runtime Variables
description: Keep device-specific configuration out of your Flow definitions
sidebar:
  order: 25
---

Runtime variables separate a value from the Flow that uses it. They are useful
for API keys, passwords, local paths, environment-specific endpoints, and any
other value that should be supplied by the person or device running the Flow.

The Flow definition keeps the variable's name, type, and settings. Its saved
runtime value lives in Flow-Like's local application storage instead of being
synced with the app.

## Configure a value

1. Open a Flow in Studio and open its variables panel.
2. Select the variable.
3. Enable **Runtime Configured**.
4. Enable **Secret** as well when the value should be masked and excluded from
   remote execution.
5. Open the app and select **Runtime Variables**.
6. Expand the Flow, enter the value, and select **Save**.

Variables marked **Secret** also appear on the Runtime Variables screen, even
when **Runtime Configured** is off.

![The Runtime Variables workspace in Flow-Like Desktop, showing a configured local endpoint and a masked secret](../../../assets/RuntimeVariables.webp)

:::note[First run]
When an interactive run needs a value that has not been saved, Flow-Like opens
the **Configure Runtime Variables** dialog before execution. Saving the dialog
continues the pending run.
:::

## Runtime configured and secret are different

The two settings solve related but distinct problems:

| Setting | Local value | Display | Remote execution |
| --- | --- | --- | --- |
| **Runtime Configured** | Saved per app and device | Uses the editor for its data type | Included with that run when it executes remotely |
| **Secret** | Saved per app and device | Masked, with an explicit reveal control | Excluded from the remote execution payload |
| **Both** | Saved per app and device | Masked | Excluded from the remote execution payload |

Use **Runtime Configured** for values that vary by user, machine, or
environment. Add **Secret** for credentials and other sensitive values.

:::caution[Remote execution]
A remotely executed Flow does not receive locally stored secret values. Do not
design an unattended or web-triggered remote Flow to depend on one. Use a
server-side credential mechanism appropriate to that deployment instead.
:::

## Where values go

Flow-Like Desktop stores configured values in its local IndexedDB database,
keyed by app and variable. Values use the same JSON-encoded byte representation
as Flow variables.

This separation prevents the configured value from being written back into the
Flow definition or synced as part of the app. It is not a claim that the value
is independently encrypted at rest: protect the operating-system account and
local application data like any other credential store.

During execution:

- A local run can receive runtime-configured values and secrets.
- A remote run can receive non-secret runtime-configured values for that run.
- Secret values are filtered out before a remote execution request is sent.
- A saved runtime value takes precedence over the value carried by the Flow
  definition.

The **Hybrid** execution mode does not split one graph between local and remote
machines. On Desktop it normally runs locally; on a web or remote-only path it
runs remotely. See [Offline vs. Online](/apps/offline-online/) for the complete
execution-mode matrix.

## Events

Events use the runtime-variable requirements of their referenced Flow. An event
can execute locally or remotely depending on the client, permissions, and the
Flow's execution mode:

- Interactive local execution can prompt for missing values.
- Remote execution includes only non-secret runtime values.
- Unattended remote events cannot use a secret that exists only on someone's
  device.

## Good practices

- Keep credentials out of regular variable defaults.
- Use clear names such as `OPENAI_API_KEY` or `DATABASE_URL`.
- Add a description that tells the runner what to provide, without including
  the value itself.
- Give each environment its own local configuration.
- Delete a saved value from **Runtime Variables** when a device should no longer
  use it.
- Do not copy or share Flow-Like's local application-data directory as a way to
  distribute credentials.

## Related guides

- [Variables in Studio](/studio/variables/)
- [Flows](/apps/boards/)
- [Events](/apps/events/)
- [Offline vs. Online](/apps/offline-online/)
- [Local-only execution](/studio/local-execution/)
