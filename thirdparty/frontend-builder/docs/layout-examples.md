# A2UI surface examples

These complete surfaces follow the current root, BoundValue, action, responsive, and widget contracts.

Contents: [login form](#login-form), [responsive dashboard](#responsive-dashboard-with-a-repeated-inline-widget), and [planning workspace](#planning-workspace-with-calendar-and-gantt).

## Login form

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
        "type": "center",
        "children": { "explicitList": ["login-card"] }
      }
    },
    {
      "id": "login-card",
      "style": {
        "className": "w-full max-w-sm border border-border bg-card"
      },
      "component": {
        "type": "card",
        "children": { "explicitList": ["login-form"] }
      }
    },
    {
      "id": "login-form",
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": {
          "explicitList": [
            "login-heading",
            "login-description",
            "email-field",
            "password-field",
            "submit-button"
          ]
        }
      }
    },
    {
      "id": "login-heading",
      "component": {
        "type": "text",
        "content": { "literalString": "Welcome back" },
        "variant": { "literalString": "heading" },
        "size": { "literalString": "2xl" }
      }
    },
    {
      "id": "login-description",
      "component": {
        "type": "text",
        "content": { "literalString": "Sign in to continue." },
        "variant": { "literalString": "body" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "email-field",
      "eventRelevant": true,
      "component": {
        "type": "textField",
        "value": { "path": "$.form.email", "defaultValue": "" },
        "label": { "literalString": "Email" },
        "placeholder": { "literalString": "you@example.com" },
        "inputType": { "literalString": "email" },
        "required": { "literalBool": true }
      }
    },
    {
      "id": "password-field",
      "eventRelevant": true,
      "component": {
        "type": "textField",
        "value": { "path": "$.form.password", "defaultValue": "" },
        "label": { "literalString": "Password" },
        "inputType": { "literalString": "password" },
        "required": { "literalBool": true }
      }
    },
    {
      "id": "submit-button",
      "style": { "className": "w-full min-h-10" },
      "component": {
        "type": "button",
        "label": { "literalString": "Sign in" },
        "variant": { "literalString": "default" },
        "actions": [
          {
            "name": "submit",
            "context": { "formId": "login-form" }
          }
        ]
      }
    }
  ],
  "dataModel": [
    { "path": "$.form.email", "value": "" },
    { "path": "$.form.password", "value": "" }
  ]
}
```

## Responsive dashboard with a repeated inline widget

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
        "gap": { "literalString": "1.5rem" },
        "children": {
          "explicitList": ["dashboard-header", "stats-grid", "revenue-card"]
        }
      }
    },
    {
      "id": "dashboard-header",
      "style": { "className": "min-w-0" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.25rem" },
        "children": {
          "explicitList": ["dashboard-title", "dashboard-subtitle"]
        }
      }
    },
    {
      "id": "dashboard-title",
      "component": {
        "type": "text",
        "content": { "literalString": "Operations overview" },
        "variant": { "literalString": "heading" },
        "size": { "literalString": "3xl" }
      }
    },
    {
      "id": "dashboard-subtitle",
      "component": {
        "type": "text",
        "content": { "literalString": "Live performance across the business." },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "stats-grid",
      "style": {
        "className": "grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3"
      },
      "component": {
        "type": "grid",
        "children": {
          "template": {
            "dataPath": "$.stats",
            "itemIdPath": "id",
            "templateComponentId": "stat-card-template"
          }
        }
      }
    },
    {
      "id": "stat-card-template",
      "component": {
        "type": "widgetInstance",
        "widgetId": "stat-card",
        "instanceId": "stat-card-template",
        "inlineWidgetDef": {
          "name": "Stat Card",
          "rootComponentId": "stat-card-root",
          "components": [
            {
              "id": "stat-card-root",
              "style": {
                "className": "h-full rounded-lg border border-border bg-card p-4 shadow-sm"
              },
              "component": {
                "type": "column",
                "gap": { "literalString": "0.5rem" },
                "children": {
                  "explicitList": ["stat-card-label", "stat-card-value", "stat-card-trend"]
                }
              }
            },
            {
              "id": "stat-card-label",
              "component": {
                "type": "text",
                "content": { "path": "$item.label", "defaultValue": "Metric" },
                "variant": { "literalString": "label" },
                "color": { "literalString": "text-muted-foreground" }
              }
            },
            {
              "id": "stat-card-value",
              "component": {
                "type": "text",
                "content": { "path": "$item.value", "defaultValue": "—" },
                "variant": { "literalString": "heading" },
                "size": { "literalString": "2xl" }
              }
            },
            {
              "id": "stat-card-trend",
              "component": {
                "type": "badge",
                "content": { "path": "$item.trend", "defaultValue": "No change" },
                "variant": { "literalString": "secondary" }
              }
            }
          ],
          "exposedProps": []
        },
        "exposedPropValues": {},
        "actionBindings": {}
      }
    },
    {
      "id": "revenue-card",
      "style": { "className": "min-w-0 border border-border bg-card" },
      "component": {
        "type": "card",
        "title": { "literalString": "Revenue trend" },
        "children": { "explicitList": ["revenue-chart"] }
      }
    },
    {
      "id": "revenue-chart",
      "style": { "className": "w-full min-w-0" },
      "component": {
        "type": "nivoChart",
        "chartType": { "literalString": "line" },
        "data": { "path": "$.revenueSeries", "defaultValue": [] },
        "height": { "literalString": "340px" },
        "animate": { "literalBool": true },
        "showLegend": { "literalBool": true },
        "legendPosition": { "literalString": "bottom" }
      }
    }
  ],
  "dataModel": [
    {
      "path": "$.stats",
      "value": [
        { "id": "revenue", "label": "Revenue", "value": "€84.2k", "trend": "+12.4%" },
        { "id": "orders", "label": "Orders", "value": "1,284", "trend": "+6.8%" },
        { "id": "sla", "label": "SLA", "value": "99.7%", "trend": "+0.3%" }
      ]
    },
    {
      "path": "$.revenueSeries",
      "value": [
        {
          "id": "Revenue",
          "data": [
            { "x": "Jan", "y": 51 },
            { "x": "Feb", "y": 64 },
            { "x": "Mar", "y": 72 },
            { "x": "Apr", "y": 84 }
          ]
        }
      ]
    }
  ]
}
```

## Planning workspace with calendar and Gantt

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
        "gap": { "literalString": "1rem" },
        "children": {
          "explicitList": ["planning-title", "planning-grid"]
        }
      }
    },
    {
      "id": "planning-title",
      "component": {
        "type": "text",
        "content": { "literalString": "Launch planning" },
        "variant": { "literalString": "heading" },
        "size": { "literalString": "3xl" }
      }
    },
    {
      "id": "planning-grid",
      "style": {
        "className": "grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-2"
      },
      "component": {
        "type": "grid",
        "children": {
          "explicitList": ["calendar-card", "gantt-card"]
        }
      }
    },
    {
      "id": "calendar-card",
      "style": {
        "className": "min-w-0 overflow-hidden border border-border bg-card"
      },
      "component": {
        "type": "card",
        "children": { "explicitList": ["launch-calendar"] }
      }
    },
    {
      "id": "launch-calendar",
      "component": {
        "type": "calendar",
        "events": { "path": "$.planning.events", "defaultValue": [] },
        "view": { "literalString": "month" },
        "date": { "literalString": "2026-07-22" },
        "title": { "literalString": "Milestones" },
        "density": { "literalString": "compact" },
        "editable": { "literalBool": true },
        "selectable": { "literalBool": true },
        "showViewSwitcher": { "literalBool": true },
        "height": { "literalString": "620px" },
        "actions": [
          {
            "name": "planning_interaction",
            "context": { "source": "calendar" }
          }
        ]
      }
    },
    {
      "id": "gantt-card",
      "style": {
        "className": "min-w-0 overflow-hidden border border-border bg-card"
      },
      "component": {
        "type": "card",
        "children": { "explicitList": ["launch-gantt"] }
      }
    },
    {
      "id": "launch-gantt",
      "component": {
        "type": "gantt",
        "tasks": { "path": "$.planning.tasks", "defaultValue": [] },
        "view": { "literalString": "week" },
        "title": { "literalString": "Delivery timeline" },
        "density": { "literalString": "compact" },
        "editable": { "literalBool": true },
        "draggable": { "literalBool": true },
        "resizable": { "literalBool": true },
        "showDependencies": { "literalBool": true },
        "showProgress": { "literalBool": true },
        "showTaskList": { "literalBool": true },
        "height": { "literalString": "620px" },
        "actions": [
          {
            "name": "planning_interaction",
            "context": { "source": "gantt" }
          }
        ]
      }
    }
  ],
  "dataModel": [
    {
      "path": "$.planning.events",
      "value": [
        {
          "id": "event-kickoff",
          "title": "Kickoff",
          "start": "2026-07-22T09:00:00+02:00",
          "end": "2026-07-22T10:00:00+02:00",
          "editable": true,
          "metadata": { "phase": "discovery" }
        },
        {
          "id": "event-release",
          "title": "Release",
          "start": "2026-08-14",
          "allDay": true,
          "editable": true,
          "metadata": { "phase": "launch" }
        }
      ]
    },
    {
      "path": "$.planning.tasks",
      "value": [
        {
          "id": "task-design",
          "name": "Design",
          "start": "2026-07-22",
          "end": "2026-07-29",
          "progress": 70,
          "dependencies": []
        },
        {
          "id": "task-build",
          "name": "Build",
          "start": "2026-07-29",
          "end": "2026-08-10",
          "progress": 25,
          "dependencies": ["task-design"]
        },
        {
          "id": "task-release",
          "name": "Release",
          "start": "2026-08-14",
          "end": "2026-08-14",
          "progress": 0,
          "dependencies": ["task-build"],
          "milestone": true
        }
      ]
    }
  ]
}
```
