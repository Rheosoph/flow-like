---
title: Customize your Home
description: Arrange Home widgets, save a personal layout, and choose which defaults to inherit.
---

Home brings your Apps, FlowPilot, workspace activity, and discovery widgets into
one page. Choose **Customize** to add widgets, change their content, or arrange
them for the active Profile. Web and desktop use the same editor.

## Save a personal layout

1. Select the Profile whose Home you want to change.
2. Open **Customize**, then **Add widget** to choose a widget. Select an existing
   widget to edit its content, data source, size, or appearance.
3. Drag widgets into position. With the keyboard, pick up a widget using its
   move handle, use the arrow keys to choose a position, and press Space to
   place it. Escape cancels the move or an active pointer resize.
4. Choose **Save** to apply the layout. **Cancel** lets you discard unsaved
   changes. Undo and redo are available while editing.

Finish an active move or resize before saving. If a save fails, the editor
retains the draft so you can retry.

## Add an image

In **Customize → Add widget → Content**, choose **Image and caption**. In its
settings, choose an **Image source**:

- **URL**: enter an HTTP(S) image URL or a path on the current site.
- **App storage**: choose an App, browse its shared storage folders, and select
  an image. Viewers need permission to read that App's files.

Add an **Image description** for people using a screen reader. You can also add
a caption in **Content**. Editorial stories and banners support the same image
sources.

Storage images keep their App and file path in the layout. Home obtains a
temporary download URL through the App's storage API and refreshes it before
it expires. Your browser caches the image according to the storage provider's
HTTP cache settings. If a storage image fails to load, check your connection and
file permissions, then choose **Try again**.

## Understand defaults

Home uses the first available layout in this order:

1. Your saved personal layout for the active Profile.
2. The published default assigned to that Profile.
3. The installation's published main default.
4. The starter bundled with Flow-Like.

A personal layout takes precedence over later changes to published defaults.
To return to inherited defaults, open **Customize → Layout options → Reset to
default**, confirm the reset, then save. You can undo the reset before saving.

To start from the current bundled design instead, choose **Use Flow-Like
starter** in the same menu. This loads an editable draft. It does not change
your saved layout until you save, and it does not publish an installation-wide
default. Administrators manage shared defaults separately; see
[platform administration](/dev/platform-administration/).

When you use an inherited layout and its published default cannot be loaded,
Home uses its last available cached default or the bundled starter. A failure to
load the Profile itself shows a retry action. Check that the intended Profile is
active before retrying a save.

## Size widgets for their content

**Match row** aligns a widget's surface with its neighbors. **Fit content**
keeps its natural height. Choose a row count or resize the widget when you need
a fixed height. On narrow screens, widgets form a single column.

The widget's **surface** controls its frame and accent. Collection **card
style** controls how the Apps, models, or packages inside it are displayed.
These are separate choices. For manual App collections, you can reorder and
remove selected Apps; automatic collection filters do not override that manual
selection.

Embedded Apps keep their own page navigation and query parameters. Their
preview pauses while you edit Home. Account activity widgets report the
available account execution records; an unavailable data source should be
retried rather than interpreted as zero activity.

## Style a widget with Tailwind classes

To go beyond the built-in surfaces and accents, select a widget and enter
Tailwind CSS utility classes in **Tailwind classes**, below **Accent** in its
settings. Separate classes with spaces. In **Customize → Layout options → Edit
JSON**, the same value is the widget's `appearance.className`.

The classes apply to the widget's surface after its surface and accent style,
so your background, border, radius, shadow, and text color replace the
built-in ones. Home compiles the classes at runtime, so arbitrary values such as
`p-[18px]` work, and scopes them to that widget. The layout still controls the
widget's position, width, height, and alignment; classes for those have no
effect. The value can be up to 1024 bytes.

For example:

- Highlighted KPI card: `rounded-3xl border-primary/40 bg-primary/10`
- Gradient hero: `bg-linear-to-br from-primary/20 to-transparent`
- Larger title on a **Borderless** surface: `[&_h2]:text-2xl`

Some tips:

- Style elements inside the widget with arbitrary variants, such as
  `[&_h2]:text-lg`.
- Use Tailwind CSS v4 class names, for example `bg-linear-to-br` rather than
  `bg-gradient-to-br`.
- Prefer theme tokens such as `bg-card`, `text-primary`, and
  `border-primary/30` over fixed colors so the widget still looks right in
  light and dark mode.
