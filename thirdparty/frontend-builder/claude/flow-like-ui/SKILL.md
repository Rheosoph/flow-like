---
name: flow-like-ui
description: Generate valid A2UI JSON for FlowLike application frontends. Use when asked to create, design, or build user interfaces, dashboards, forms, pages, layouts, or any visual UI components as A2UI JSON. Supports 60+ component types including layout (row, column, grid), display (text, image, charts), interactive (button, textField, select), containers (card, modal, tabs), game (canvas2d, scene3d, model3d), and geo (geoMap).
---

# A2UI Frontend Generator

Convert UI descriptions into valid A2UI JSON that renders directly in the FlowLike runtime.

## Response Format

Always respond with ONLY a JSON code block. No explanatory text before or after.

```json
{
  "rootComponentId": "root-id",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1rem"
  },
  "components": [
    {
      "id": "unique-id",
      "style": { "className": "tailwind-classes" },
      "component": { "type": "componentType", ...props }
    }
  ]
}
```

## Absolute Rules

1. **JSON only** — No explanations, just the JSON code block
2. **Unique IDs** — Every component gets a unique kebab-case ID (`header-row`, `submit-btn`)
3. **BoundValue wrapper** — ALL prop values use BoundValue format
4. **Reference children by ID** — Use `{"explicitList": ["id1", "id2"]}`
5. **Prefer theme tokens** — Use `bg-background`, `text-foreground`, etc. Hardcoded colors only if user requests them
6. **Root component required** — `rootComponentId` must reference an existing component ID
7. **Flat component list** — All components are siblings in `components`; hierarchy is via children references

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

```json
"children": {"explicitList": ["child-id-1", "child-id-2"]}
```

For data-driven repeated children:

```json
"children": {"template": {"dataPath": "$.items", "itemIdPath": "id", "templateComponentId": "item-template"}}
```

## Theme Variables

**Backgrounds:** `bg-background`, `bg-muted`, `bg-card`, `bg-primary`, `bg-secondary`, `bg-accent`, `bg-destructive`
**Text:** `text-foreground`, `text-muted-foreground`, `text-primary-foreground`, `text-secondary-foreground`, `text-destructive`
**Borders:** `border-border`, `border-input`, `ring-ring`

For full styling guide with Tailwind utilities and responsive design, see [references/styling-guide.md](references/styling-guide.md).

## Responsive Breakpoints (Mobile-First)

Base = mobile, `sm:` ≥640px, `md:` ≥768px, `lg:` ≥1024px, `xl:` ≥1280px, `2xl:` ≥1536px

## Custom CSS

For effects beyond Tailwind, use `canvasSettings.customCss`:

```json
"canvasSettings": {
  "customCss": ".glow { animation: pulse 2s infinite; }"
}
```

## Actions

Interactive components can fire actions:

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
| `stack` | Z-axis layering | align, width, height, children |
| `scrollArea` | Scrollable container | direction, children |
| `aspectRatio` | Maintain ratio | ratio (required), children |
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
| `progress` | Progress bar | value*, max, showLabel, variant |
| `spinner` | Loading spinner | size, color |
| `skeleton` | Loading placeholder | width, height, rounded |
| `divider` | Separator line | orientation, thickness |
| `iframe` | Embedded content / HTML preview | src, srcdoc, title, width, height, sandbox, allow, referrerPolicy, border |
| `table` | Data table | columns*, data*, striped, searchable, paginated |
| `plotlyChart` | Plotly.js charts | chartType, title, series, data |
| `nivoChart` | Nivo charts (25+ types) | chartType*, data, indexBy, keys |
| `filePreview` | File preview | src*, fit |
| `boundingBoxOverlay` | Boxes on image | src*, boxes*, showLabels |

### Interactive
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `button` | Clickable button | label*, variant, size, icon, disabled, loading |
| `textField` | Text input | value*, label, placeholder, inputType, multiline |
| `select` | Dropdown | value*, options*, label, multiple, searchable |
| `slider` | Range slider | value*, min, max, step, label |
| `checkbox` | Boolean toggle | checked*, label, disabled |
| `switch` | Toggle switch | checked*, label, disabled |
| `radioGroup` | Radio buttons | value*, options*, orientation, label |
| `dateTimeInput` | Date/time picker | value*, mode, label |
| `fileInput` | File upload | value, label, accept, multiple |
| `imageInput` | Image upload | value, label, showPreview |
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

### Game
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `canvas2d` | 2D canvas | width*, height*, children |
| `sprite` | 2D sprite | src*, x*, y*, rotation, scale |
| `shape` | 2D shape | shapeType*, x*, y*, fill, stroke |
| `scene3d` | 3D scene (Three.js) | width*, height*, cameraType, controlMode |
| `model3d` | 3D model viewer | src*, lightingPreset, environment, autoRotate |
| `dialogue` | VN dialogue box | text*, speakerName, typewriter |
| `characterPortrait` | Character portrait | image*, position, size |
| `choiceMenu` | Choice selection | choices*, title, layout |
| `inventoryGrid` | Inventory display | items*, columns, rows |
| `healthBar` | HP/resource bar | value*, maxValue*, variant |
| `miniMap` | Game mini-map | width*, height*, markers, playerX, playerY |

### Geo
| Component | Purpose | Key Props |
|-----------|---------|-----------|
| `geoMap` | Interactive map | viewport, markers, routes, showControls |

*= required prop

---

## Quick Example

User: "Login form with email, password, and submit button"

```json
{
  "rootComponentId": "login-card",
  "canvasSettings": {"backgroundColor": "bg-background", "padding": "1rem"},
  "components": [
    {
      "id": "login-card",
      "style": {"className": "w-full max-w-sm mx-auto"},
      "component": {
        "type": "card",
        "title": {"literalString": "Welcome Back"},
        "description": {"literalString": "Enter your credentials to sign in"},
        "children": {"explicitList": ["login-form"]}
      }
    },
    {
      "id": "login-form",
      "style": {"className": ""},
      "component": {
        "type": "column",
        "gap": {"literalString": "1rem"},
        "children": {"explicitList": ["email-field", "password-field", "submit-btn"]}
      }
    },
    {
      "id": "email-field",
      "style": {"className": ""},
      "component": {
        "type": "textField",
        "value": {"literalString": ""},
        "label": {"literalString": "Email"},
        "placeholder": {"literalString": "you@example.com"},
        "inputType": {"literalString": "email"},
        "required": {"literalBool": true}
      }
    },
    {
      "id": "password-field",
      "style": {"className": ""},
      "component": {
        "type": "textField",
        "value": {"literalString": ""},
        "label": {"literalString": "Password"},
        "placeholder": {"literalString": "••••••••"},
        "inputType": {"literalString": "password"},
        "required": {"literalBool": true}
      }
    },
    {
      "id": "submit-btn",
      "style": {"className": "w-full"},
      "component": {
        "type": "button",
        "label": {"literalString": "Sign In"},
        "variant": {"literalString": "default"}
      }
    }
  ]
}
```

## References

For detailed documentation, consult these files as needed:

- **[references/components-reference.md](references/components-reference.md)** — Complete component props with all accepted values
- **[references/bound-value-guide.md](references/bound-value-guide.md)** — Data binding patterns, JSONPath syntax, form input binding
- **[references/styling-guide.md](references/styling-guide.md)** — Tailwind CSS utilities, theme variables, responsive patterns, custom CSS
- **[references/layout-examples.md](references/layout-examples.md)** — Full JSON examples: page layouts, forms, dashboards, cards, tables

Generate A2UI JSON for any UI request. Output ONLY valid JSON.
