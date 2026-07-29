---
title: Pages
description: Build app Pages and connect them to events, routes, and flows
sidebar:
  order: 1
---

A Page is an app-specific A2UI surface. Its component graph defines the interface; Page settings define presentation, lifecycle behavior, and metadata.

Pages are created from a Flow and retain that Flow's `boardId`. They appear across the app in the **Events → Pages** workspace.

![The Pages workspace in Flow-Like Desktop](../../../../assets/PagesOverview.webp)

## Page, Event, and Route

These are separate records:

| Record | Owns |
| --- | --- |
| **Page** | Components, widget references, canvas settings, layout, lifecycle hooks, and metadata |
| **Event** | The board and node to run, execution mode, event interface, and optional `default_page_id` |
| **Route** | Only a URL `path` and the `eventId` that handles it |

A navigable Page is resolved in two steps:

1. The route maps a pathname to an app event.
2. The event's `default_page_id` selects the Page.

The Page does not need a separate route object pointing directly to it. See [Routes](/dev/a2ui/routes/) for the current mapping API.

## Create a Page

1. Open the Flow that will provide the Page's behavior.
2. Open its **Pages** panel and create a Page.
3. Flow-Like opens `/page-builder` with the Page, app, and board IDs in the query string.
4. Build the surface and configure Page settings.
5. Create or edit a UI Event, select the Page as its target, and assign the Event a route path.

The app-level Pages workspace lets you reopen a Page, jump to its connected Flow, or delete it.

## Page Builder

![The Page Builder editing an A2UI surface in dark mode](../../../../assets/PageBuilder.webp)

The Page Builder hosts the shared `WidgetBuilder` component and adds Page-specific behavior around it:

- a Page switcher when the app has multiple Pages;
- an **Open Flow** button when the Page has a board;
- automatic saving and a visible saving/unsaved/saved state;
- Page settings for behavior, layout, and SEO;
- the global FlowPilot assistant with the active surface as context.

The builder URL accepts:

```text
/page-builder?id=<pageId>&app=<appId>&board=<boardId>
```

`board` is optional in the URL, but a Page created from a Flow stores its board association and uses it to discover available Simple Event nodes.

## Current Page Model

The public state interface stores the following core fields:

```typescript
interface IPage {
  id: string;
  name: string;
  route?: string;
  content: PageContent[];
  layoutType: "Freeform" | "Stack" | "Grid" | "Sidebar" | "HolyGrail";
  components: SurfaceComponent[];

  boardId?: string;
  canvasSettings?: {
    backgroundColor?: string;
    backgroundImage?: string;
    padding?: string;
    customCss?: string;
  };

  onLoadEventId?: string;
  onUnloadEventId?: string;
  onIntervalEventId?: string;
  onIntervalSeconds?: number;
  cache?: boolean;

  title?: string;
  meta?: PageMeta;
  widgetRefs?: Record<string, IWidgetRef>;
  version?: [number, number, number];
  createdAt: string;
  updatedAt: string;
}
```

`components` is the editable A2UI surface. `widgetRefs` stores the Widget definitions needed by Widget instances on that Page.

:::note[Route source of truth]
`IPage` includes an optional `route` field, and `createPage` still accepts a route argument. Current app navigation resolves through `routeState`, where a path maps to an Event ID. Treat the route-state mapping as the navigation source of truth.
:::

## Page Settings

### General

Edit the Page name and description, inspect the immutable Page ID and current version, and save metadata explicitly.

### Behavior

A Page can invoke Simple Event nodes from its connected Flow:

| Hook | When it runs |
| --- | --- |
| **On Page Load** | After the Page is loaded |
| **On Page Unload** | When the user navigates away |
| **On Interval** | Repeatedly at a positive interval in seconds |

If an On Load event is configured, **Cache Page** can show the last rendered state immediately while the load event refreshes it.

The settings panel only lists `events_simple` nodes from the connected board. If the list is empty, add a Simple Event node to that Flow.

### Layout

The current layout choices are:

| Type | Intended use |
| --- | --- |
| `Freeform` | Position elements freely |
| `Stack` | Build a vertical composition |
| `Grid` | Arrange content on a grid |
| `Sidebar` | Combine main content and a sidebar |
| `HolyGrail` | Use the classic header, columns, and footer pattern |

Canvas settings control the background color or image, outer padding, and custom CSS. Component-level responsive styling remains part of the A2UI component graph.

### SEO

Set the Page title, meta description, favicon, and theme color. These values describe the Page; they do not configure its app route.

## State API

```typescript
interface IPageState {
  getPages(appId: string, boardId?: string): Promise<PageListItem[]>;
  getPage(appId: string, pageId: string, boardId?: string): Promise<IPage>;
  createPage(
    appId: string,
    pageId: string,
    name: string,
    route: string,
    boardId: string,
    title?: string,
  ): Promise<IPage>;
  updatePage(appId: string, page: IPage): Promise<void>;
  deletePage(appId: string, pageId: string, boardId: string): Promise<void>;
}
```

Although `createPage` takes a `route` argument, exposing a Page to users requires an Event and a `path → eventId` route mapping.

## Saving and Previewing

Component and Widget-reference changes are saved after a short debounce. Page metadata and canvas settings use their own shorter debounce, and the settings panel also offers an explicit metadata save.

Use **Preview** to test the surface at Desktop, Laptop, Tablet, Mobile, or Mobile Small dimensions. Test lifecycle events against representative data before publishing the app.

## Continue

- [Visual Builder](/dev/a2ui/visual-builder/) — learn every part of the shared editor
- [Widgets](/dev/a2ui/widgets/) — create reusable blocks for Pages
- [Routes](/dev/a2ui/routes/) — expose a Page through an Event
