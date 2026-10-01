---
title: Native widgets
description: Add Flow-Like launchers, data charts, and App Pages to iOS and macOS system surfaces.
---

Native widgets show cached Flow-Like content or open an App on iOS and macOS.
They are separate from [Home widgets](/start/home/) and reusable [A2UI
Widgets](/apps/widgets/). Available widget families depend on your installed
client and operating system.

## Choose a widget

Add native widgets for Open App, FlowPilot, Inbox, Needs attention, Recent runs, Recently
used apps, Usage, Workspace, or Favorite Events. To include an Event in
**Favorite Events**, open the Event's **Identity → Native integrations**,
select **Widgets**, enable its favorite setting, then save and activate the Event. On
iOS 18 and macOS 26 or later, system controls also open FlowPilot,
Notifications, or a selected Event. Widgets display cached app data; the
operating system controls their refresh schedule. Open the app to refresh a
stale widget. **Data Chart** and **App Page** widgets let you choose content
in **Settings → Native widgets**. App pages use a supported set of native
elements; widgets cannot display a live camera.

## Create a Data Chart or App Page widget

Open **Settings → Native widgets** in Flow-Like on the device where you want
the widget. Choose **Data chart** or **App page**, give it a name, and select
its app. Preview the content, choose a color, and save. Then add Flow-Like's
**Data Chart** or **App Page** from the iOS Home Screen widget gallery. Edit
the widget to select the configuration you saved. Both types support small,
medium, and large sizes.

A Data Chart reads a table, ontology object type, or saved query. Configure
measures, grouping, filters, and number formatting before choosing a chart.
Supported types are single metric, progress, gauge, columns, horizontal bars,
stacked columns, line, area, donut, and pie. Progress and gauge require a
positive target. Aggregations run over the selected source before limiting
the result. Each widget displays up to 120 points and six series.

An App Page displays a published page, or one container within it. Choose
its internal path and enter query values as plain text; Flow-Like handles
link encoding. The native renderer supports text, images, icons, cards,
badges, progress, compact tables, common charts, and basic layouts. Buttons
and links open the app. Inputs, camera and audio elements, and other
interactive components need the full app. The preview lists content that
cannot appear in the widget.

Open the selected app page to capture content populated by workflows or
user input. Widget refreshes do not run page lifecycle workflows. Charts
refresh at the configured interval while Flow-Like is open and visible.
When the app is closed, widgets display the last cached content and its
update time. iOS controls when that content appears on the Home Screen;
the configured interval is not a background execution schedule. Content
expires after seven days without an update.

Saved configurations and cached content belong to the selected account,
profile, and hub on this device. Switching accounts clears the active
native content. Open the intended workspace before choosing a saved widget
in the gallery. You can save up to 12 configurations per workspace.

## Open a chosen App path

Add the **Open App** widget, edit it, and choose an App, optional **Internal
path**, and **Widget title**. It opens that destination when tapped. Open
Flow-Like in the intended account and Profile first to refresh its App picker.
See [path and query rules](/apps/native-integrations/#open-an-app-at-a-chosen-path)
for encoding, limits, and examples.

## Cached data and account scope

**Recently used apps** records actual app openings on this device, separately
for each account and profile. It does not use the app's last edit date.
**Needs attention** lists Error and Fatal execution records from the last seven
UTC calendar days, including today. **Usage** shows account-wide execution and
model totals across profiles, covering all recorded usage. Recorded AI cost
includes model and embedding costs in USD. **Workspace** counts apps in the
selected profile; its execution count covers the whole account across profiles
over the last seven UTC calendar days, including today.
Available live execution metadata also feeds Recent runs and iOS Live
Activities while the app receives run updates.

App and Event pickers in Shortcuts show cached app artwork when available.
The Open App widget, recent app rows, and workspace app tiles use the same
artwork, with initials as a fallback. Apple controls the icons on top-level
Shortcut action tiles.

The device's system cache contains display names, status, navigation targets,
and small copies of app artwork, without Event configuration, input defaults,
or access tokens. Answer requests and results are stored briefly in the shared
app container so Shortcuts can receive the response. Account
or workspace changes clear the native cache and invalidate queued actions.
