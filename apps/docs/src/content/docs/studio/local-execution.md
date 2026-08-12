---
title: Local-Only Execution
description: Run flows that need the browser, desktop, or other device-local capabilities
sidebar:
  order: 45
---

Some Flow-Like nodes need capabilities that exist only on the machine running
Flow-Like Desktop. These nodes are marked as **local-only** in the catalog and
show a monitor badge on the Flow canvas.

Before a run, Flow-Like inspects every node in the Flow, including nodes inside
layers. If any node is local-only, the pre-run result marks the complete Flow as
requiring local execution. A single run is not divided between local and remote
workers.

## Capabilities that run locally

The current [Automation catalog](/nodes/automation/) contains the main
local-only node families:

| Family | Examples | Why it needs the device |
| --- | --- | --- |
| **Browser** | Open a browser, navigate, fill fields, extract page data, download files, and take screenshots | Controls a browser session on the runner |
| **Computer** | Inspect accessibility elements, focus windows, move the mouse, type, use the clipboard, and capture the display | Uses the active operating-system session |
| **Vision** | Find or click image templates, inspect pixels, and wait for a visual state | Reads the runner's display |
| **RPA** | Locate targets, act, assert, retry, checkpoint, and collect diagnostics | Coordinates local UI interactions and recovery |

These are concrete catalog capabilities—not a general promise that every
camera, microphone, USB device, GPU, or locally installed program is available.
Check the documentation for the specific node you intend to use.

For a complete automation guide, see
[Desktop & Browser Automation](/topics/desktop-automation/overview/).

## Choose a compatible execution mode

| Flow or Event setting | Compatible with local-only nodes? |
| --- | --- |
| **Local Flow** | Yes, from Flow-Like Desktop |
| **Hybrid Flow** | Yes when invoked locally; no for a remote invocation |
| **Remote Flow** | No |
| **Local Event** | Yes, while the Desktop event runner is available |
| **Remote Event** | No |

**Hybrid** means that the same Flow can run locally or remotely depending on
the caller. It does not split one graph across both environments. If the graph
contains a local-only node, invoke that Flow locally.

An online App can still run a compatible Flow locally from Desktop. Conversely,
an offline App has no server-side invocation path. See
[Offline vs. Online](/apps/offline-online/) for the storage and execution
matrix.

## Run from Desktop

1. Open the App and Flow in Flow-Like Desktop.
2. Confirm that the Flow mode permits local execution.
3. Configure any [runtime variables](/apps/runtime-variables/) required on this
   device.
4. Start the Flow from its entry node, a quick action, or a local Event.
5. Keep Desktop running for local scheduled or background Events.

Local execution identifies the host. The Flow can still call APIs, databases,
models, and other network services when its nodes and the machine's network
policy allow them.

## Computer-automation consent

Flows that control the computer or read the screen require explicit approval in
Desktop. A manual run can be approved once or remembered for the Flow. A local
Event can be approved for that Event so later API, chat, or scheduled triggers
do not need a foreground prompt.

Remembered approvals are stored on the current desktop. Operating-system
permissions—such as accessibility, screen capture, mouse, or keyboard
control—are separate and may also need to be granted.

:::caution[Approve only trusted automation]
Computer automation can view on-screen data and interact with other
applications as the signed-in user. Review the Flow, restrict its credentials,
and grant only the operating-system permissions it needs.
:::

## Build reliable local automations

- Prefer browser selectors or accessibility elements over fixed coordinates.
- Resolve the intended browser page, window, and display before interacting.
- Wait for an observable state instead of relying only on fixed delays.
- Bound retries and verify the result after consequential actions.
- Keep runtime values, paths, and credentials specific to the target machine.
- Capture diagnostics without including passwords, tokens, or sensitive screen
  regions.
- Test again after application, operating-system, theme, or display-scaling
  changes.
