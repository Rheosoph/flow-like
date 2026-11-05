---
title: Native widgets
description: Add Flow-Like launchers, data charts, and App Pages to Android, iOS, and macOS system surfaces.
---

Native widgets show cached Flow-Like content or open an App on Android, iOS, and macOS.
They are separate from [Home widgets](/start/home/) and reusable [A2UI
Widgets](/apps/widgets/). Available widget families depend on your installed
client and operating system.

## Choose a widget

Add native widgets for Open App, FlowPilot, Inbox, Needs attention, Recent runs, Recently
used apps, Usage, Workspace, Favorite Events, Bots, Bot tasks, or Bot approvals. To include an Event in
**Favorite Events**, open the Event's **Identity → Native integrations**,
select **Widgets**, enable its favorite setting, then save and activate the Event. On
iOS 18 and macOS 26 or later, system controls also open FlowPilot,
Notifications, or a selected Event. Widgets display cached app data; the
operating system controls their refresh schedule. Open the app to refresh a
stale widget. **Data Chart** and **App Page** widgets let you choose content
in **Settings → Native widgets**. App pages use a supported set of native
elements; widgets cannot display a live camera.

On Android, add **Flow-Like** from your launcher's widget picker, then choose
its content. Each widget has its own selection. Use its settings button to
change the selection, or resize it to show more rows. App launchers accept an
optional internal path. Bot task rows and task status notifications open the
task's conversation. Expired content requires opening Flow-Like to refresh.

## Create a Data Chart or App Page widget

Open **Settings → Native widgets** in Flow-Like on the device where you want
the widget. Choose **Data chart** or **App page**, give it a name, and select
its app. Preview the content, choose a color, and save. On iOS, add Flow-Like's
**Data Chart** or **App Page** from the Home Screen widget gallery, then edit
the widget to select the configuration you saved. Both types support small,
medium, and large sizes. On Android, add a Flow-Like widget and select your
saved chart or page in its configuration screen.

A Data Chart reads a table, ontology object type, or saved query. Configure
measures, grouping, filters, and number formatting before choosing a chart.
Supported types are single metric, progress, gauge, columns, horizontal bars,
stacked columns, line, area, donut, and pie. Progress and gauge require a
positive target. Aggregations run over the selected source before limiting
the result. Each widget displays up to 120 points and six series.
Android's compact chart renderer shows up to 30 distinct labels, or eight
horizontal bars. Open the app for the complete result.

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
the configured interval is not a background execution schedule. Android
also refreshes widgets from the cached snapshot when the launcher requests
an update. Content
expires after seven days without an update.

Android renders charts and pages as a bounded image with an accessibility
description. A tap opens the configured app destination. Page content is
limited to 60 elements and eight levels of nesting, and compact tables show
up to eight rows. Interactive controls remain available inside the app.

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

## Android controls, search, and continuation

Edit Android's Quick Settings panel to add **Bots**, **Events**,
**Ask FlowPilot**, or **Notifications**. Bots and Events open a picker using
the current workspace's cached entries. Actions require an unlocked device.
Quick Settings tiles are available on Android 7 and later.

On Android 12 and later, Flow-Like publishes eligible app, bot, and Event
names to AppSearch. Whether those results appear in system search depends
on the device's launcher and search provider. Entries follow the existing
system-search eligibility settings, expire with the snapshot, and are
removed when you clear the native cache or switch accounts. Search entries
contain titles and launch links, without conversation messages or task inputs.

To continue on another device, open a bot conversation or app page in
Flow-Like. Choose **Continue on another device** in widget configuration or
the Bots or Events Quick Settings picker, then use the Android share sheet.
The recipient needs Flow-Like open in the same account, profile, and hub.
The link resumes that page after checking the workspace and app origin;
it does not transfer credentials or run a task.

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
