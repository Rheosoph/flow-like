---
title: Siri, Shortcuts, and Handoff
description: Open Apps and Events, return workflow results to Shortcuts, and continue on another device.
---

Use native integrations in the iOS and macOS apps to reach existing Flow-Like
entry points. The available system surfaces depend on the operating-system and
client version. For Home Screen and desktop content, use [Native widgets](/apps/native-widgets/).

## Enable native entry points

In the iOS and macOS apps, open an Event's **Identity** settings and enable
**Native integrations**. Choose its system surfaces, then save and activate the
Event. Page and Chat launchers open their existing interface. Quick Actions,
forms, and API Events can run with their configured input defaults. Your current
access and the saved Event settings are checked again each time.
The Shortcuts **Input** accepts a JSON object keyed by Event input names and
overrides those defaults. For an Event with one String input, plain text also
works. Unknown inputs or values with the wrong type stop the run.
Siri and Shortcuts share one action catalog and are enabled together.

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

These actions bring Flow-Like to the foreground and wait for execution. A
question that needs more interaction can continue in the app. Supply required
Event inputs in the Shortcut; a missing input or a page-only Event reports that
interaction is needed. The response wait ends after 90 seconds, and the system
may cancel it sooner. A run that already started can continue in Flow-Like.
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
