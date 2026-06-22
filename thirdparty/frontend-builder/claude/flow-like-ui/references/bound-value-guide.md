# BoundValue & Data Binding Guide

This guide explains how to use the BoundValue format and data binding in A2UI components.

---

## BoundValue Basics

**Every component property value MUST be wrapped in a BoundValue object.** This enables both static values and dynamic data binding.

### Static Values

#### String
```json
{ "literalString": "Hello World" }
```

#### Number
```json
{ "literalNumber": 42 }
```

#### Boolean
```json
{ "literalBool": true }
```

#### Options Array (for select, radioGroup)
```json
{
  "literalOptions": [
    { "value": "opt1", "label": "Option 1" },
    { "value": "opt2", "label": "Option 2" },
    { "value": "opt3", "label": "Option 3" }
  ]
}
```

---

## Data Binding

Data binding allows components to display dynamic values from a data source.

### Basic Binding
```json
{
  "path": "$.user.name",
  "defaultValue": "Guest"
}
```

- `path`: JSONPath expression to the data field
- `defaultValue`: Fallback value if path doesn't resolve

### JSONPath Syntax

| Pattern | Description | Example |
|---------|-------------|---------|
| `$.field` | Root level field | `$.title` |
| `$.nested.field` | Nested field | `$.user.email` |
| `$.array[0]` | Array index | `$.items[0]` |
| `$.array[*].field` | All items in array | `$.users[*].name` |

### Common Binding Patterns

#### User Profile Data
```json
{
  "id": "user-name",
  "component": {
    "type": "text",
    "content": { "path": "$.user.displayName", "defaultValue": "Anonymous" }
  }
}
```

#### Image from Data
```json
{
  "id": "profile-pic",
  "component": {
    "type": "avatar",
    "src": { "path": "$.user.avatarUrl" },
    "fallback": { "path": "$.user.initials", "defaultValue": "?" }
  }
}
```

#### List Data in Table
```json
{
  "id": "data-table",
  "component": {
    "type": "table",
    "columns": {
      "literalOptions": [
        { "id": "name", "header": "Name", "accessor": "name" },
        { "id": "email", "header": "Email", "accessor": "email" }
      ]
    },
    "data": { "path": "$.users" }
  }
}
```

#### Conditional Display (via binding)
```json
{
  "id": "premium-badge",
  "component": {
    "type": "badge",
    "content": { "literalString": "Premium" }
  },
  "style": {
    "className": "{{ $.user.isPremium ? 'block' : 'hidden' }}"
  }
}
```

---

## Form Input Binding

Form inputs require a `value` binding for two-way data flow.

### Text Field
```json
{
  "id": "email-input",
  "component": {
    "type": "textField",
    "value": { "path": "$.form.email", "defaultValue": "" },
    "label": { "literalString": "Email Address" },
    "placeholder": { "literalString": "you@example.com" }
  }
}
```

### Select Dropdown
```json
{
  "id": "country-select",
  "component": {
    "type": "select",
    "value": { "path": "$.form.country", "defaultValue": "" },
    "options": { "path": "$.countries" },
    "label": { "literalString": "Country" }
  }
}
```

### Checkbox
```json
{
  "id": "terms-checkbox",
  "component": {
    "type": "checkbox",
    "checked": { "path": "$.form.acceptedTerms", "defaultValue": false },
    "label": { "literalString": "I accept the terms and conditions" }
  }
}
```

---

## Chart Data Binding

### Nivo Bar Chart
```json
{
  "id": "sales-chart",
  "component": {
    "type": "nivoChart",
    "chartType": { "literalString": "bar" },
    "data": { "path": "$.salesData" },
    "indexBy": { "literalString": "month" },
    "keys": { "literalOptions": [
      { "value": "revenue", "label": "Revenue" },
      { "value": "profit", "label": "Profit" }
    ]}
  }
}
```

**Expected data format:**
```json
{
  "salesData": [
    { "month": "Jan", "revenue": 100, "profit": 20 },
    { "month": "Feb", "revenue": 150, "profit": 35 }
  ]
}
```

### Nivo Pie Chart
```json
{
  "id": "category-pie",
  "component": {
    "type": "nivoChart",
    "chartType": { "literalString": "pie" },
    "data": { "path": "$.categoryBreakdown" }
  }
}
```

**Expected data format:**
```json
{
  "categoryBreakdown": [
    { "id": "electronics", "label": "Electronics", "value": 45 },
    { "id": "clothing", "label": "Clothing", "value": 30 },
    { "id": "food", "label": "Food", "value": 25 }
  ]
}
```

---

## Complex Nested Structures

### Array of Objects
```json
{
  "path": "$.orders[*]",
  "defaultValue": []
}
```

### Deep Nesting
```json
{
  "path": "$.company.departments[0].employees[*].name"
}
```

### With Filters (when supported)
```json
{
  "path": "$.products[?(@.inStock == true)]"
}
```

---

## Best Practices

1. **Always provide defaultValue** for paths that might not exist
2. **Use descriptive paths** - prefer `$.user.firstName` over `$.u.fn`
3. **Keep paths shallow** when possible for performance
4. **Use literalOptions** for static lists, `path` for dynamic lists
5. **Test bindings** with sample data before deployment
6. **Initialize bindings in dataModel** so generated UIs render sensible content before live data arrives

---

## dataModel Section

When using `path` bindings, always include a `dataModel` array at the top level to provide initial values:

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
    { "path": "$.items", "value": [
      { "id": 1, "name": "Item A" },
      { "id": 2, "name": "Item B" }
    ]},
    { "path": "$.settings.darkMode", "value": true }
  ]
}
```

Rules:

- Each entry has `path` (JSONPath starting with `$`) and `value` (any JSON value).
- Paths should correspond to the `path` values used in BoundValue bindings.
- Provide useful defaults so the UI renders correctly on first load.
- Nested objects can be set as one entry (`$.user` with an object value) or as individual paths (`$.user.name`, `$.user.email`).

---

## Mixing Static and Dynamic

You can use static values for labels and dynamic values for content:

```json
{
  "id": "welcome-message",
  "component": {
    "type": "row",
    "gap": { "literalString": "0.5rem" },
    "children": { "explicitList": ["welcome-label", "user-name"] }
  }
},
{
  "id": "welcome-label",
  "component": {
    "type": "text",
    "content": { "literalString": "Welcome," },
    "weight": { "literalString": "normal" }
  }
},
{
  "id": "user-name",
  "component": {
    "type": "text",
    "content": { "path": "$.user.name", "defaultValue": "Guest" },
    "weight": { "literalString": "bold" }
  }
}
```
