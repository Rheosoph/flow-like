---
name: flow-like-ui
description: Generate complete, valid A2UI JSON for FlowLike frontends. Use when asked to create, design, or build an interface, dashboard, form, page, responsive layout, reusable widget, chart, calendar, Gantt view, media experience, game UI, map, or other visual UI as A2UI JSON. Covers all 71 runtime component types, BoundValue data binding, actions, inline widgets, theme-aware styling, and responsive design.
---

# Generate FlowLike A2UI

Convert a UI request into a complete A2UI surface that can be imported or rendered by FlowLike. A2UI uses a flat component array: parent components reference child IDs instead of nesting component objects.

## Workflow

1. Design the complete surface without asking follow-up questions when sensible defaults suffice.
2. Select only registered components from [references/components-reference.md](references/components-reference.md).
3. Build one flat component array with one main root and valid child references.
4. Use an inline `widgetInstance` for a genuinely reusable or data-repeated element. Keep a page to at most one or two widgets.
5. Add BoundValue wrappers, initial `dataModel` entries, actions, theme classes, and mobile-first layout.
6. Check every ID, required prop, wrapper, child reference, binding path, and structured value before responding.

## Output contract

Respond with only one `json` code block. Put the complete surface in that block; do not split it or add prose.

```json
{
  "rootComponentId": "root",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1rem"
  },
  "components": [
    {
      "id": "root",
      "style": {
        "className": "min-h-screen w-full bg-background text-foreground"
      },
      "component": {
        "type": "column",
        "children": {
          "explicitList": []
        }
      }
    }
  ],
  "dataModel": []
}
```

Enforce these invariants:

- Set `rootComponentId` to exactly `"root"`.
- Include exactly one top-level component with `id: "root"` and make it own every top-level section through `children`.
- Put every component in the top-level `components` array. Never nest component objects inside another component's `children`.
- Give every non-root component a unique, descriptive kebab-case ID.
- Keep all components in one response and stay within 120 top-level components.
- Include `dataModel`, using an empty array when the surface has no initial bound data.

## Component shape

```json
{
  "id": "submit-button",
  "style": {
    "className": "w-full sm:w-auto"
  },
  "component": {
    "type": "button",
    "label": {
      "literalString": "Submit"
    },
    "actions": [
      {
        "name": "submit",
        "context": {
          "formId": "contact-form"
        }
      }
    ]
  }
}
```

Place `id`, `style`, and optional `eventRelevant` on the surface-component wrapper. Place `type`, props, `children`, `actions`, and `hidden` inside `component`.

## BoundValue and raw fields

Wrap component prop values unless the reference explicitly marks a field as raw or structural.

| Value | Wrapper |
|---|---|
| String | `{"literalString":"text"}` |
| Number | `{"literalNumber":42}` |
| Boolean | `{"literalBool":true}` |
| Select/radio options | `{"literalOptions":[{"value":"v","label":"Label"}]}` |
| JSON array/object | `{"literalJson":"[{\"id\":1}]"}` |
| Data binding | `{"path":"$.data.field","defaultValue":"fallback"}` |

Do not wrap these structural or raw values:

- Surface fields: `rootComponentId`, `canvasSettings`, `components`, `dataModel`, wrapper `id`, wrapper `style`, and wrapper `eventRelevant`.
- Component fields: `type`, `children`, and `actions`.
- Raw schema fields: `overlay.baseComponentId`, `overlay.overlays`, `popover.contentComponentId`, `tabs.tabs`, `tabs.listStyle`, `tabs.triggerStyle`, `tabs.contentStyle`, `accordion.items`, `plotlyChart.series`, `plotlyChart.xAxis`, and `plotlyChart.yAxis`.
- Plain `link` fields: `external`, `target`, `variant`, and `underline`.
- `widgetInstance` wiring: `instanceId`, `widgetId`, `appId`, `inlineWidgetDef`, `exposedPropValues`, `actionBindings`, and `styleOverride`.

Within raw arrays, follow the nested schema in the component reference. Nested display values such as a tab label still use BoundValue where specified.

Read [references/bound-value-guide.md](references/bound-value-guide.md) when the surface uses data paths, forms, repeated templates, or structured JSON props.

## Children

Use explicit children for a fixed layout:

```jsonc
"children": {
  "explicitList": ["page-header", "main-content"]
}
```

Use a template to repeat one component or inline widget over an array:

```jsonc
"children": {
  "template": {
    "dataPath": "$.projects",
    "itemIdPath": "id",
    "templateComponentId": "project-card-template"
  }
}
```

Inside the template component and its descendants, bind with `$item` and `$index`, such as `{"path":"$item.name"}`. `itemIdPath` is relative to each item and stabilizes React keys.

## Reusable and repeated widgets

Use `widgetInstance` when a card, row, list item, or other element is reused or data-repeated. Use plain components for a one-off page section.

```json
{
  "id": "project-card-template",
  "component": {
    "type": "widgetInstance",
    "widgetId": "project-card",
    "instanceId": "project-card-template",
    "inlineWidgetDef": {
      "name": "Project Card",
      "rootComponentId": "project-card-root",
      "components": [
        {
          "id": "project-card-root",
          "style": {
            "className": "h-full rounded-lg border border-border bg-card p-4"
          },
          "component": {
            "type": "column",
            "children": {
              "explicitList": ["project-card-title"]
            }
          }
        },
        {
          "id": "project-card-title",
          "component": {
            "type": "text",
            "content": {
              "path": "$item.name",
              "defaultValue": "Project"
            },
            "weight": {
              "literalString": "semibold"
            }
          }
        }
      ],
      "exposedProps": []
    },
    "exposedPropValues": {},
    "actionBindings": {}
  }
}
```

An inline widget has its own flat component tree and its own matching `rootComponentId`. Its internal IDs are local to the widget. Use `exposedProps` only for caller-settable overrides; bind dynamic row/item content to `$item`.

## Actions

Put actions inside the component object:

```jsonc
"actions": [
  {
    "name": "navigate_page",
    "context": {
      "route": "/projects",
      "queryParams": {
        "view": "active"
      }
    }
  }
]
```

- An action invokes an event; it does not carry the current value of a text field, select, checkbox, file input, calendar, or Gantt item.
- Keep `context` to static routing or identity data. The handler reads live element values from the surface.
- Use `workflow_event` only when a real workflow `nodeId` is supplied; optionally include real `boardId` and `appId`. Never invent these IDs.
- Use `navigate_page` with `context.route`, or `external_link` with `context.url`, for built-in navigation.
- Custom action names are forwarded as `userAction` messages.
- Calendar and Gantt components add an `interaction` such as `open`, `create`, `update`, `move`, `resize`, `delete`, `link`, or `reorder` to the configured action context.
- Mark relevant input wrappers with `eventRelevant: true` when a workflow should receive them in its input-value collection.

## Styling and responsiveness

- Prefer `style.className` with Tailwind utilities.
- Always use shadcn theme tokens: `bg-background`, `bg-card`, `bg-muted`, `bg-primary`, `bg-secondary`, `bg-accent`, `bg-destructive`, `text-foreground`, `text-muted-foreground`, `text-primary-foreground`, `text-secondary-foreground`, `text-destructive`, `border-border`, `border-input`, and `ring-ring`.
- Do not use fixed palette colors such as `bg-white`, `text-black`, or `bg-gray-*`; they break theme adaptation.
- Design mobile-first. Base classes must work below 640px, then enhance with `sm:`, `md:`, `lg:`, `xl:`, and `2xl:`.
- Prevent horizontal overflow with `w-full`, `max-w-full`, `min-w-0`, `overflow-hidden`, `break-words`, and responsive grid columns.
- Give charts, calendars, Gantt timelines, maps, iframes, media, canvases, stacks, and 3D scenes stable dimensions.
- Keep controls touch-friendly and visible; use text labels for primary actions.
- Use `canvasSettings.customCss` only for effects that Tailwind cannot express.

Read [references/styling-guide.md](references/styling-guide.md) for the style object, layout patterns, scoped custom CSS, and responsive examples.

## Component catalog

Use exact, case-sensitive type names.

- Layout: `row`, `column`, `stack`, `grid`, `scrollArea`, `aspectRatio`, `overlay`, `absolute`, `box`, `center`, `spacer`
- Display and data: `text`, `image`, `icon`, `video`, `lottie`, `markdown`, `divider`, `badge`, `avatar`, `userProfile`, `progress`, `spinner`, `skeleton`, `table`, `tableRow`, `tableCell`, `iframe`, `filePreview`, `diffView`, `plotlyChart`, `nivoChart`, `boundingBoxOverlay`, `geoMap`, `graph`, `ontologyGraph`, `calendar`, `gantt`
- Interactive: `button`, `feedback`, `appLink`, `textField`, `select`, `slider`, `checkbox`, `switch`, `radioGroup`, `dateTimeInput`, `fileInput`, `imageInput`, `voiceInput`, `link`, `imageLabeler`, `imageHotspot`
- Containers: `card`, `modal`, `tabs`, `accordion`, `drawer`, `tooltip`, `popover`
- Game and 3D: `canvas2d`, `sprite`, `shape`, `scene3d`, `model3d`, `dialogue`, `characterPortrait`, `choiceMenu`, `inventoryGrid`, `healthBar`, `miniMap`
- Widget: `widgetInstance`

Read [references/components-reference.md](references/components-reference.md) before using unfamiliar or complex components. It is the accepted-prop and required-prop authority for this skill.

For full working surfaces, read [references/layout-examples.md](references/layout-examples.md).

## Final check

Before responding, verify:

- The response is one JSON code block and parses as JSON.
- `rootComponentId` and the only main root ID are both `root`.
- Every component type is in the 71-type catalog.
- Every required prop exists and every component prop uses the correct BoundValue or raw shape.
- Every explicit child, template component, tab content, accordion content, popover content, and overlay reference exists in its component scope.
- Every binding uses a supported path and has useful initial data or a fallback.
- Actions contain no invented runtime IDs or copied live input values.
- The layout is usable on mobile and uses theme tokens.
