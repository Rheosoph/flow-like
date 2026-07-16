//! A2UI Copilot helper functions for component documentation
//!
//! Provides component schema docs and style examples used by the system prompt.

// ============================================================================
// Helper Functions
// ============================================================================

/// Get component schema documentation
pub fn get_component_schema(component_type: &str) -> String {
    match component_type.to_lowercase().as_str() {
        "column" => r#"Column - Vertical flex container
Properties:
- type: "column" (required)
- gap: BoundValue string (e.g., { "literalString": "16px" }) - Space between children
- align: BoundValue string - "start" | "center" | "end" | "stretch" | "baseline"
- justify: BoundValue string - "start" | "center" | "end" | "between" | "around" | "evenly"
- wrap: BoundValue boolean - Whether children can wrap
- children: { explicitList: ["child-id-1", "child-id-2"] }

Example:
{
  "id": "main-column",
  "style": { "className": "p-4 gap-4" },
  "component": {
    "type": "column",
    "gap": { "literalString": "16px" },
    "children": { "explicitList": ["header", "content", "footer"] }
  }
}"#
        .to_string(),

        "row" => r#"Row - Horizontal flex container
Properties:
- type: "row" (required)
- gap: BoundValue string - Space between children
- align: BoundValue string - "start" | "center" | "end" | "stretch" | "baseline"
- justify: BoundValue string - "start" | "center" | "end" | "between" | "around" | "evenly"
- wrap: BoundValue boolean
- children: { explicitList: [...] }

Example:
{
  "id": "button-row",
  "style": { "className": "gap-2" },
  "component": {
    "type": "row",
    "gap": { "literalString": "8px" },
    "justify": { "literalString": "end" },
    "children": { "explicitList": ["cancel-btn", "submit-btn"] }
  }
}"#
        .to_string(),

        "grid" => r#"Grid - CSS Grid container
Properties:
- type: "grid" (required)
- columns: BoundValue string (e.g., { "literalString": "repeat(3, 1fr)" })
- rows: BoundValue string (optional)
- gap: BoundValue string
- autoFlow: BoundValue string - "row" | "column" | "dense"
- children: { explicitList: [...] }

Example:
{
  "id": "card-grid",
  "style": { "className": "gap-4" },
  "component": {
    "type": "grid",
    "columns": { "literalString": "repeat(auto-fill, minmax(250px, 1fr))" },
    "gap": { "literalString": "16px" },
    "children": { "explicitList": ["card-1", "card-2", "card-3"] }
  }
}"#
        .to_string(),

        "text" => r#"Text - Text display component
Properties:
- type: "text" (required)
- content: BoundValue - { literalString: "..." } or { path: "$.data.title" }
- variant: BoundValue string - "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "body" | "caption" | "code" | "label"
- size: BoundValue string - "xs" | "sm" | "md" | "lg" | "xl" | "2xl" | "3xl"
- weight: BoundValue string - "normal" | "medium" | "semibold" | "bold"
- color: BoundValue string (Tailwind color like "text-primary")
- align: BoundValue string - "left" | "center" | "right"

Example:
{
  "id": "page-title",
  "style": { "className": "text-2xl font-bold text-primary" },
  "component": {
    "type": "text",
    "content": { "literalString": "Welcome" },
    "variant": { "literalString": "h1" }
  }
}"#
        .to_string(),

        "button" => r#"Button - Interactive button
Properties:
- type: "button" (required)
- label: BoundValue - Button text
- variant: BoundValue string - "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
- size: BoundValue string - "sm" | "md" | "lg" | "icon"
- disabled: BoundValue (boolean)
- loading: BoundValue (boolean) - Shows loading spinner when true
- icon: BoundValue (string) - Lucide icon name (e.g., "send", "plus", "trash")
- iconPosition: BoundValue - "left" | "right" (default: "left")
- tooltip: BoundValue (string) - Tooltip text on hover
- actions: { onClick: { type: "emit", event: "..." } }

Example:
{
  "id": "submit-btn",
  "component": {
    "type": "button",
    "label": { "literalString": "Submit" },
    "variant": { "literalString": "default" },
    "icon": { "literalString": "send" },
    "iconPosition": { "literalString": "left" }
  },
  "actions": {
    "onClick": { "type": "emit", "event": "form_submit" }
  }
}"#
        .to_string(),

        "feedback" => r#"Feedback - Built-in thumbs up/down feedback control
Properties:
- type: "feedback" (required)
- mode: "icon" | "compact" | "segmented" | "rating" | "extended"
- size: "sm" | "md" | "lg"
- title: BoundValue
- positiveLabel / negativeLabel: BoundValue
- showComment: BoundValue (boolean)
- commentMode: "none" | "inline" | "modal" (use "modal" to keep compact/icon controls simple while collecting a comment)
- commentTitle / commentDescription / commentSubmitLabel / commentCancelLabel: BoundValue
- feedbackId: BoundValue (optional stable id)
- includeState: BoundValue (boolean; include component/page state in feedback context)
- pageContextMode: "none" | "path" | "query" (default "path"; "query" sends query params)
- pageContextQueryParamAllowlist / pageContextQueryParamDenylist: BoundValue (comma-separated query param names)
- includePageHash: BoundValue (boolean; default false)

Example:
{
  "id": "page-feedback",
  "component": {
    "type": "feedback",
    "mode": { "literalString": "segmented" },
    "title": { "literalString": "Was this helpful?" },
    "showComment": { "literalBool": true },
    "commentMode": { "literalString": "modal" },
    "pageContextMode": { "literalString": "path" }
  }
}"#
        .to_string(),

        "applink" | "app_link" => r#"AppLink - Built-in link button to app shell screens
Properties:
- type: "appLink" (required)
- target: "config" | "settings" | "overview"
- label: BoundValue (optional; defaults from target)
- variant: "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
- size: "sm" | "md" | "lg" | "icon"
- icon: BoundValue (Lucide icon name)

Example:
{
  "id": "settings-link",
  "component": {
    "type": "appLink",
    "target": { "literalString": "settings" }
  }
}"#
        .to_string(),

        "card" => r#"Card - Content container with optional header/footer
Properties:
- type: "card" (required)
- title: BoundValue (optional)
- description: BoundValue (optional)
- footer: { explicitList: [...] } (optional)
- headerActions: { explicitList: [...] } (optional)
- children: { explicitList: [...] }

Example:
{
  "id": "user-card",
  "style": { "className": "p-6 shadow-md" },
  "component": {
    "type": "card",
    "title": { "literalString": "User Profile" },
    "children": { "explicitList": ["avatar", "user-info"] }
  }
}"#
        .to_string(),

        "userprofile" | "user_profile" => r#"UserProfile - Fetch and display a Flow-Like user by subject/sub ID
Properties:
- type: "userProfile" (required)
- value: BoundValue - user subject/sub ID. Compatible with Set Element Value.
- variant: BoundValue - "avatar" | "chip" | "row" | "detailed" | "card"
- avatarSize: BoundValue - "xs" | "sm" | "md" | "lg" | "xl" | "2xl"
- showHover: BoundValue (boolean) - enable hover details
- showEmail: BoundValue (boolean)
- showDescription: BoundValue (boolean)
- showUserId: BoundValue (boolean)
- showProfileLink: BoundValue (boolean)
- fallbackLabel: BoundValue

Example:
{
  "id": "assignee-profile",
  "component": {
    "type": "userProfile",
    "value": { "path": "$.assigneeSub" },
    "variant": { "literalString": "row" },
    "showHover": { "literalBool": true }
  }
}"#
        .to_string(),

        "textfield" | "text_field" => r#"TextField - Text input
Properties:
- type: "textField" (required)
- value: BoundValue - Current value
- placeholder: BoundValue string
- inputType: BoundValue string - "text" | "email" | "password" | "number" | "tel" | "url"
- multiline: BoundValue boolean
- rows: BoundValue number (for multiline)
- disabled: BoundValue (boolean)
- error: BoundValue (string, error message)
- label: BoundValue string
- actions: { onChange: { type: "update", path: "..." } }

Example:
{
  "id": "email-input",
  "component": {
    "type": "textField",
    "value": { "path": "$.form.email" },
    "placeholder": { "literalString": "Enter email" },
    "inputType": { "literalString": "email" },
    "label": { "literalString": "Email Address" }
  },
  "actions": {
    "onChange": { "type": "update", "path": "$.form.email" }
  }
}"#
        .to_string(),

        "select" => r#"Select - Dropdown selection
Properties:
- type: "select" (required)
- value: BoundValue - Selected value
- options: [{ value: string, label: string }]
- placeholder: string
- disabled: BoundValue (boolean)
- multiple: boolean
- actions: { onChange: { type: "update", path: "..." } }

Example:
{
  "id": "country-select",
  "component": {
    "type": "select",
    "value": { "path": "$.form.country" },
    "placeholder": "Select country",
    "options": [
      { "value": "us", "label": "United States" },
      { "value": "uk", "label": "United Kingdom" }
    ]
  }
}"#
        .to_string(),

        "image" => r#"Image - Image display
Properties:
- type: "image" (required)
- src: BoundValue - Image URL
- alt: string - Alt text (required for accessibility)
- fit: "cover" | "contain" | "fill" | "none" | "scale-down"
- loading: "lazy" | "eager"
- fallback: string - Fallback image URL

Example:
{
  "id": "profile-avatar",
  "style": { "className": "w-24 h-24 rounded-full" },
  "component": {
    "type": "image",
    "src": { "path": "$.user.avatar" },
    "alt": "User avatar",
    "fit": "cover"
  }
}"#
        .to_string(),

        "icon" => r#"Icon - Lucide icon
Properties:
- type: "icon" (required)
- name: string - Lucide icon name (e.g., "user", "settings", "chevron-right")
- size: "xs" | "sm" | "md" | "lg" | "xl" or number
- color: string - Tailwind color class

Example:
{
  "id": "settings-icon",
  "style": { "className": "text-muted-foreground" },
  "component": {
    "type": "icon",
    "name": "settings",
    "size": "md"
  }
}"#
        .to_string(),

        "diffview" | "diff_view" => r#"DiffView - Side-by-side, unified or inline diff for text, code, markdown & documents
Properties:
- type: "diffView" (required)
- original: BoundValue (required) - Left/old content or document URL
- modified: BoundValue (required) - Right/new content or document URL
- mode: "split" | "unified" | "inline" (default "split")
- kind: "auto" | "text" | "code" | "markdown" | "json" | "document" (default "auto")
- language: BoundValue (string) - Syntax language e.g. "typescript", "rust", "json", "markdown", "python" (default "plaintext")
- markdownMode: "source" | "rendered" (default "source")
- showLineNumbers: BoundValue (boolean, default true)
- wordWrap: BoundValue (boolean, default false)
- wordLevel: BoundValue (boolean, default true)
- collapseUnchanged: BoundValue (boolean, default false)
- contextLines: BoundValue (number, default 3)
- showStats: BoundValue (boolean, default true)
- originalLabel: BoundValue (string, default "Original")
- modifiedLabel: BoundValue (string, default "Modified")
- ignoreWhitespace: BoundValue (boolean, default false)
- ignoreCase: BoundValue (boolean, default false)
- trimTrailingWhitespace: BoundValue (boolean, default false)
- swapSides: BoundValue (boolean, default false)

Example:
{
  "id": "config-diff",
  "component": {
    "type": "diffView",
    "original": { "literalString": "const port = 3000" },
    "modified": { "literalString": "const port = 8080" },
    "mode": { "literalString": "split" },
    "kind": { "literalString": "code" },
    "language": { "literalString": "typescript" },
    "showLineNumbers": { "literalBool": true }
  }
}"#
        .to_string(),

        "calendar" => r#"Calendar - Interactive calendar for viewing & scheduling events
Properties:
- type: "calendar" (required)
- events: BoundValue (required) - Array of CalendarEvent { id, title, start, end?, allDay?, color?, description?, location?, calendarId?, editable?, link?, metadata? } (link: relative path navigates in-app, absolute URL opens externally)
- view: "month" | "week" | "day" | "agenda" (default "month")
- date: BoundValue (string, ISO 8601) - Focused date
- editable: BoundValue (boolean, default true) - Allow drag/resize
- selectable: BoundValue (boolean, default true) - Allow slot selection to create
- firstDayOfWeek: BoundValue (number, 0=Sunday, default 1)
- minTime / maxTime: BoundValue (string "HH:MM") - Time-grid bounds
- slotDuration: BoundValue (number, minutes, default 30)
- showWeekends / showNowIndicator / showAllDay: BoundValue (boolean)
- locale: BoundValue (string, e.g. "en-US")
- title: BoundValue (string) - Header title
- density: "compact" | "default" | "comfortable"
- showViewSwitcher: BoundValue (boolean, default true)
- height: BoundValue (string, CSS value e.g. "600px")
- responsive: BoundValue (boolean) - auto agenda on narrow widths
- compactBreakpoint: BoundValue (number, px)
- actions: bind a workflow_event action; interactions fire with _action_context { interaction: "create"|"move"|"resize"|"open"|"delete", id?, start, end, ... }

Example:
{
  "id": "planner",
  "component": {
    "type": "calendar",
    "events": { "path": "$.events" },
    "view": { "literalString": "week" },
    "editable": { "literalBool": true }
  }
}"#
        .to_string(),

        "gantt" => r#"Gantt - Interactive Gantt timeline for planning tasks
Properties:
- type: "gantt" (required)
- tasks: BoundValue (required) - Array of GanttTask { id, name, start, end, progress?, dependencies?, parent?, color?, assignee?, milestone?, collapsed?, link?, metadata? } (link: relative path navigates in-app, absolute URL opens externally)
- view: "day" | "week" | "month" | "quarter" | "compact" (default "week")
- editable: BoundValue (boolean, default true) - Master switch for drag/resize/link
- draggable / resizable: BoundValue (boolean) - Fine-grained edit controls
- showDependencies: BoundValue (boolean, default true) - Draw dependency arrows
- showProgress: BoundValue (boolean, default true) - Bar progress fill
- showToday: BoundValue (boolean, default true) - Today marker line
- rowHeight: BoundValue (number, px)
- columns: BoundValue (array of string) - Extra left-panel columns e.g. ["assignee","progress"]
- title: BoundValue (string) - Header title
- density: "compact" | "default" | "comfortable"
- showViewSwitcher: BoundValue (boolean, default true)
- showTaskList: BoundValue (boolean, default true) - Left task-list panel
- taskListWidth: BoundValue (number, px)
- shadeWeekends: BoundValue (boolean, default true)
- height: BoundValue (string, CSS value e.g. "600px")
- responsive: BoundValue (boolean) - auto compact on narrow widths
- compactBreakpoint: BoundValue (number, px)
- actions: bind a workflow_event action; interactions fire with _action_context { interaction: "create"|"move"|"resize"|"open"|"delete"|"link", id?, start?, end?, fromId?, toId? }

Example:
{
  "id": "roadmap",
  "component": {
    "type": "gantt",
    "tasks": { "path": "$.tasks" },
    "view": { "literalString": "week" },
    "showDependencies": { "literalBool": true }
  }
}"#
        .to_string(),

        "checkbox" => r#"Checkbox - Boolean toggle with label
Properties:
- type: "checkbox" (required)
- checked: BoundValue (boolean)
- label: string
- disabled: BoundValue (boolean)
- actions: { onChange: { type: "update", path: "..." } }

Example:
{
  "id": "terms-checkbox",
  "component": {
    "type": "checkbox",
    "checked": { "path": "$.form.acceptTerms" },
    "label": "I accept the terms and conditions"
  }
}"#
        .to_string(),

        "switch" => r#"Switch - Toggle switch
Properties:
- type: "switch" (required)
- checked: BoundValue (boolean)
- label: string
- disabled: BoundValue (boolean)

Example:
{
  "id": "notifications-switch",
  "component": {
    "type": "switch",
    "checked": { "path": "$.settings.notifications" },
    "label": "Enable notifications"
  }
}"#
        .to_string(),

        "tabs" => r#"Tabs - Tabbed content container
Properties:
- type: "tabs" (required)
- value: BoundValue - Active tab value
- tabs: [{ value: string, label: string, icon?: string }]
- children: { explicitList: [...] } - Tab content panels

Example:
{
  "id": "settings-tabs",
  "component": {
    "type": "tabs",
    "value": { "path": "$.ui.activeTab" },
    "tabs": [
      { "value": "general", "label": "General", "icon": "settings" },
      { "value": "security", "label": "Security", "icon": "shield" }
    ],
    "children": { "explicitList": ["general-panel", "security-panel"] }
  }
}"#
        .to_string(),

        "modal" => r#"Modal - Dialog overlay
Properties:
- type: "modal" (required)
- open: BoundValue (boolean)
- title: BoundValue
- description: BoundValue (optional)
- children: { explicitList: [...] }
- actions: { onClose: { type: "update", path: "...", value: false } }

Example:
{
  "id": "confirm-modal",
  "component": {
    "type": "modal",
    "open": { "path": "$.ui.showConfirm" },
    "title": { "literalString": "Confirm Action" },
    "children": { "explicitList": ["modal-content", "modal-actions"] }
  }
}"#
        .to_string(),

        _ => format!(
            "No detailed schema page for component type: {}. Detailed pages exist for: column, row, grid, text, button, feedback, appLink, card, userProfile, textField, select, image, icon, diffView, calendar, gantt, checkbox, switch, tabs, modal (plus style categories spacing, colors, effects, layout, responsive, typography). For every other component, use the component documentation embedded in your system prompt — it is the authoritative reference.",
            component_type
        ),
    }
}

/// Get style examples for a category
pub fn get_style_examples(category: &str) -> String {
    match category.to_lowercase().as_str() {
        "spacing" => r#"Spacing Classes:
- Padding: p-1 p-2 p-3 p-4 p-5 p-6 p-8 p-10 p-12
- Padding X/Y: px-4 py-2 px-6 py-4
- Padding directional: pt-4 pr-4 pb-4 pl-4
- Margin: m-1 m-2 m-4 m-auto mx-auto my-4
- Gap: gap-1 gap-2 gap-3 gap-4 gap-6 gap-8

Common patterns:
- Card: "p-4" or "p-6"
- Button: "px-4 py-2"
- Section: "py-8" or "py-12"
- Container: "px-4 mx-auto max-w-screen-lg""#
            .to_string(),

        "colors" => r#"Color Classes:
Background:
- bg-background bg-card bg-popover bg-muted
- bg-primary bg-secondary bg-accent bg-destructive
- bg-white bg-black bg-transparent
- bg-gray-100 bg-gray-200 ... bg-gray-900

Text:
- text-foreground text-muted-foreground
- text-primary text-secondary text-destructive
- text-white text-black
- text-gray-500 text-gray-600 text-gray-700

Border:
- border-border border-input
- border-primary border-secondary
- border-gray-200 border-gray-300

Common patterns:
- Card: "bg-card text-card-foreground"
- Muted text: "text-muted-foreground"
- Primary button: "bg-primary text-primary-foreground"
- Hover: "hover:bg-accent hover:text-accent-foreground""#
            .to_string(),

        "effects" => r#"Effect Classes:
Border radius:
- rounded-none rounded-sm rounded rounded-md rounded-lg rounded-xl rounded-2xl rounded-full

Shadows:
- shadow-none shadow-sm shadow shadow-md shadow-lg shadow-xl shadow-2xl

Opacity:
- opacity-0 opacity-25 opacity-50 opacity-75 opacity-100

Transitions:
- transition-all transition-colors transition-opacity
- duration-150 duration-200 duration-300

Common patterns:
- Card: "rounded-lg shadow-md"
- Button: "rounded-md shadow-sm hover:shadow-md transition-all"
- Avatar: "rounded-full"
- Modal: "rounded-xl shadow-2xl""#
            .to_string(),

        "layout" => r#"Layout Classes:
Display:
- flex flex-row flex-col
- grid
- block inline inline-block hidden

Flex:
- items-start items-center items-end items-stretch
- justify-start justify-center justify-end justify-between justify-around
- flex-wrap flex-nowrap
- flex-1 flex-auto flex-none

Grid:
- grid-cols-1 grid-cols-2 grid-cols-3 grid-cols-4
- grid-rows-1 grid-rows-2
- col-span-2 col-span-full
- auto-rows-min auto-rows-max

Sizing:
- w-full w-1/2 w-1/3 w-auto w-screen
- h-full h-screen h-auto
- min-w-0 max-w-md max-w-lg max-w-screen-xl
- min-h-screen

Common patterns:
- Center content: "flex items-center justify-center"
- Space between: "flex justify-between items-center"
- Responsive grid: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4""#
            .to_string(),

        "responsive" => r#"Responsive Prefixes:
- sm: 640px and up
- md: 768px and up
- lg: 1024px and up
- xl: 1280px and up
- 2xl: 1536px and up

Examples:
- "p-2 md:p-4 lg:p-6" - Padding increases with screen size
- "grid-cols-1 md:grid-cols-2 lg:grid-cols-3" - Responsive grid
- "hidden md:block" - Hide on mobile, show on tablet+
- "text-sm md:text-base lg:text-lg" - Responsive text size
- "flex-col md:flex-row" - Stack on mobile, row on tablet+"#
            .to_string(),

        "typography" => r#"Typography Classes:
Size:
- text-xs text-sm text-base text-lg text-xl text-2xl text-3xl text-4xl

Weight:
- font-thin font-light font-normal font-medium font-semibold font-bold font-extrabold

Style:
- italic not-italic underline line-through

Line height:
- leading-none leading-tight leading-snug leading-normal leading-relaxed leading-loose

Alignment:
- text-left text-center text-right text-justify

Common patterns:
- Heading: "text-2xl font-bold"
- Subheading: "text-lg font-semibold"
- Body: "text-base font-normal"
- Caption: "text-sm text-muted-foreground"
- Code: "font-mono text-sm""#
            .to_string(),

        _ => format!(
            "Unknown style category: {}. Available: spacing, colors, effects, layout, responsive, typography",
            category
        ),
    }
}
