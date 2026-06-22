---
name: flow-like-ui
description: Generate valid A2UI JSON for FlowLike application frontends. Use when asked to create, design, or build user interfaces, dashboards, forms, pages, layouts, or any visual UI components as A2UI JSON. Supports 60+ component types including layout (row, column, grid), display (text, image, charts), interactive (button, textField, select), containers (card, modal, tabs), game (canvas2d, scene3d, sprite), and geo (geoMap).
---

# A2UI Frontend Generator

Convert UI descriptions into valid A2UI (Agent-to-UI) JSON that renders directly in the FlowLike runtime. A2UI is a declarative, flat-list protocol — components reference children by ID, not by nesting.

## Response Format

Always respond with ONLY a JSON code block. No explanatory text before or after.

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

## Absolute Rules

1. **JSON only** — No explanations, just the JSON code block
2. **Exactly one root component named root** — `rootComponentId` MUST be exactly `"root"`, and `components[]` MUST contain exactly one component with `"id": "root"`
3. **Root wraps the whole UI** — Put every top-level section inside the root component's `children`
4. **Flat component list** — All components are siblings in `components[]`; hierarchy is expressed via children references, NEVER by nesting
5. **Unique IDs** — Every non-root component gets a unique kebab-case ID (`header-row`, `submit-btn`)
6. **BoundValue wrapper** — ALL prop values MUST use BoundValue format unless the field is structural
7. **Reference children by ID** — Use `{"explicitList": ["id1", "id2"]}`
8. **Prefer theme tokens** — Use `bg-background`, `text-foreground`, etc. Hardcoded colors only if user requests them
9. **Include dataModel** — When using data binding paths, always provide a `dataModel` array with initial values

Structural fields that are not BoundValues: `id`, `style.className`, `component.type`, `children`, `actions`, `canvasSettings`, `dataModel`, and raw objects inside option/data arrays.

## BoundValue Format

Every component property value MUST be wrapped:

| Type | Format |
|------|--------|
| String | `{"literalString": "text"}` |
| Number | `{"literalNumber": 42}` |
| Boolean | `{"literalBool": true}` |
| Options | `{"literalOptions": [{"value": "v", "label": "L"}]}` |
| JSON | `{"literalJson": "..."}` |
| Data Binding | `{"path": "$.data.field", "defaultValue": "fallback"}` |

For full data binding patterns, see [references/bound-value-guide.md](references/bound-value-guide.md).

## Children Format

Static children:
```json
"children": {"explicitList": ["child-id-1", "child-id-2"]}
```

Data-driven repeated children (templating):
```json
"children": {"template": {"dataPath": "$.items", "itemIdPath": "id", "templateComponentId": "item-template"}}
```

## Theme Variables

**Backgrounds:** `bg-background`, `bg-muted`, `bg-card`, `bg-primary`, `bg-secondary`, `bg-accent`, `bg-destructive`
**Text:** `text-foreground`, `text-muted-foreground`, `text-primary-foreground`, `text-secondary-foreground`, `text-destructive`
**Borders:** `border-border`, `border-input`, `ring-ring`

For full styling guide with Tailwind utilities and responsive design, see [references/styling-guide.md](references/styling-guide.md).

## Responsive Breakpoints (Mobile-First)

Base = mobile, `sm:` >=640px, `md:` >=768px, `lg:` >=1024px, `xl:` >=1280px, `2xl:` >=1536px

## UI Quality Checklist

- Build mobile-first. Base classes must work on phones, then use `sm:`, `md:`, `lg:`, `xl:`, and `2xl:` for larger screens.
- Prevent horizontal overflow with `w-full`, `max-w-*`, `min-w-0`, `overflow-hidden`, `break-words`, and responsive grid columns.
- Prefer responsive grids: `grid-cols-1 sm:grid-cols-2 lg:grid-cols-3`, or `repeat(auto-fit, minmax(220px, 1fr))` for card lists.
- Keep touch targets comfortable. Buttons, links, and inputs should usually have at least `h-10`, `px-3`, or generous padding.
- Use clean hierarchy: root wrapper, page sections, rows/grids, cards or controls, then content.
- Avoid fixed desktop-only widths. Use `max-w-* mx-auto` for centered layouts and `w-full` for forms, charts, images, tables, and maps.
- Give charts, media, maps, and 3D scenes stable dimensions with `aspectRatio`, `min-h-*`, or height props.
- Make dashboards scannable with stat cards first, charts/tables below, clear labels, muted helper text, and consistent spacing.
- Prefer icons for common actions when appropriate, but keep text labels for primary actions and navigation.
- Use theme tokens so the UI remains readable in light and dark mode.

## Custom CSS

For effects beyond Tailwind, use `canvasSettings.customCss`:

```json
"canvasSettings": {
  "customCss": ".glow { animation: pulse 2s infinite; }"
}
```

## Actions

Interactive components can fire actions back to the agent/backend:

```json
"actions": [{"name": "submit", "context": {"formId": "contact-form"}}]
```

---

## Available Components

For complete prop documentation, see [references/components-reference.md](references/components-reference.md).

### Layout
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `column` | Vertical flex container | gap, align, justify, wrap, children |
| `row` | Horizontal flex container | gap, align, justify, wrap, children |
| `grid` | CSS Grid container | columns, rows, gap, autoFlow, children |
| `stack` | Z-axis layering (**MUST set width/height**) | align, width, height, children |
| `scrollArea` | Scrollable container | direction, children |
| `aspectRatio` | Maintain ratio | ratio* (required), children |
| `overlay` | Items over a base | baseComponentId, overlays |
| `absolute` | Free positioning | width, height, children |
| `box` | Semantic HTML container | as (div/section/header/etc.), children |
| `center` | Center content | inline, children |
| `spacer` | Flexible/fixed space | size, flex |

### Display
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `text` | Typography | content*, variant, size, weight, color, align |
| `image` | Image display | src*, alt, fit, loading, aspectRatio |
| `icon` | Lucide icons | name*, size, color, strokeWidth |
| `video` | Video player | src*, poster, autoplay, loop, muted, controls |
| `lottie` | Lottie animations | src*, autoplay, loop, speed |
| `markdown` | Rendered markdown | content*, allowHtml |
| `badge` | Small label/tag | content*, variant |
| `avatar` | User avatar | src, fallback, size |
| `userProfile` | Flow-Like user lookup | value*, variant, avatarSize, showHover |
| `progress` | Progress bar | value*, max, showLabel, variant |
| `spinner` | Loading spinner | size, color |
| `skeleton` | Loading placeholder | width, height, rounded |
| `divider` | Separator line | orientation, thickness |
| `iframe` | Embedded content / HTML preview | src, srcdoc, title, width, height, sandbox |
| `table` | Data table | columns*, data*, striped, searchable, paginated |
| `tableRow` | Manual table row | cells*, selected, disabled |
| `tableCell` | Manual table cell | content*, isHeader, colSpan, rowSpan |
| `plotlyChart` | Plotly.js charts | chartType, title, data, layout |
| `nivoChart` | Nivo charts (25+ types) | chartType*, data, indexBy, keys |
| `filePreview` | File preview | src/url, mimeType, fileType |
| `boundingBoxOverlay` | Boxes on image | src*, boxes*, showLabels |
| `geoMap` | Interactive map | center, zoom, markers, mapStyle |

### Interactive
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `button` | Clickable button | label*, variant, size, icon, disabled, loading |
| `feedback` | User feedback control | mode, title, showComment, feedbackId |
| `appLink` | Link into app shell | target, label, variant, icon |
| `textField` | Text input | value*, label, placeholder, inputType, multiline |
| `select` | Dropdown | value*, options*, label, multiple, searchable |
| `slider` | Range slider | value*, min, max, step, label |
| `checkbox` | Boolean toggle | checked*, label, disabled |
| `switch` | Toggle switch | checked*, label, disabled |
| `radioGroup` | Radio buttons | value*, options*, orientation, label |
| `dateTimeInput` | Date/time picker | value*, mode, label |
| `fileInput` | File upload | value, label, accept, multiple |
| `imageInput` | Image upload | value, label, showPreview |
| `voiceInput` | Voice recording input | value*, label, maxDuration, visualizer |
| `link` | Navigation link | href*, label, variant, external |
| `imageLabeler` | Draw boxes on image | src*, labels*, boxes |
| `imageHotspot` | Clickable hotspots | src*, hotspots*, markerStyle |

### Container
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `card` | Content container | title, description, variant, children |
| `modal` | Dialog overlay | open*, title, size, children |
| `tabs` | Tabbed content | value*, tabs (array), variant |
| `accordion` | Collapsible sections | items (array), multiple |
| `drawer` | Slide-out panel | open*, side, title, children |
| `tooltip` | Hover tooltip | content*, side, children |
| `popover` | Click popover | contentComponentId*, side, trigger |

### Game / Visual
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `canvas2d` | 2D drawing canvas | width*, height*, backgroundColor, children |
| `sprite` | 2D sprite | src*, x*, y*, rotation, scale, flipX, flipY |
| `shape` | 2D vector shape | shapeType*, x*, y*, fill, stroke, strokeWidth |
| `scene3d` | 3D scene (Three.js) | width*, height*, cameraType, controlMode |
| `model3d` | 3D model viewer | src*, lightingPreset, environment, autoRotate |
| `dialogue` | Visual novel dialogue | text*, speakerName, typewriter, portrait |
| `characterPortrait` | Character portrait | image*, position, expression, flip |
| `choiceMenu` | Interactive choices | choices*, title, layout, columns |
| `inventoryGrid` | Item grid | items*, columns, rows, cellSize |
| `healthBar` | HP/resource bar | value*, maxValue*, variant, fillColor |
| `miniMap` | Game mini-map | mapImage*, width*, height*, markers, playerX, playerY |

### Special
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `widgetInstance` | Embed reusable widget | widgetId, widgetInputs, bindOutputs |

*= required prop

---

## Quick Examples

### Login Form

User: "Login form with email, password, and submit button"

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
        "className": "min-h-screen w-full bg-background text-foreground p-4"
      },
      "component": {
        "type": "center",
        "children": {
          "explicitList": [
            "login-card"
          ]
        }
      }
    },
    {
      "id": "login-card",
      "style": {
        "className": "w-full max-w-sm mx-auto"
      },
      "component": {
        "type": "card",
        "title": {
          "literalString": "Welcome Back"
        },
        "description": {
          "literalString": "Enter your credentials to sign in"
        },
        "children": {
          "explicitList": [
            "login-form"
          ]
        }
      }
    },
    {
      "id": "login-form",
      "component": {
        "type": "column",
        "gap": {
          "literalString": "1rem"
        },
        "children": {
          "explicitList": [
            "email-field",
            "password-field",
            "submit-btn"
          ]
        }
      }
    },
    {
      "id": "email-field",
      "component": {
        "type": "textField",
        "value": {
          "path": "$.form.email",
          "defaultValue": ""
        },
        "label": {
          "literalString": "Email"
        },
        "placeholder": {
          "literalString": "you@example.com"
        },
        "inputType": {
          "literalString": "email"
        },
        "required": {
          "literalBool": true
        }
      }
    },
    {
      "id": "password-field",
      "component": {
        "type": "textField",
        "value": {
          "path": "$.form.password",
          "defaultValue": ""
        },
        "label": {
          "literalString": "Password"
        },
        "placeholder": {
          "literalString": "********"
        },
        "inputType": {
          "literalString": "password"
        },
        "required": {
          "literalBool": true
        }
      }
    },
    {
      "id": "submit-btn",
      "style": {
        "className": "w-full"
      },
      "component": {
        "type": "button",
        "label": {
          "literalString": "Sign In"
        },
        "variant": {
          "literalString": "default"
        },
        "actions": [
          {
            "name": "login",
            "context": {
              "form": "login"
            }
          }
        ]
      }
    }
  ],
  "dataModel": [
    {
      "path": "$.form.email",
      "value": ""
    },
    {
      "path": "$.form.password",
      "value": ""
    }
  ]
}
```

### Stats Dashboard

User: "Dashboard with 3 stat cards and a chart"

```json
{
  "rootComponentId": "root",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1.5rem"
  },
  "components": [
    {
      "id": "root",
      "style": {
        "className": "min-h-screen w-full bg-background text-foreground"
      },
      "component": {
        "type": "column",
        "gap": {
          "literalString": "1.5rem"
        },
        "children": {
          "explicitList": [
            "stats-grid",
            "chart-card"
          ]
        }
      }
    },
    {
      "id": "stats-grid",
      "component": {
        "type": "grid",
        "columns": {
          "literalString": "repeat(auto-fit, minmax(200px, 1fr))"
        },
        "gap": {
          "literalString": "1rem"
        },
        "children": {
          "explicitList": [
            "stat-users",
            "stat-revenue",
            "stat-orders"
          ]
        }
      }
    },
    {
      "id": "stat-users",
      "component": {
        "type": "card",
        "padding": {
          "literalString": "1.5rem"
        },
        "children": {
          "explicitList": [
            "stat-users-content"
          ]
        }
      }
    },
    {
      "id": "stat-users-content",
      "component": {
        "type": "column",
        "gap": {
          "literalString": "0.5rem"
        },
        "children": {
          "explicitList": [
            "stat-users-label",
            "stat-users-value"
          ]
        }
      }
    },
    {
      "id": "stat-users-label",
      "component": {
        "type": "text",
        "content": {
          "literalString": "Total Users"
        },
        "size": {
          "literalString": "sm"
        },
        "color": {
          "literalString": "text-muted-foreground"
        }
      }
    },
    {
      "id": "stat-users-value",
      "component": {
        "type": "text",
        "content": {
          "path": "$.stats.users",
          "defaultValue": "0"
        },
        "variant": {
          "literalString": "h3"
        },
        "weight": {
          "literalString": "bold"
        }
      }
    },
    {
      "id": "stat-revenue",
      "component": {
        "type": "card",
        "padding": {
          "literalString": "1.5rem"
        },
        "children": {
          "explicitList": [
            "stat-revenue-content"
          ]
        }
      }
    },
    {
      "id": "stat-revenue-content",
      "component": {
        "type": "column",
        "gap": {
          "literalString": "0.5rem"
        },
        "children": {
          "explicitList": [
            "stat-revenue-label",
            "stat-revenue-value"
          ]
        }
      }
    },
    {
      "id": "stat-revenue-label",
      "component": {
        "type": "text",
        "content": {
          "literalString": "Revenue"
        },
        "size": {
          "literalString": "sm"
        },
        "color": {
          "literalString": "text-muted-foreground"
        }
      }
    },
    {
      "id": "stat-revenue-value",
      "component": {
        "type": "text",
        "content": {
          "path": "$.stats.revenue",
          "defaultValue": "$0"
        },
        "variant": {
          "literalString": "h3"
        },
        "weight": {
          "literalString": "bold"
        }
      }
    },
    {
      "id": "stat-orders",
      "component": {
        "type": "card",
        "padding": {
          "literalString": "1.5rem"
        },
        "children": {
          "explicitList": [
            "stat-orders-content"
          ]
        }
      }
    },
    {
      "id": "stat-orders-content",
      "component": {
        "type": "column",
        "gap": {
          "literalString": "0.5rem"
        },
        "children": {
          "explicitList": [
            "stat-orders-label",
            "stat-orders-value"
          ]
        }
      }
    },
    {
      "id": "stat-orders-label",
      "component": {
        "type": "text",
        "content": {
          "literalString": "Orders"
        },
        "size": {
          "literalString": "sm"
        },
        "color": {
          "literalString": "text-muted-foreground"
        }
      }
    },
    {
      "id": "stat-orders-value",
      "component": {
        "type": "text",
        "content": {
          "path": "$.stats.orders",
          "defaultValue": "0"
        },
        "variant": {
          "literalString": "h3"
        },
        "weight": {
          "literalString": "bold"
        }
      }
    },
    {
      "id": "chart-card",
      "component": {
        "type": "card",
        "title": {
          "literalString": "Revenue Over Time"
        },
        "children": {
          "explicitList": [
            "revenue-chart"
          ]
        }
      }
    },
    {
      "id": "revenue-chart",
      "component": {
        "type": "nivoChart",
        "chartType": {
          "literalString": "bar"
        },
        "data": {
          "path": "$.charts.revenue"
        },
        "height": {
          "literalString": "300px"
        },
        "animate": {
          "literalBool": true
        }
      }
    }
  ],
  "dataModel": [
    {
      "path": "$.stats.users",
      "value": "12,458"
    },
    {
      "path": "$.stats.revenue",
      "value": "$48,200"
    },
    {
      "path": "$.stats.orders",
      "value": "384"
    },
    {
      "path": "$.charts.revenue",
      "value": [
        {
          "month": "Jan",
          "revenue": 4200
        },
        {
          "month": "Feb",
          "revenue": 5800
        },
        {
          "month": "Mar",
          "revenue": 4900
        }
      ]
    }
  ]
}
```

## References

For detailed documentation, consult these files as needed:

- **[references/components-reference.md](references/components-reference.md)** -- Complete component props with all accepted values
- **[references/bound-value-guide.md](references/bound-value-guide.md)** -- Data binding patterns, JSONPath syntax, form input binding
- **[references/styling-guide.md](references/styling-guide.md)** -- Tailwind CSS utilities, theme variables, responsive patterns, custom CSS
- **[references/layout-examples.md](references/layout-examples.md)** -- Full JSON examples: page layouts, forms, dashboards, cards, tables

Generate A2UI JSON for any UI request. Output ONLY valid JSON.
