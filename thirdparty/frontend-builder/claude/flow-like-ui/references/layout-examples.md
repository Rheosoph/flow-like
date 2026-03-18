# A2UI Layout Examples

Complete JSON examples for common UI patterns.

---

## Page Layouts

### Basic Page with Header and Content

```json
{
  "rootComponentId": "page-layout",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "0"
  },
  "components": [
    {
      "id": "page-layout",
      "style": { "className": "min-h-screen flex flex-col" },
      "component": {
        "type": "column",
        "children": { "explicitList": ["header", "main-content"] }
      }
    },
    {
      "id": "header",
      "style": { "className": "border-b border-border px-4 py-3" },
      "component": {
        "type": "row",
        "justify": { "literalString": "between" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["logo", "nav"] }
      }
    },
    {
      "id": "logo",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "AppName" },
        "variant": { "literalString": "h4" },
        "weight": { "literalString": "bold" }
      }
    },
    {
      "id": "nav",
      "style": { "className": "" },
      "component": {
        "type": "row",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["nav-1", "nav-2", "nav-3"] }
      }
    },
    {
      "id": "nav-1",
      "style": { "className": "" },
      "component": {
        "type": "link",
        "href": { "literalString": "/" },
        "label": { "literalString": "Home" },
        "variant": "default"
      }
    },
    {
      "id": "nav-2",
      "style": { "className": "" },
      "component": {
        "type": "link",
        "href": { "literalString": "/features" },
        "label": { "literalString": "Features" },
        "variant": "default"
      }
    },
    {
      "id": "nav-3",
      "style": { "className": "" },
      "component": {
        "type": "link",
        "href": { "literalString": "/pricing" },
        "label": { "literalString": "Pricing" },
        "variant": "default"
      }
    },
    {
      "id": "main-content",
      "style": { "className": "flex-1 p-6" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1.5rem" },
        "children": { "explicitList": ["page-title", "content-area"] }
      }
    },
    {
      "id": "page-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Welcome" },
        "variant": { "literalString": "h1" }
      }
    },
    {
      "id": "content-area",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Your content goes here." },
        "color": { "literalString": "text-muted-foreground" }
      }
    }
  ]
}
```

### Sidebar Layout

```json
{
  "rootComponentId": "sidebar-layout",
  "canvasSettings": {
    "backgroundColor": "bg-background"
  },
  "components": [
    {
      "id": "sidebar-layout",
      "style": { "className": "flex min-h-screen" },
      "component": {
        "type": "row",
        "children": { "explicitList": ["sidebar", "main-area"] }
      }
    },
    {
      "id": "sidebar",
      "style": { "className": "w-64 border-r border-border bg-muted/50 p-4" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "children": { "explicitList": ["sidebar-title", "sidebar-nav"] }
      }
    },
    {
      "id": "sidebar-title",
      "style": { "className": "mb-4" },
      "component": {
        "type": "text",
        "content": { "literalString": "Dashboard" },
        "variant": { "literalString": "h5" },
        "weight": { "literalString": "semibold" }
      }
    },
    {
      "id": "sidebar-nav",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.25rem" },
        "children": { "explicitList": ["nav-overview", "nav-analytics", "nav-settings"] }
      }
    },
    {
      "id": "nav-overview",
      "style": { "className": "p-2 rounded hover:bg-muted" },
      "component": {
        "type": "row",
        "gap": { "literalString": "0.5rem" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["nav-overview-icon", "nav-overview-text"] }
      }
    },
    {
      "id": "nav-overview-icon",
      "style": { "className": "" },
      "component": {
        "type": "icon",
        "name": { "literalString": "home" },
        "size": { "literalString": "sm" }
      }
    },
    {
      "id": "nav-overview-text",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Overview" }
      }
    },
    {
      "id": "nav-analytics",
      "style": { "className": "p-2 rounded hover:bg-muted" },
      "component": {
        "type": "row",
        "gap": { "literalString": "0.5rem" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["nav-analytics-icon", "nav-analytics-text"] }
      }
    },
    {
      "id": "nav-analytics-icon",
      "style": { "className": "" },
      "component": {
        "type": "icon",
        "name": { "literalString": "bar-chart-2" },
        "size": { "literalString": "sm" }
      }
    },
    {
      "id": "nav-analytics-text",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Analytics" }
      }
    },
    {
      "id": "nav-settings",
      "style": { "className": "p-2 rounded hover:bg-muted" },
      "component": {
        "type": "row",
        "gap": { "literalString": "0.5rem" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["nav-settings-icon", "nav-settings-text"] }
      }
    },
    {
      "id": "nav-settings-icon",
      "style": { "className": "" },
      "component": {
        "type": "icon",
        "name": { "literalString": "settings" },
        "size": { "literalString": "sm" }
      }
    },
    {
      "id": "nav-settings-text",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Settings" }
      }
    },
    {
      "id": "main-area",
      "style": { "className": "flex-1 p-6" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["main-title"] }
      }
    },
    {
      "id": "main-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Main Content Area" },
        "variant": { "literalString": "h2" }
      }
    }
  ]
}
```

---

## Forms

### Login Form

```json
{
  "rootComponentId": "login-container",
  "canvasSettings": {
    "backgroundColor": "bg-background"
  },
  "components": [
    {
      "id": "login-container",
      "style": { "className": "min-h-screen flex items-center justify-center p-4" },
      "component": {
        "type": "center",
        "children": { "explicitList": ["login-card"] }
      }
    },
    {
      "id": "login-card",
      "style": { "className": "w-full max-w-sm" },
      "component": {
        "type": "card",
        "title": { "literalString": "Welcome Back" },
        "description": { "literalString": "Enter your credentials to sign in" },
        "children": { "explicitList": ["login-form"] }
      }
    },
    {
      "id": "login-form",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["email-field", "password-field", "login-btn", "signup-link"] }
      }
    },
    {
      "id": "email-field",
      "style": { "className": "" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Email" },
        "placeholder": { "literalString": "you@example.com" },
        "inputType": { "literalString": "email" },
        "required": { "literalBool": true }
      }
    },
    {
      "id": "password-field",
      "style": { "className": "" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Password" },
        "placeholder": { "literalString": "••••••••" },
        "inputType": { "literalString": "password" },
        "required": { "literalBool": true }
      }
    },
    {
      "id": "login-btn",
      "style": { "className": "w-full" },
      "component": {
        "type": "button",
        "label": { "literalString": "Sign In" },
        "variant": { "literalString": "default" }
      }
    },
    {
      "id": "signup-link",
      "style": { "className": "text-center" },
      "component": {
        "type": "text",
        "content": { "literalString": "Don't have an account? Sign up" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    }
  ]
}
```

### Contact Form

```json
{
  "rootComponentId": "contact-form",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "2rem"
  },
  "components": [
    {
      "id": "contact-form",
      "style": { "className": "max-w-lg mx-auto" },
      "component": {
        "type": "card",
        "title": { "literalString": "Contact Us" },
        "description": { "literalString": "Fill out the form below and we'll get back to you." },
        "children": { "explicitList": ["form-fields"] }
      }
    },
    {
      "id": "form-fields",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["name-row", "email-input", "subject-select", "message-input", "submit-btn"] }
      }
    },
    {
      "id": "name-row",
      "style": { "className": "" },
      "component": {
        "type": "row",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["first-name", "last-name"] }
      }
    },
    {
      "id": "first-name",
      "style": { "className": "flex-1" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "First Name" },
        "placeholder": { "literalString": "John" }
      }
    },
    {
      "id": "last-name",
      "style": { "className": "flex-1" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Last Name" },
        "placeholder": { "literalString": "Doe" }
      }
    },
    {
      "id": "email-input",
      "style": { "className": "" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Email" },
        "placeholder": { "literalString": "john@example.com" },
        "inputType": { "literalString": "email" }
      }
    },
    {
      "id": "subject-select",
      "style": { "className": "" },
      "component": {
        "type": "select",
        "value": { "literalString": "" },
        "label": { "literalString": "Subject" },
        "placeholder": { "literalString": "Select a topic" },
        "options": {
          "literalOptions": [
            { "value": "general", "label": "General Inquiry" },
            { "value": "support", "label": "Technical Support" },
            { "value": "sales", "label": "Sales Question" },
            { "value": "feedback", "label": "Feedback" }
          ]
        }
      }
    },
    {
      "id": "message-input",
      "style": { "className": "" },
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Message" },
        "placeholder": { "literalString": "How can we help you?" },
        "multiline": { "literalBool": true },
        "rows": { "literalNumber": 4 }
      }
    },
    {
      "id": "submit-btn",
      "style": { "className": "w-full" },
      "component": {
        "type": "button",
        "label": { "literalString": "Send Message" },
        "variant": { "literalString": "default" },
        "icon": { "literalString": "send" },
        "iconPosition": { "literalString": "right" }
      }
    }
  ]
}
```

---

## Cards & Grids

### Feature Grid

```json
{
  "rootComponentId": "features-section",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "2rem"
  },
  "components": [
    {
      "id": "features-section",
      "style": { "className": "max-w-6xl mx-auto" },
      "component": {
        "type": "column",
        "gap": { "literalString": "2rem" },
        "children": { "explicitList": ["section-header", "features-grid"] }
      }
    },
    {
      "id": "section-header",
      "style": { "className": "text-center" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["section-title", "section-desc"] }
      }
    },
    {
      "id": "section-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Features" },
        "variant": { "literalString": "h2" }
      }
    },
    {
      "id": "section-desc",
      "style": { "className": "max-w-2xl" },
      "component": {
        "type": "text",
        "content": { "literalString": "Everything you need to build amazing applications" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "features-grid",
      "style": { "className": "" },
      "component": {
        "type": "grid",
        "columns": { "literalString": "repeat(auto-fit, minmax(280px, 1fr))" },
        "gap": { "literalString": "1.5rem" },
        "children": { "explicitList": ["feature-1", "feature-2", "feature-3"] }
      }
    },
    {
      "id": "feature-1",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "variant": { "literalString": "bordered" },
        "padding": { "literalString": "1.5rem" },
        "children": { "explicitList": ["feature-1-content"] }
      }
    },
    {
      "id": "feature-1-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["feature-1-icon", "feature-1-title", "feature-1-desc"] }
      }
    },
    {
      "id": "feature-1-icon",
      "style": { "className": "text-primary" },
      "component": {
        "type": "icon",
        "name": { "literalString": "zap" },
        "size": { "literalString": "lg" }
      }
    },
    {
      "id": "feature-1-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Lightning Fast" },
        "variant": { "literalString": "h4" }
      }
    },
    {
      "id": "feature-1-desc",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Built for performance with optimized rendering and caching." },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "feature-2",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "variant": { "literalString": "bordered" },
        "padding": { "literalString": "1.5rem" },
        "children": { "explicitList": ["feature-2-content"] }
      }
    },
    {
      "id": "feature-2-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["feature-2-icon", "feature-2-title", "feature-2-desc"] }
      }
    },
    {
      "id": "feature-2-icon",
      "style": { "className": "text-primary" },
      "component": {
        "type": "icon",
        "name": { "literalString": "shield" },
        "size": { "literalString": "lg" }
      }
    },
    {
      "id": "feature-2-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Secure by Default" },
        "variant": { "literalString": "h4" }
      }
    },
    {
      "id": "feature-2-desc",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Enterprise-grade security with encryption and access controls." },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "feature-3",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "variant": { "literalString": "bordered" },
        "padding": { "literalString": "1.5rem" },
        "children": { "explicitList": ["feature-3-content"] }
      }
    },
    {
      "id": "feature-3-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["feature-3-icon", "feature-3-title", "feature-3-desc"] }
      }
    },
    {
      "id": "feature-3-icon",
      "style": { "className": "text-primary" },
      "component": {
        "type": "icon",
        "name": { "literalString": "users" },
        "size": { "literalString": "lg" }
      }
    },
    {
      "id": "feature-3-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Team Collaboration" },
        "variant": { "literalString": "h4" }
      }
    },
    {
      "id": "feature-3-desc",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Real-time collaboration with your entire team." },
        "color": { "literalString": "text-muted-foreground" }
      }
    }
  ]
}
```

---

## Data Display

### Stats Dashboard

```json
{
  "rootComponentId": "dashboard",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1.5rem"
  },
  "components": [
    {
      "id": "dashboard",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1.5rem" },
        "children": { "explicitList": ["stats-row", "charts-row"] }
      }
    },
    {
      "id": "stats-row",
      "style": { "className": "" },
      "component": {
        "type": "grid",
        "columns": { "literalString": "repeat(auto-fit, minmax(200px, 1fr))" },
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["stat-1", "stat-2", "stat-3", "stat-4"] }
      }
    },
    {
      "id": "stat-1",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "padding": { "literalString": "1rem" },
        "children": { "explicitList": ["stat-1-content"] }
      }
    },
    {
      "id": "stat-1-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "children": { "explicitList": ["stat-1-label", "stat-1-value"] }
      }
    },
    {
      "id": "stat-1-label",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Total Users" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "stat-1-value",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "path": "$.stats.totalUsers", "defaultValue": "0" },
        "variant": { "literalString": "h3" },
        "weight": { "literalString": "bold" }
      }
    },
    {
      "id": "stat-2",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "padding": { "literalString": "1rem" },
        "children": { "explicitList": ["stat-2-content"] }
      }
    },
    {
      "id": "stat-2-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "children": { "explicitList": ["stat-2-label", "stat-2-value"] }
      }
    },
    {
      "id": "stat-2-label",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Revenue" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "stat-2-value",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "path": "$.stats.revenue", "defaultValue": "$0" },
        "variant": { "literalString": "h3" },
        "weight": { "literalString": "bold" }
      }
    },
    {
      "id": "stat-3",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "padding": { "literalString": "1rem" },
        "children": { "explicitList": ["stat-3-content"] }
      }
    },
    {
      "id": "stat-3-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "children": { "explicitList": ["stat-3-label", "stat-3-value"] }
      }
    },
    {
      "id": "stat-3-label",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Active Sessions" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "stat-3-value",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "path": "$.stats.activeSessions", "defaultValue": "0" },
        "variant": { "literalString": "h3" },
        "weight": { "literalString": "bold" }
      }
    },
    {
      "id": "stat-4",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "padding": { "literalString": "1rem" },
        "children": { "explicitList": ["stat-4-content"] }
      }
    },
    {
      "id": "stat-4-content",
      "style": { "className": "" },
      "component": {
        "type": "column",
        "gap": { "literalString": "0.5rem" },
        "children": { "explicitList": ["stat-4-label", "stat-4-value"] }
      }
    },
    {
      "id": "stat-4-label",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Conversion Rate" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "stat-4-value",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "path": "$.stats.conversionRate", "defaultValue": "0%" },
        "variant": { "literalString": "h3" },
        "weight": { "literalString": "bold" }
      }
    },
    {
      "id": "charts-row",
      "style": { "className": "" },
      "component": {
        "type": "grid",
        "columns": { "literalString": "1fr 1fr" },
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["line-chart-card", "bar-chart-card"] }
      }
    },
    {
      "id": "line-chart-card",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "title": { "literalString": "Visitors Over Time" },
        "children": { "explicitList": ["line-chart"] }
      }
    },
    {
      "id": "line-chart",
      "style": { "className": "" },
      "component": {
        "type": "nivoChart",
        "chartType": { "literalString": "line" },
        "data": { "path": "$.charts.visitors" },
        "height": { "literalString": "300px" },
        "animate": { "literalBool": true }
      }
    },
    {
      "id": "bar-chart-card",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "title": { "literalString": "Sales by Category" },
        "children": { "explicitList": ["bar-chart"] }
      }
    },
    {
      "id": "bar-chart",
      "style": { "className": "" },
      "component": {
        "type": "nivoChart",
        "chartType": { "literalString": "bar" },
        "data": { "path": "$.charts.sales" },
        "indexBy": { "literalString": "category" },
        "keys": { "literalOptions": [{ "value": "amount", "label": "Amount" }] },
        "height": { "literalString": "300px" },
        "animate": { "literalBool": true }
      }
    }
  ]
}
```

---

## User Lists

### User Table

```json
{
  "rootComponentId": "users-section",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1.5rem"
  },
  "components": [
    {
      "id": "users-section",
      "style": { "className": "" },
      "component": {
        "type": "card",
        "title": { "literalString": "Team Members" },
        "description": { "literalString": "Manage your team and their permissions" },
        "children": { "explicitList": ["users-table"] }
      }
    },
    {
      "id": "users-table",
      "style": { "className": "" },
      "component": {
        "type": "table",
        "columns": {
          "literalOptions": [
            { "id": "name", "header": "Name", "accessor": "name" },
            { "id": "email", "header": "Email", "accessor": "email" },
            { "id": "role", "header": "Role", "accessor": "role" },
            { "id": "status", "header": "Status", "accessor": "status" }
          ]
        },
        "data": { "path": "$.users" },
        "striped": { "literalBool": true },
        "hoverable": { "literalBool": true },
        "searchable": { "literalBool": true },
        "paginated": { "literalBool": true },
        "pageSize": { "literalNumber": 10 }
      }
    }
  ]
}
```

---

## Empty States

### No Data State

```json
{
  "rootComponentId": "empty-state",
  "components": [
    {
      "id": "empty-state",
      "style": { "className": "py-12" },
      "component": {
        "type": "center",
        "children": { "explicitList": ["empty-content"] }
      }
    },
    {
      "id": "empty-content",
      "style": { "className": "text-center max-w-sm" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["empty-icon", "empty-title", "empty-desc", "empty-action"] }
      }
    },
    {
      "id": "empty-icon",
      "style": { "className": "text-muted-foreground" },
      "component": {
        "type": "icon",
        "name": { "literalString": "inbox" },
        "size": { "literalNumber": 48 }
      }
    },
    {
      "id": "empty-title",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "No items yet" },
        "variant": { "literalString": "h4" }
      }
    },
    {
      "id": "empty-desc",
      "style": { "className": "" },
      "component": {
        "type": "text",
        "content": { "literalString": "Get started by creating your first item." },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "empty-action",
      "style": { "className": "" },
      "component": {
        "type": "button",
        "label": { "literalString": "Create Item" },
        "variant": { "literalString": "default" },
        "icon": { "literalString": "plus" }
      }
    }
  ]
}
```

---

## Settings Panel

### Notification Settings with Toggles

```json
{
  "rootComponentId": "settings-panel",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1.5rem"
  },
  "components": [
    {
      "id": "settings-panel",
      "style": { "className": "max-w-lg mx-auto" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1.5rem" },
        "children": { "explicitList": ["settings-header", "toggle-list", "save-btn"] }
      }
    },
    {
      "id": "settings-header",
      "component": {
        "type": "text",
        "content": { "literalString": "Notification Settings" },
        "variant": { "literalString": "h3" }
      }
    },
    {
      "id": "toggle-list",
      "component": {
        "type": "column",
        "gap": { "literalString": "0.75rem" },
        "children": { "explicitList": ["email-toggle", "push-toggle"] }
      }
    },
    {
      "id": "email-toggle",
      "style": { "className": "flex items-center justify-between p-3 rounded-lg border border-border" },
      "component": {
        "type": "row",
        "align": { "literalString": "center" },
        "justify": { "literalString": "between" },
        "children": { "explicitList": ["email-info", "email-switch"] }
      }
    },
    {
      "id": "email-info",
      "component": {
        "type": "column",
        "gap": { "literalString": "2px" },
        "children": { "explicitList": ["email-title", "email-desc"] }
      }
    },
    {
      "id": "email-title",
      "component": {
        "type": "text",
        "content": { "literalString": "Email Notifications" },
        "weight": { "literalString": "medium" }
      }
    },
    {
      "id": "email-desc",
      "component": {
        "type": "text",
        "content": { "literalString": "Receive updates via email" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "email-switch",
      "component": {
        "type": "switch",
        "checked": { "path": "$.settings.emailEnabled", "defaultValue": true }
      }
    },
    {
      "id": "push-toggle",
      "style": { "className": "flex items-center justify-between p-3 rounded-lg border border-border" },
      "component": {
        "type": "row",
        "align": { "literalString": "center" },
        "justify": { "literalString": "between" },
        "children": { "explicitList": ["push-info", "push-switch"] }
      }
    },
    {
      "id": "push-info",
      "component": {
        "type": "column",
        "gap": { "literalString": "2px" },
        "children": { "explicitList": ["push-title", "push-desc"] }
      }
    },
    {
      "id": "push-title",
      "component": {
        "type": "text",
        "content": { "literalString": "Push Notifications" },
        "weight": { "literalString": "medium" }
      }
    },
    {
      "id": "push-desc",
      "component": {
        "type": "text",
        "content": { "literalString": "Get notified on your device" },
        "size": { "literalString": "sm" },
        "color": { "literalString": "text-muted-foreground" }
      }
    },
    {
      "id": "push-switch",
      "component": {
        "type": "switch",
        "checked": { "path": "$.settings.pushEnabled", "defaultValue": false }
      }
    },
    {
      "id": "save-btn",
      "style": { "className": "w-full" },
      "component": {
        "type": "button",
        "label": { "literalString": "Save Settings" },
        "variant": { "literalString": "default" },
        "actions": [{ "name": "saveSettings", "context": {} }]
      }
    }
  ],
  "dataModel": [
    { "path": "$.settings.emailEnabled", "value": true },
    { "path": "$.settings.pushEnabled", "value": false }
  ]
}
```

---

## Game UI

### Simple Game HUD

```json
{
  "rootComponentId": "game-hud",
  "canvasSettings": {
    "backgroundColor": "transparent",
    "padding": "0"
  },
  "components": [
    {
      "id": "game-hud",
      "style": { "className": "relative w-full h-full" },
      "component": {
        "type": "column",
        "children": { "explicitList": ["top-bar", "dialogue-area"] }
      }
    },
    {
      "id": "top-bar",
      "style": { "className": "p-4" },
      "component": {
        "type": "row",
        "justify": { "literalString": "between" },
        "align": { "literalString": "center" },
        "children": { "explicitList": ["hp-bar", "score-badge"] }
      }
    },
    {
      "id": "hp-bar",
      "style": { "className": "w-48" },
      "component": {
        "type": "healthBar",
        "value": { "path": "$.player.hp", "defaultValue": 75 },
        "maxValue": { "literalNumber": 100 },
        "label": { "literalString": "HP" },
        "variant": { "literalString": "danger" },
        "showLabel": { "literalBool": true },
        "animated": { "literalBool": true }
      }
    },
    {
      "id": "score-badge",
      "component": {
        "type": "badge",
        "content": { "path": "$.player.score", "defaultValue": "0 pts" },
        "variant": { "literalString": "secondary" }
      }
    },
    {
      "id": "dialogue-area",
      "style": { "className": "mt-auto p-4" },
      "component": {
        "type": "dialogue",
        "text": { "path": "$.dialogue.text", "defaultValue": "Welcome, adventurer!" },
        "speakerName": { "path": "$.dialogue.speaker", "defaultValue": "Narrator" },
        "typewriter": { "literalBool": true },
        "speed": { "literalNumber": 30 }
      }
    }
  ],
  "dataModel": [
    { "path": "$.player.hp", "value": 75 },
    { "path": "$.player.score", "value": "1,250 pts" },
    { "path": "$.dialogue.text", "value": "Welcome, adventurer! Your quest begins here." },
    { "path": "$.dialogue.speaker", "value": "Narrator" }
  ]
}
```

---

## Tabs with Content

### Tabbed Settings Page

```json
{
  "rootComponentId": "tabbed-settings",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1.5rem"
  },
  "components": [
    {
      "id": "tabbed-settings",
      "style": { "className": "max-w-2xl mx-auto" },
      "component": {
        "type": "column",
        "gap": { "literalString": "1.5rem" },
        "children": { "explicitList": ["page-title", "settings-tabs"] }
      }
    },
    {
      "id": "page-title",
      "component": {
        "type": "text",
        "content": { "literalString": "Account Settings" },
        "variant": { "literalString": "h2" }
      }
    },
    {
      "id": "settings-tabs",
      "component": {
        "type": "tabs",
        "value": { "literalString": "profile" },
        "tabs": [
          { "id": "profile", "label": "Profile", "icon": "user", "contentComponentId": "profile-tab" },
          { "id": "security", "label": "Security", "icon": "lock", "contentComponentId": "security-tab" }
        ]
      }
    },
    {
      "id": "profile-tab",
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["name-field", "bio-field", "profile-save"] }
      }
    },
    {
      "id": "name-field",
      "component": {
        "type": "textField",
        "value": { "path": "$.profile.name", "defaultValue": "" },
        "label": { "literalString": "Display Name" },
        "placeholder": { "literalString": "Your name" }
      }
    },
    {
      "id": "bio-field",
      "component": {
        "type": "textField",
        "value": { "path": "$.profile.bio", "defaultValue": "" },
        "label": { "literalString": "Bio" },
        "placeholder": { "literalString": "Tell us about yourself" },
        "multiline": { "literalBool": true },
        "rows": { "literalNumber": 3 }
      }
    },
    {
      "id": "profile-save",
      "component": {
        "type": "button",
        "label": { "literalString": "Save Profile" },
        "variant": { "literalString": "default" },
        "actions": [{ "name": "saveProfile", "context": {} }]
      }
    },
    {
      "id": "security-tab",
      "component": {
        "type": "column",
        "gap": { "literalString": "1rem" },
        "children": { "explicitList": ["current-password", "new-password", "security-save"] }
      }
    },
    {
      "id": "current-password",
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "Current Password" },
        "inputType": { "literalString": "password" }
      }
    },
    {
      "id": "new-password",
      "component": {
        "type": "textField",
        "value": { "literalString": "" },
        "label": { "literalString": "New Password" },
        "inputType": { "literalString": "password" }
      }
    },
    {
      "id": "security-save",
      "component": {
        "type": "button",
        "label": { "literalString": "Update Password" },
        "variant": { "literalString": "default" },
        "actions": [{ "name": "updatePassword", "context": {} }]
      }
    }
  ],
  "dataModel": [
    { "path": "$.profile.name", "value": "Jane Doe" },
    { "path": "$.profile.bio", "value": "Product designer passionate about user experience." }
  ]
}
```