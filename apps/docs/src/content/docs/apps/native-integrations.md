---
title: Bots, Siri, and native actions
description: Reach bots from mobile notifications and sharing, return answers to Shortcuts, and continue on another device.
---

Use native integrations to reach bots, Apps, and Events from your device's
notifications, share sheet, and shortcuts. Available surfaces depend on the
operating system and client build. For Home Screen and desktop content, use
[Native widgets](/apps/native-widgets/).

## Reach a bot on mobile

Open Flow-Like and select your profile before adding a bot widget or shortcut.
On iOS, **Open Bot**, **Ask Bot**, and **Run Bot Task** use the bots in that
profile. **Ask Bot** opens the app and returns the answer as Text to Shortcuts;
the response wait ends after 90 seconds. **Run Bot Task** sends the task's
current instructions once without changing its schedule. Deleted or paused
tasks cannot run through an old shortcut. Android provides configurable widgets,
launcher shortcuts, Direct Share targets, and Quick Settings tiles for bots,
Events, FlowPilot, and notifications.

Choose Flow-Like in another app's share sheet, then select the destination bot.
Bot notifications open the exact conversation. Eligible notifications also
offer a text reply, **Allow once**, or **Deny**. Requests for secret values open
the app for review. A notification from another account or profile cannot act
in your current workspace.

Running turns appear in iOS Live Activities and Android status notifications.
Their status comes from the bot host. iOS background refresh and Android
WorkManager can retry explicit plain-text notification replies and approval
decisions using the saved account session. Android renews that session through
the same credential store used by foreground sign-in, including after the app
process has stopped. Delivery remains queued through
temporary connection failures for up to 24 hours. Expired sign-in or a terminal
failure requires opening the app. Device scheduling and suspension can delay
delivery; use a server or active desktop host for continuous bot work.

## Capture and process bot input

In a bot's chat actions, choose dictation, document scanning, or a photo.
Review the text and attachments before pressing **Send to bot**. Capture
availability depends on the device; unavailable controls are hidden.

Android recognizes text from photos and scanned or selected PDF documents on
the device. The review screen includes that editable text and the original
attachments. Recognition is bounded to ten files, ten pages per PDF, and one
minute per request. An unreadable page leaves its attachment available.

Supported builds also offer **Summarize on this device** and **Extract details
on this device**. These actions process up to 8 KiB of UTF-8 text locally and
return editable output. They do not fall back to a cloud model. Sending the
reviewed result uses the selected bot's normal delivery path. Apple devices
require an available Foundation Models system model. Android may ask to
download a model before processing.

For Android builds, the Gradle property `enableBotLocalAi=true` includes the
optional ML Kit libraries and raises the minimum Android SDK to 26.
`enableBotAppFunctions=true` includes the experimental AppFunctions service
and requires compile SDK 37. AppFunctions can list or open bots, read cached
task status, and open the automation review screen described below. Both properties
default to `false`; ordinary bot sharing, notifications, and capture do not
require them.

## Enable native entry points

In the native app, open an Event's **Identity** settings and enable
**Native integrations**. Choose its system surfaces, then save and activate the
Event. Page and Chat launchers open their existing interface. Quick Actions,
forms, and API Events can run with their configured input defaults. Your current
access and the saved Event settings are checked again each time.
The Shortcuts **Input** accepts a JSON object keyed by Event input names and
overrides those defaults. For an Event with one String input, plain text also
works. Unknown inputs or values with the wrong type stop the run.
The **Automations** setting enables Siri, Shortcuts, and Android automation
entry points together. The **Search** setting covers Spotlight and Android
system search on supported launchers.

## Return a result to an Android automation

An Android caller can launch the `com.flow_like.app.AUTOMATE` intent with an
Activity result contract. Flow-Like opens a review screen where the user
chooses **Ask Bot**, **Ask FlowPilot**, **Ask App Chat**, **Run Event Text**,
**Run Event JSON**, or **Run Bot Task**, supplies the input, and presses **Run**.
The app returns the completed answer or Event output to the caller. A queued
request alone is not a successful result.

For an integration that already knows the target, supply `action_json` with
the native action object and `response_mode` set to `text` or `result`.
For example, `{"kind":"ask_bot","botId":"BOT_ID","prompt":"Summarize my open tasks"}`
prepares a bot question for review. Event actions use `kind: "run_event"`,
`appId`, `eventId`, and optional `text` input. The returned Intent contains
`status` and, when present, `text`, `json`, or `error`. Status is `success`,
`error`, or `interaction_required`. The same 256 KiB response limit described
below applies.

Android keeps the caller's request for up to 90 seconds. Cancelling the
automation stops its active local run, removes queued input, and rejects late
results. It cannot undo work a remote service already performed. Switching
account or workspace invalidates the request. Optional AppFunctions entries
open this same review screen; the Activity result contract is the interface
for callers that need a returned answer.

## Location Events and private copies on Android

**Get Current Location** uses native Android location services. Location Events
can monitor up to 100 regions with precise location permission. Background
regions also require **Allow all the time** in Android settings. Registrations
are restored after restart. Supported remote Events can receive crossings
without opening the app; local Events wait for the native engine to run.
Android scheduling can delay delivery. Remote crossings are retained for
14 minutes, and locally queued crossings for one hour.

In a bot's chat actions, choose **Private clipboard** to save text inside the
current workspace or paste a saved item for review. Private copies are encrypted
on the device and unavailable after their expiry or an account change. Workflow
clipboard nodes also support **Private clipboard** for text, HTML, and PNG
images, with a 3 MiB encoded item limit. Set **Expire after** to choose a
lifetime; private copies default to 30 minutes. HTML and images paste as
attachments. Sending the reviewed content follows the bot's normal delivery path.

Private copies stay inside Flow-Like. Android system clipboard writes still
work for ordinary copy and paste. A system clipboard request for strict expiry
returns an unsupported-options error. **Keep on device** requires an Android
version that can exclude remote clipboard sharing; otherwise the request fails
without copying the content. Choose **Private clipboard** when those guarantees
are required on older devices.

## Use an answer in another Shortcut action

- **Ask FlowPilot** takes a question and returns its final answer as Text.
- **Ask App Chat** takes a Chat Event and question and returns the chat reply as Text.
- **Run Event** returns an Event Result with **Text** and **JSON** properties.
  Chat replies populate Text. The **Return Generic Result** node supplies JSON,
  preserving objects, arrays, numbers, booleans, and null. String results also
  populate Text. Execution logs and intermediate reasoning are excluded.

For example, add **Ask FlowPilot**, enter a question, then add **Create Note**
and select the Ask FlowPilot output as the note's content. The same chain works
with **Ask App Chat**. For an object result, select its **JSON** property
and use **Get Dictionary from Input** before reading its fields.

On iOS, **Ask FlowPilot** runs through the native Rust engine without opening
chat. You can use its Text output in a Shortcuts automation. Open Flow-Like once
to select your profile, configure a model, and sign in before using the action.
Each request captures the published profile, Hub, and account. Rust refreshes
that profile's credentials and refuses the request if its account changes.
After restarting the device, unlock it once so the saved credentials are available.

Background questions have a 25-second deadline, and iOS may end execution
sooner. Cancelling the Shortcut or reaching that deadline cancels the FlowPilot
run. An interrupted request is not replayed automatically. Screen navigation,
questions, and approval requests return **unavailable**; FlowPilot can continue
work that needs no interaction. This action returns text and does not start
voice capture. Use **Open FlowPilot** to start voice in the app.

**Ask App Chat**, **Run Event**, and **Ask FlowPilot** on macOS bring Flow-Like
to the foreground and wait for execution. A question that needs more interaction
can continue in the app. Supply required Event inputs in the Shortcut; a missing
input or a page-only Event reports that interaction is needed. Their response
wait ends after 90 seconds, and the system may cancel it sooner. A run that
already started can continue in Flow-Like.
Responses are limited to 256 KiB in total, with at most 192 KiB each for Text
and JSON. Larger responses report an error rather than returning truncated data.

For a hosted **MCP** Event, select one registered tool in its native settings.
**Run Event** calls that selected tool with a JSON object in **Input** and
returns its JSON result. Widget launchers open an arguments dialog for review.
Changing or removing the selected tool invalidates an old shortcut. REST and
background trigger Events need a supported app interface before they can
become native entry points.

**Open App**, **Open Notifications**, and **Open Workspace** open their named
surfaces. The FlowPilot widget also has a voice entry point: start dictation in
the app, review the transcript, and press **Send**. If speech recognition is
unavailable, use keyboard dictation or type instead. Microphone capture stops
when the screen is hidden.

## Open an app at a chosen path

In Shortcuts, add Flow-Like's **Open App** action and choose an **App**. Leave
**Path** empty to open the app normally, or enter a path saved on one of its
active UI Events, such as `/orders/123?tab=details`. The path stays inside the
chosen app. A missing or inactive route shows an error instead of opening a
different page.

For a Home Screen or desktop launcher, add the **Open App** widget and edit it.
Choose the app, optional **Internal path**, and optional **Widget title**.
The widget opens that destination when tapped; it does not render the page
inside the widget. Open Flow-Like in the desired account and profile first
to refresh the app picker.

Both the Shortcut and widget have separate lists of query names and values.
Pair them by position, keeping the lists the same length. Enter values as
plain text, including spaces, Unicode, `+`, `%`, `&`, and `#`. Flow-Like encodes
them for the link. For example, the value `A+B & 東京 50%` reaches the Event
unchanged. Do not percent-encode values in these lists yourself.

A query already written in **Path** uses normal URL syntax: `+` means a space,
and `%2B` means a literal plus. Structured pairs are appended after that query,
so names can repeat. With `/orders?tag=first` and another pair `tag: second`,
**Get Query Params** returns `second` for `tag`. The Event payload also includes
`_query_param_values.tag` as `["first", "second"]`, preserving every value in
order. Names such as `id`, `eventId`, and `route` remain app data and cannot
change the selected app. Literal names such as `_id` remain distinct from `id`.

Use at most 32 pairs in total. Paths are limited to 4,096 UTF-8 bytes, names to
256 bytes, values to 4,096 bytes, and the combined path and structured pairs to
8,192 bytes. External URLs, traversal segments, control characters, and raw
fragments in **Path** are rejected. A `#` in a structured value is supported.

## Spotlight and Handoff

Spotlight can open FlowPilot, usable apps, and Events selected for Spotlight.
Handoff can continue an exposed page or chat on another device signed into the
same account and profile when the hub supplies an HTTPS app address. For a
configured app path, Handoff includes that path and its app-owned query values.
Unrelated outer URL fields are excluded.

See [Native widgets](/apps/native-widgets/#cached-data-and-account-scope) for cached data and account scope.
