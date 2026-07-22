# BoundValue and data binding

A2UI wraps component prop values so the same prop can be static or resolved from surface data. Structural and raw schema fields are the exceptions listed in `SKILL.md` and the component reference.

Contents: [literal values](#literal-values), [data paths](#data-paths), [initial data](#initial-datamodel), [forms](#two-way-form-binding), [structured data](#static-structured-data), [repeated children](#repeated-children-and-scoped-paths), and [conditional visibility](#conditional-visibility).

## Literal values

### String

```json
{ "literalString": "Hello world" }
```

### Number

```json
{ "literalNumber": 42 }
```

### Boolean

```json
{ "literalBool": true }
```

### Select and radio options

Use `literalOptions` only for `{value, label}` option arrays:

```json
{
  "literalOptions": [
    { "value": "draft", "label": "Draft" },
    { "value": "published", "label": "Published" }
  ]
}
```

### JSON arrays and objects

Use `literalJson` for structured component props such as table rows/columns, chart data/config, map data, calendar events, Gantt tasks, 3D vectors, or image boxes. The value is a JSON-encoded string.

```json
{
  "literalJson": "[{\"id\":\"a\",\"value\":42}]"
}
```

Do not use `literalOptions` for arbitrary objects or arrays of strings.

## Data paths

Bind a prop to the surface data model with `path`:

```json
{
  "path": "$.user.displayName",
  "defaultValue": "Guest"
}
```

- Start main-surface paths with `$.`.
- Use dot-separated object keys: `$.user.email`.
- Use numeric array indices: `$.items[0].name` or `$.items.0.name`.
- The runtime supports direct path lookup, not general JSONPath evaluation. Do not use wildcards, filters, slices, recursive descent, or expressions.
- Add `defaultValue` when a missing path would otherwise leave an incomplete UI.

## Initial dataModel

When using main-surface `$.` paths, provide matching initial values:

```json
{
  "rootComponentId": "root",
  "components": [
    {
      "id": "root",
      "component": {
        "type": "column",
        "children": { "explicitList": ["user-name"] }
      }
    },
    {
      "id": "user-name",
      "component": {
        "type": "text",
        "content": { "path": "$.user.name", "defaultValue": "Guest" }
      }
    }
  ],
  "dataModel": [
    { "path": "$.user.name", "value": "Jane Doe" },
    {
      "path": "$.items",
      "value": [
        { "id": "a", "name": "Item A" },
        { "id": "b", "name": "Item B" }
      ]
    }
  ]
}
```

Each `dataModel` entry has a raw `path` and raw JSON `value`. Initialize a whole object at `$.user` or individual leaves such as `$.user.name`; do not do both for the same data unless intentional.

## Two-way form binding

Input components write back when their value prop is a `path` binding.

```json
{
  "id": "email-input",
  "eventRelevant": true,
  "component": {
    "type": "textField",
    "value": { "path": "$.form.email", "defaultValue": "" },
    "label": { "literalString": "Email" },
    "placeholder": { "literalString": "you@example.com" },
    "inputType": { "literalString": "email" },
    "required": { "literalBool": true }
  }
}
```

Use the writable prop for each control:

- `textField`, `select`, `slider`, `dateTimeInput`, `fileInput`, `imageInput`, and `voiceInput`: `value`
- `checkbox` and `switch`: `checked`
- `radioGroup` and `tabs`: `value`
- `modal` and `drawer`: `open`

Use `eventRelevant: true` on the outer wrapper when a workflow action should include that element in the input-value collection.

## Static structured data

### Table

```json
{
  "id": "users-table",
  "component": {
    "type": "table",
    "columns": {
      "literalJson": "[{\"id\":\"name\",\"header\":{\"literalString\":\"Name\"},\"accessor\":{\"literalString\":\"name\"}},{\"id\":\"email\",\"header\":{\"literalString\":\"Email\"},\"accessor\":{\"literalString\":\"email\"}}]"
    },
    "data": {
      "literalJson": "[{\"name\":\"Jane\",\"email\":\"jane@example.com\"}]"
    },
    "sortable": { "literalBool": true }
  }
}
```

### Nivo bar chart

```json
{
  "id": "sales-chart",
  "component": {
    "type": "nivoChart",
    "chartType": { "literalString": "bar" },
    "data": {
      "literalJson": "[{\"month\":\"Jan\",\"revenue\":100,\"profit\":20},{\"month\":\"Feb\",\"revenue\":150,\"profit\":35}]"
    },
    "indexBy": { "literalString": "month" },
    "keys": { "literalJson": "[\"revenue\",\"profit\"]" },
    "height": { "literalString": "320px" }
  }
}
```

For dynamic table/chart data, replace `literalJson` with a `path` and initialize that path in `dataModel`.

## Repeated children and scoped paths

A template repeats one existing component definition for every item in an array:

```json
{
  "id": "project-grid",
  "component": {
    "type": "grid",
    "children": {
      "template": {
        "dataPath": "$.projects",
        "itemIdPath": "id",
        "templateComponentId": "project-card-template"
      }
    }
  }
}
```

Inside the template component and its descendants:

- `$item` resolves to the current item.
- `$item.name` resolves a field on the current item.
- `$index` is replaced with the current numeric index.
- A writable `$item.field` path writes back to the corresponding item under the template's `dataPath`.
- `itemIdPath` is relative to the current item, for example `id` or `metadata.id`.

```json
{
  "id": "project-title",
  "component": {
    "type": "text",
    "content": { "path": "$item.name", "defaultValue": "Untitled project" }
  }
}
```

## Conditional visibility

Bind `hidden` to a boolean path. Do not put template expressions in `style.className`; class names are plain strings.

```json
{
  "id": "premium-badge",
  "component": {
    "type": "badge",
    "content": { "literalString": "Premium" },
    "hidden": { "path": "$.user.hidePremiumBadge", "defaultValue": true }
  }
}
```

## Actions and live values

Never copy a bound form value into `actions[].context`. Action context is static identity/routing data. At runtime, the handler reads current input values from the surface; a configured workflow can also fetch the target elements directly.

## Checklist

- Use the wrapper that matches the value type.
- Use `literalJson` for arbitrary arrays/objects and `literalOptions` only for `{value,label}` options.
- Keep data paths simple and direct.
- Initialize every important main-surface path or supply a useful fallback.
- Use `$item` / `$index` only inside a repeated template scope.
- Put `eventRelevant` on the outer surface-component wrapper.
- Keep raw and structural fields unwrapped exactly as documented in `SKILL.md`.
