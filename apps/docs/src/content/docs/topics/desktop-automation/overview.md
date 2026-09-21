---
title: Desktop & Browser Automation
description: Automate browsers and desktop applications with Flow-Like's current Browser, Computer, RPA, Vision, Selector, and LLM nodes
sidebar:
  order: 1
---

Flow-Like can automate browser pages and visible desktop applications from a Flow. The current Automation catalog includes direct browser and computer control, accessibility-based element lookup, template matching, reusable selectors, checkpoints, recovery helpers, and optional AI-assisted observation.

:::note[Run automation where the interface is available]
Browser and computer automation need a compatible local execution environment. Desktop interactions also depend on the operating system, active session, application, and permissions. Confirm those requirements on the machine that will run the Flow.
:::

![The Flow-Like desktop automation strategy: choose a browser, computer, or vision surface, resolve the target deterministically, interact, and verify the result](../../../../assets/DesktopAutomationOverview.svg)

## Choose the right automation surface

| Surface | Best for | How it targets elements |
|---------|----------|-------------------------|
| **[Browser](/nodes/automation/browser/)** | Web applications, forms, downloads, and page extraction | Browser selectors and page state |
| **[Computer](/nodes/automation/computer/)** | Native desktop applications and system UI | Accessibility elements, windows, coordinates, and direct input |
| **[Vision](/nodes/automation/vision/)** | Interfaces without stable selectors or accessibility metadata | Image templates, regions, and pixel colors |
| **[RPA](/nodes/automation/rpa/)** | Reliable orchestration around UI interactions | Retries, timeouts, checkpoints, assertions, and recovery |
| **[Selector](/nodes/automation/selector/)** | Reusable target definitions with multiple fallback strategies | Ranked selector sets |
| **[Fingerprint](/nodes/automation/fingerprint/)** | Matching a target across UI changes | Stored and compared element fingerprints |
| **[LLM](/nodes/automation/llm/)** | Observation, planning, and recovery when deterministic targeting is insufficient | Configured vision-capable models |

Prefer the most deterministic surface available. Browser selectors are usually better than screen coordinates for a web page; accessibility elements are usually better than image matching for a native control. Vision and LLM nodes are useful fallbacks, not substitutes for a stable target.

## Automation sessions

Use [Start Automation Session](/nodes/automation/automation-start-session/) at the beginning of a desktop automation and [Stop Automation Session](/nodes/automation/automation-stop-session/) when it finishes. Keeping the session explicit makes resource ownership and cleanup visible in the Flow.

A typical desktop recipe is:

1. Start an automation session.
2. Find or focus the target window.
3. Locate an accessibility element, template, or coordinate.
4. Perform the interaction.
5. Assert the expected result or take a checkpoint.
6. Stop the session on both success and failure paths.

## Browser automation

The Browser catalog covers the complete page lifecycle:

| Capability | Current nodes |
|------------|---------------|
| Lifecycle | **Start WebDriver**, **Stop WebDriver**, **Attach to Browser**, **Open Browser**, **New Page**, **Close Page**, **Close Browser** |
| Context | **List Tabs**, **Select Tab**, **Enter Frame**, **Leave Frame**, **Handle Browser Dialog** |
| Navigation | **Go To URL**, **Go Back**, **Go Forward**, **Reload** |
| Interaction | **Click Element**, **Double Click Element**, **Right Click Element**, **Drag Element**, **Hover Element**, **Scroll Into View** |
| Input | **Type Text**, **Press Key**, **Key Chord**, **Select Option** |
| Waiting | **Wait For Selector**, **Wait Delay**, **Wait For Network Idle** |
| Extraction | **Get Text**, **Get Attribute**, **Get HTML**, **Execute JavaScript** |
| Capture | **Take Screenshot**, **Screenshot Element** |
| Files | Uploads, download directory configuration, download triggers, and download waiting |
| State | Cookie, local-storage, and session-storage operations |
| Diagnostics | Console logs, network requests, DOM snapshots, and accessibility snapshots |

### Recipe: submit a web form reliably

1. [Open Browser](/nodes/automation/browser/browser-open/) and create a [New Page](/nodes/automation/browser/browser-new-page/). Start the Network Observer before navigation if you plan to wait for network idle.
2. Navigate with [Go To URL](/nodes/automation/browser/navigation/browser-goto/).
3. Use [Wait For Selector](/nodes/automation/browser/wait/browser-wait-for/) before interacting.
4. Enter values with [Type Text](/nodes/automation/browser/input/browser-type-text/) and submit with [Click Element](/nodes/automation/browser/interact/browser-click/).
5. Wait for either the result selector or [network idle](/nodes/automation/browser/observe/browser-wait-for-network-idle/).
6. Capture the final state with [Take Screenshot](/nodes/automation/browser/capture/browser-screenshot/).
7. Close the browser in the cleanup path.

Use selector-based browser nodes for browser content. They accept the typed
Selector output as well as existing CSS strings. Keep the returned session
handle for the intended tab and frame; switching one branch does not retarget a
different branch's handle.

**Start Network Observer** must run before the requests you want to observe.
**Wait For Network Idle** uses outstanding requests from that observer. Network
and console observers and HTTP Basic authentication use Chrome or Edge's CDP
connection. Basic authentication credentials are restricted to the configured
HTTP(S) origin and are not exposed to page JavaScript. Standard page actions use
WebDriver; protocol-specific nodes report an error on an unsupported browser.

Use **Start WebDriver** with an installed, compatible driver or provide an
existing endpoint. **Attach to Browser** connects to an explicitly configured
Chrome or Edge debugging instance. Closing the automation session disconnects
an attached browser; it does not own the browser's lifetime.

Coordinate-based desktop input is more fragile when zoom, layout, or window
position changes.

## Computer automation

Computer nodes interact with the active desktop session.

### Accessibility

[Get Accessibility Tree](/nodes/automation/computer/accessibility/computer-get-accessibility-tree/) inspects the accessible controls exposed by the current interface. [Find Accessibility Element](/nodes/automation/computer/accessibility/computer-find-accessibility-element/) locates a target from that structure.

**Act on Accessibility Element** invokes, focuses, or sets a value on the identified
native control. It checks that the target still matches before acting.

Accessibility targeting is the preferred starting point for native controls because it can remain stable across window movement and display scaling. Some custom-rendered applications expose little or no useful accessibility metadata; use Vision or a coordinate fallback for those interfaces.

### Windows and displays

The current Window nodes can:

- list windows and inspect the active window;
- find a window by title;
- focus a window;
- minimize, maximize, restore, move, resize, or close a window;
- capture a window;
- launch an application.

The Display nodes list displays and identify the primary display. Resolve the intended display or window before using absolute coordinates.

### Mouse, keyboard, and clipboard

The Computer catalog includes:

- **Mouse Move**, **Natural Mouse Move**, **Mouse Click**, **Mouse Double Click**, **Mouse Drag**, and **Scroll**;
- **Type Text** and **Key Press** for keyboard input;
- text and image clipboard getters and setters;
- **Wait** for deliberate pauses between interactions.

**Set Clipboard Text** and **Set Clipboard Image** also work outside desktop automation. They use the device API when the Event runs locally and ask the invoking frontend to copy the result when it runs remotely. Their automation session input is optional, and existing session connections still pass through. Clipboard readers continue to require local desktop execution.

A remote write needs a live client invocation. A scheduled or API-only run has no user clipboard to address. The write follows its success output only after the client acknowledges it; otherwise it follows the error output with a structured error. Browsers may show a **Copy result** button when they require a fresh click. Requests expire when their client timeout elapses or the run ends and are never executed from saved run output.

The optional **Keep on device** and **Expire after** inputs use iOS clipboard protections. A host that cannot enforce these options returns an error. Text is limited to 1 MiB and images to 16 MiB of PNG data. Writing clipboard content does not send a paste keystroke to another application.

Use [Natural Mouse Move](/nodes/automation/computer/mouse/computer-natural-mouse-move/) when the path itself matters. Use [Mouse Click](/nodes/automation/computer/mouse/computer-mouse-click/) or [Click At Position](/nodes/automation/rpa/rpa-click-at-position/) only after resolving the correct coordinates for the current session.

## Screen capture and visual targeting

The current catalog separates general computer capture from Vision helpers:

| Task | Node |
|------|------|
| Capture the desktop | [Screenshot](/nodes/automation/computer/capture/computer-screenshot/) |
| Capture a rectangular region | [Screenshot Region](/nodes/automation/vision/vision-screenshot-region/) |
| Save a capture to a file | [Screenshot To File](/nodes/automation/vision/vision-screenshot-to-file/) |
| Locate one template | [Find Template](/nodes/automation/vision/vision-find-template/) |
| Locate every matching template | [Find All Templates](/nodes/automation/vision/vision-find-all-templates/) |
| Find and click a template | [Click Template](/nodes/automation/vision/vision-click-template/) |
| Wait for appearance or disappearance | **Wait For Template** / **Wait Template Disappear** |
| Inspect the display | **Get Screen Size** / **Get Pixel Color** |

### Recipe: resilient template interaction

1. Capture a current region rather than searching an unnecessarily large display.
2. Use **Wait For Template** with a deliberate timeout.
3. Locate the template and inspect its match before clicking when the action is consequential.
4. Click with **Click Template** or use the returned position with a mouse node.
5. Assert that the expected follow-up template or color exists.
6. On failure, take a snapshot and enter a recovery path.

Template and fingerprint modes require a unique current target. A missing or
ambiguous match stops the action. Choose coordinate mode explicitly when a Flow
should click a fixed location without validating a visual or accessible target.

Template images should be cropped around a distinctive, stable control. Recreate
them when the target application's theme, display scaling, or visual design
changes.

## Reliability and recovery

RPA helpers make failures explicit instead of hiding them inside a long chain of UI actions.

| Concern | Useful nodes |
|---------|--------------|
| Bounded execution | **With Timeout**, **Delay**, **Calculate Elapsed** |
| Retry | [Retry Loop](/nodes/automation/rpa/rpa-retry-loop/), **Wait For Template**, **Wait For Color** |
| Assertions | **Assert Template Exists**, **Assert Color At Position** |
| Checkpoints | **Save Checkpoint**, **Parse Checkpoint**, [Take Snapshot](/nodes/automation/rpa/rpa-take-snapshot/) |
| Error paths | **Try Catch**, **Error Recovery**, **Diagnose Failure** |
| Audit trail | **Log Action** |

For an important interaction:

1. Bound the operation with a timeout.
2. Wait for a deterministic readiness signal.
3. Perform the action.
4. Assert the resulting state.
5. Retry only failures that are safe to repeat.
6. Capture diagnostic evidence before recovery or exit.

**With Timeout** cancels the action branch cooperatively. An operating-system or browser request already submitted may still complete, so verify the target state before retrying an action with external effects. Retry only when the operation is idempotent or you can determine whether it already succeeded.

## Selectors and fingerprints

Selectors let a Flow keep several ways to identify the same target. Build a selector, combine alternatives into a selector set, validate them, rank the candidates, and retrieve the best current match.

Fingerprints store descriptive target data that can be compared or updated later. They are useful when a target changes slightly between versions but retains enough stable characteristics to identify it.

Use these layers to make fallbacks deliberate:

1. stable browser or accessibility selector;
2. alternate selector or stored fingerprint;
3. template match;
4. coordinate fallback;
5. optional LLM-assisted resolution.

## Optional LLM assistance

LLM automation nodes cover three groups:

- **Vision**: observe or classify a screen, find or describe an element, extract structured information, resolve candidates, and rank matches;
- **Planning**: plan actions or suggest the next step;
- **Healing**: diagnose a failure and propose a repaired selector or template.

Use a configured model only when deterministic methods do not provide enough signal. Treat its output as a proposal: validate the selected target and add a bounded fallback before performing consequential actions.

**Plan Actions** defaults to general proposals, including native desktop targets.
To feed **Execute Browser Action Plan**, select its **Browser** execution target
and supply **Page Context** from a DOM or accessibility snapshot. Browser plans
must use selectors grounded in that context; the executor validates supported
actions and their parameters before running them.

:::caution[Review data handling]
Screen captures and extracted UI content may contain personal, confidential, or regulated information. If a Flow sends that content to a configured model or external service, the applicable provider, connection, and organizational policies govern where it is processed. Minimize the captured region and redact sensitive values when possible.
:::

## Recording

The recorder creates editable nodes from your actions. Native recording waits
for the input hook to start before showing a recording state. Stop it with the
configured global shortcut or the tray menu, including while another application
has focus. While Desktop stays open, a completed recording remains available until
you insert or clear it on its original profile, app, and board. Insert recordings
before quitting Desktop. Switching profiles stops active recording
and clears its preview. Opening another profile or board cannot insert or discard
the retained recording.

Native recording preserves mouse buttons, modifiers, scroll positions, window
focus, and clipboard shortcuts. Optional target images and accessibility
fingerprints use a recent hover sample from the same position and window,
taken before the click changes the control. If no matching sample is available,
the recording keeps the action coordinates. Inserting with template or fingerprint
replay enabled requires the corresponding sample; choose coordinate replay when
that evidence is unavailable.

For web pages, select browser recording and supply the debugger address of a
Chrome or Edge instance started with remote debugging, plus its compatible
WebDriver endpoint. Browser recording creates selector-based actions, waits for
their targets, and keeps tab and frame context. It does not require desktop Input Monitoring. The
browser must expose its debugging endpoint before recording begins. Cross-origin
frames and shadow-DOM targets cannot currently be recorded as document selectors;
the recorder reports that limitation. Add frame or custom targeting steps to the
Flow when a page requires them. Password and file-input values are excluded
from browser recordings; configure those steps explicitly in the Flow.

## Permissions and operating-system behavior

[Automation approval](/studio/local-execution/#automation-approval-and-system-permissions)
is separate from operating-system access. Desktop inspects the actual workflow
revision, asks for its capabilities, and checks them again at native execution.
Background Events require a remembered approval.

| Platform | Native integration and requirements |
| --- | --- |
| macOS | Accessibility supports native element lookup and actions, window control, and input. Screen Recording is checked separately for captures. Native recording checks Input Monitoring and Accessibility for window identity. Grants belong to the current executable and signing identity. |
| Windows | Native elements use UI Automation. Input and window actions run in the interactive desktop; elevated applications and the secure desktop can reject them. |
| Linux with X11 | Input and capture require an accessible display. Window control uses native X11 messages and verifies the window manager's supported operations and resulting state. Native accessibility uses the AT-SPI session bus and the target application's exposed controls. |
| Linux with Wayland | Input control uses a compositor RemoteDesktop session retained for the app session. Select the displays to control in its portal; absolute movement requires their logical position and size. Pointer-location queries are unavailable through this backend. Capture availability depends on the screenshot backend and may require approval for each capture. Global native input recording and generic native window management are not available through this backend; browser recording and browser automation remain available. |
| iOS and Android | The desktop automation catalog is unavailable. Device clipboard operations use the separate device integration. |

The permission dialog identifies denied, unavailable, and unsupported
capabilities. Recheck after granting access. If macOS still reports denial,
quit and reopen the app and verify that System Settings lists the executable
you are running.

Test capture, input, and target lookup on the machine that will run the Flow.
Repeat that check after operating-system, application, theme, or display changes.

When upgrading an existing Flow, review collection connections on **Upload
Multiple Files**, **Observe Screen**, **Plan Actions**, **Rank Candidates**,
and **Resolve Element**. These nodes now declare their array item types.
Before catalog synchronization, migration checks the changed pins against their
new contracts and preserves compatible saved array literals and connections.
Repair values that fail the new item schema, and connections into typed inputs
from incompatible producers or generic producers whose item type cannot be
determined. Validate the Flow before running it. Existing CSS selector pins and
their connections are preserved alongside the added typed locator inputs.

Saved mouse nodes retain an explicitly enabled fingerprint option. Supply a
valid fingerprint or turn that option off for coordinate replay; missing target
evidence now stops the action.

## Design checklist

- Start with a stable target, not a fixed delay.
- Resolve the intended window, page, and display before interacting.
- Prefer selectors or accessibility metadata over coordinates.
- Keep retries bounded and safe to repeat.
- Assert the post-condition after important actions.
- Capture diagnostics without exposing secrets.
- Provide cleanup paths that close pages, browsers, and automation sessions.
- Test at the same display scaling and permissions used in production.

## Next steps

- Browse the complete [Automation node catalog](/nodes/automation/).
- Use [Document Processing](/topics/document-processing/overview/) for extracted documents and images.
- Use [API Integrations](/topics/api-integrations/overview/) when the target system exposes a reliable API.
- Use [Building Internal Tools](/topics/internal-tools/overview/) to create a control surface for an automation.
