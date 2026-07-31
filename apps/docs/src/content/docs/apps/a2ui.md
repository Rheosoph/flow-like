---
title: Custom UI (A2UI)
description: Build rich user interfaces with AI or by hand
sidebar:
  order: 46
---

Flow-Like uses [A2UI (Agent-to-User Interface)](https://a2ui.org/) to describe
rich interfaces as structured data. The same component model powers visual
Pages, reusable Widgets, and interfaces returned by a Flow.

Because the interface is data rather than hard-coded application markup, you
can build it visually, edit its JSON, or ask FlowPilot to help generate and
refine it.

## Where A2UI appears

### Pages

A [Page](/apps/pages/) is a full interface owned by a Flow. It combines an A2UI
component tree with Page-level settings such as layout, lifecycle events, and
SEO metadata. A UI Event gives the Page a navigable [route](/apps/routes/).

![A support operations Page rendered in Flow-Like's visual Page Builder](../../../assets/PageBuilder.webp)

### Widgets

A [Widget](/apps/widgets/) is a reusable A2UI component tree owned by an app.
It can expose selected component properties for each instance and define named
events that the containing workflow can handle.

![A reusable support health card in Flow-Like's visual Widget Builder](../../../assets/WidgetBuilder.webp)

### Flow-generated interfaces

A Flow can return A2UI updates at runtime. This is useful when the structure or
content must respond to execution data, user input, or an AI-generated result.
The renderer applies those updates to the active surface.

## Three ways to create an interface

### Visual Builder

The Page and Widget builders provide a component catalog, hierarchy, live
canvas, and property Inspector. Use them to compose a surface without writing
the component payload by hand.

### FlowPilot

FlowPilot can work with the active builder surface. Describe the interface or
the change you want, review the result on the canvas, and continue refining it.
Specific components also offer **Optimize with FlowPilot** from the builder.

### Dev Mode

Select **Dev Mode** in the builder toolbar to inspect or edit the underlying
JSON. This is useful for precise changes, generated payloads, and debugging.
Return to the visual canvas to validate the result.

All three approaches modify the same component model, so you can move between
them during one editing session.

## Core structure

An A2UI surface has one root component and a collection of components
referenced by ID:

```json
{
  "rootComponentId": "root",
  "components": [
    {
      "id": "root",
      "component": {
        "type": "column",
        "children": {
          "explicitList": ["heading", "content"]
        }
      }
    },
    {
      "id": "heading",
      "component": {
        "type": "text",
        "content": {
          "literalString": "Support overview"
        }
      }
    },
    {
      "id": "content",
      "component": {
        "type": "text",
        "content": {
          "literalString": "24 requests are open."
        }
      }
    }
  ]
}
```

The root is the only top-level component. Layout components reference their
children by ID, which makes the hierarchy explicit and lets the builder move
or replace individual sections safely.

## Values, data, and actions

Component properties can use a literal value or a binding to surface data. A
literal is appropriate for fixed labels and presentation. A binding is
appropriate for values supplied or updated by the Flow.

Interactive components can trigger a Page workflow with a `workflow_event`
action that identifies the selected Event by `nodeId`. Widget components use
`widget_event` plus an `actionId` that the containing instance binds to a
workflow. Navigation uses the separate `navigate_page` or `external_link`
action. A handler can update data or return new A2UI content after it runs.

For the available component types and their exact properties, use the
[A2UI component reference](/reference/a2ui-components/).

## Create and expose a Page

1. Open the Flow that should provide the Page's behavior.
2. Open its **Pages** panel and select **New**.
3. Build the interface visually, with FlowPilot, or in Dev Mode.
4. In the app's **Events** workspace, create or edit a UI Event.
5. Configure that Event to open the Page and assign a unique route.
6. Preview the Page with representative data before sharing the app.

Create reusable component groups in the app's **Widgets** workspace, then add
them to Pages as Widget instances.

## Design for both themes

Prefer semantic theme classes such as `bg-background`, `bg-card`,
`text-foreground`, `text-muted-foreground`, and `border-border`. They adapt to
the active light or dark theme. Use fixed colors only when the color itself
carries meaning, and verify contrast in both modes.

Responsive layouts should start with a single-column or flexible structure,
then add wider-screen grid rules where needed. Give charts, media, maps, and
other visual surfaces an explicit height or aspect ratio so their surrounding
layout remains stable.

:::tip[For developers]
See the [A2UI Developer Guide](/dev/a2ui/overview/) for programmatic surface
updates and the [FlowPilot UI reference](/reference/flowpilot-ui/) for
AI-assisted generation.
:::
