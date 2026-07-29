---
title: Widgets
description: Build, version, and reuse A2UI component groups
sidebar:
  order: 2
---

A Widget is a reusable A2UI component graph stored in an app. It can be
rendered on its own for preview, inserted into a Page as a Widget instance,
and configured with named action IDs.

Widgets are managed in the app's **Widgets** workspace.

![The Widgets workspace in Flow-Like Desktop](../../../../assets/WidgetsOverview.webp)

## Widget Lifecycle

1. Create a Widget with a name and optional description.
2. Open its details view to preview it and edit its metadata.
3. Open the Visual Builder to compose its A2UI surface.
4. Define named Widget events when the block needs to expose interaction.
5. Create Patch, Minor, or Major snapshots when you need a stable version.
6. Add the Widget from a Page's component palette.

There is no `.widget` export/import flow in the current UI. Reuse happens through the Widget records available to the signed-in user and the references stored with a Page.

## Details and Builder

The Widget details view and Widget Builder serve different purposes.

| View | Purpose |
| --- | --- |
| **Widget details** | Rendered preview and descriptive metadata |
| **Widget Builder** | Component graph, data bindings, styles, actions, and Widget settings |

Select **Open Builder** from the details view.

![The Widget Builder editing a reusable A2UI surface in dark mode](../../../../assets/WidgetBuilder.webp)

The builder uses the same Components, Hierarchy, Canvas, Inspector, Dev Mode, and responsive Preview as a Page. See the [Visual Builder guide](/dev/a2ui/visual-builder/) for those controls.

## Current Widget Model

```typescript
interface IWidget {
  id: string;
  name: string;
  description?: string;
  rootComponentId: string;
  components: SurfaceComponent[];
  dataModel: DataEntry[];
  customizationOptions: CustomizationOption[];
  exposedProps?: ExposedProp[];
  actions?: WidgetAction[];
  tags: string[];
  catalogId?: string;
  thumbnail?: string;
  version?: [number, number, number];
  createdAt: string;
  updatedAt: string;
}
```

The core surface fields are:

- `rootComponentId` — entry point into the flat component graph;
- `components` — the A2UI components rendered by the Widget;
- `dataModel` — values that bound component properties can read;
- `actions` — named interactions exposed by the Widget;
- `customizationOptions` and `exposedProps` — stored instance-configuration metadata;
- `version` — the loaded or latest semantic version tuple.

New Widgets start with `rootComponentId: "root"` and empty component, data-model, customization, action, and tag collections.

## Widget Settings

Select **Settings** in the Widget Builder header.

### General

Edit the name, description, and tags, then select **Save Metadata**. These settings describe the Widget in the library; they do not change an instance on a Page.

### Events

Widget events define IDs that elements inside the Widget can select:

```typescript
interface WidgetAction {
  id: string;
  label: string;
  description?: string;
  icon?: string;
  contextSchema: WidgetActionContextField[];
}
```

The current settings editor creates an ID, label, optional description, and an initially empty context schema.

To use an event:

1. Add it in **Widget Settings → Events**.
2. Select a Button or another interactive component.
3. Select the event in the action editor. The stored action name is
   `widget_event`, and `context.actionId` is the Widget event's `id`.
4. Insert the Widget into a Page.
5. Populate the Widget instance's action binding through a host that mounts the instance editor, or through validated JSON tooling.

For example:

```json
{
  "actions": [
    {
      "name": "widget_event",
      "context": {
        "actionId": "acknowledge"
      }
    }
  ]
}
```

Do not put `acknowledge` in the action's `name` field. The current action
handler only resolves a Widget binding when the name is exactly
`widget_event`.

This separates reusable presentation from the Page-specific workflow that
handles it.

### Versions

The Versions tab can:

- load an existing Widget version;
- show version history;
- create a Patch, Minor, or Major snapshot.

Creating a version first saves the current Widget and then records a new version. The editor URL represents a selected version as:

```text
/widget?id=<widgetId>&app=<appId>&version=<major>_<minor>_<patch>
```

Use the version type to communicate compatibility:

| Version type | Use when |
| --- | --- |
| **Patch** | Correcting a compatible implementation detail |
| **Minor** | Adding compatible behavior or options |
| **Major** | Changing behavior in a way existing Pages may need to review |

### Advanced

Advanced displays the Widget ID, root component ID, current version, timestamps, component count, and data-model entry count. These fields are informational in the current panel.

## Saving

Component changes update local editor state immediately and save after a short debounce. The header shows **Saving**, **Unsaved changes**, or **Saved** and offers **Save Now** when changes are pending.

Metadata and Widget events are saved explicitly from their Settings tabs.

## Insert a Widget into a Page

The component palette loads Widgets available to the user and groups them by project. Drag a Widget onto a compatible Page container.

The builder then:

1. loads the Widget definition;
2. stores it in the Page's `widgetRefs` under a new instance ID;
3. creates a `widgetInstance` component;
4. adds that component ID to the selected parent;
5. initializes empty exposed-property values and action bindings.

The Page owns the instance configuration while retaining a reference to the reusable Widget definition.

The inserted component currently has this shape:

```json
{
  "type": "widgetInstance",
  "instanceId": "widget-status-card-…",
  "widgetId": "status-card",
  "appId": "app-id",
  "exposedPropValues": {},
  "actionBindings": {}
}
```

At runtime, the renderer looks up the Widget by the instance ID in the Page's `widgetRefs`. If no inline reference exists, it can fetch the Widget by app and Widget ID. It applies `exposedPropValues` to the definition's exposed properties and provides `actionBindings` to the Widget's action context. For `widget_event`, the current handler looks up `actionBindings[actionId]` and executes workflow bindings; command bindings are not executed by this path.

:::note[Current editor boundary]
The repository exports a dedicated `WidgetInstanceInspector` with customization and workflow/command binding controls, but the current Page Builder does not mount that inspector. The normal insertion flow therefore creates empty instance values and bindings. Do not document those controls as available in the Page Builder until the host wires them in.
:::

## State API

```typescript
interface IWidgetState {
  getWidgets(
    appId: string,
    language?: string,
  ): Promise<[string, string, IMetadata | undefined][]>;
  getWidget(
    appId: string,
    widgetId: string,
    version?: [number, number, number],
  ): Promise<IWidget>;
  createWidget(
    appId: string,
    widgetId: string,
    name: string,
    description?: string,
  ): Promise<IWidget>;
  updateWidget(appId: string, widget: IWidget): Promise<void>;
  deleteWidget(appId: string, widgetId: string): Promise<void>;
  createWidgetVersion(
    appId: string,
    widgetId: string,
    versionType: "Major" | "Minor" | "Patch",
  ): Promise<[number, number, number]>;
  getWidgetVersions(
    appId: string,
    widgetId: string,
  ): Promise<[number, number, number][]>;
}
```

Metadata has separate `getWidgetMeta` and `pushWidgetMeta` operations so library copy can be localized independently of the Widget surface.

## Related Guides

- [Pages](/dev/a2ui/pages/) — compose Widgets into app experiences
- [Visual Builder](/dev/a2ui/visual-builder/) — edit the component graph
- [A2UI overview](/dev/a2ui/overview/) — understand surfaces and actions
