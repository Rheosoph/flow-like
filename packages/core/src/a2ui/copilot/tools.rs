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
- actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }] - actions[0] fires on click (inside the component object)

Action wiring (same contract for every interactive component):
- "workflow_event" invokes ONE named board event; context carries routing ids ONLY (nodeId, optional boardId/appId)
- NEVER copy element/dashboard values into the context - the event body reads current element state itself at runtime (Get Element -> Get Element Value / Get File Input Files)
- Other built-in names: "navigate_page" (context.route, optional context.queryParams) and "external_link" (context.url)
- A board can set or re-point this later with Set Element Action (a2uiSetElementAction)

Example:
{
  "id": "submit-btn",
  "component": {
    "type": "button",
    "label": { "literalString": "Submit" },
    "variant": { "literalString": "default" },
    "icon": { "literalString": "send" },
    "iconPosition": { "literalString": "left" },
    "actions": [{ "name": "workflow_event", "context": { "nodeId": "evt-submit-form" } }]
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
- footer: BoundValue (optional footer text)
- variant: BoundValue - "default" | "bordered" | "elevated"
- hoverable / clickable: BoundValue (boolean)
- headerImage / headerIcon: BoundValue
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
- Bind value with { "path": "$.form.email" } to persist edits. Optional on-change actions use the button's workflow_event contract; the event reads the current value with Get Element Value instead of a pushed payload

Example:
{
  "id": "email-input",
  "component": {
    "type": "textField",
    "value": { "path": "$.form.email" },
    "placeholder": { "literalString": "Enter email" },
    "inputType": { "literalString": "email" },
    "label": { "literalString": "Email Address" }
  }
}"#
        .to_string(),

        "select" => r#"Select - Dropdown selection
Properties:
- type: "select" (required)
- value: BoundValue - Selected value
- options: BoundValue - { "literalOptions": [{ "value": "...", "label": "..." }] } or { "path": "$.data.options" }
- placeholder: BoundValue (string)
- label: BoundValue (string)
- disabled: BoundValue (boolean)
- multiple: BoundValue (boolean)
- searchable: BoundValue (boolean)
- Bind value with a path to persist the selection. Optional on-change actions use the button's workflow_event contract; the event reads the current selection with Get Element Value instead of a pushed payload

Example:
{
  "id": "country-select",
  "component": {
    "type": "select",
    "value": { "path": "$.form.country" },
    "placeholder": { "literalString": "Select country" },
    "options": { "literalOptions": [
      { "value": "us", "label": "United States" },
      { "value": "uk", "label": "United Kingdom" }
    ] }
  }
}"#
        .to_string(),

        "image" => r#"Image - Image display
Properties:
- type: "image" (required)
- src: BoundValue - Image URL
- alt: BoundValue (string) - Alt text (recommended for accessibility)
- fit: BoundValue - "contain" | "cover" | "fill" | "none" | "scaleDown"
- loading: BoundValue - "lazy" | "eager"
- fallback: BoundValue (string) - Fallback image URL
- aspectRatio: BoundValue

Example:
{
  "id": "profile-avatar",
  "style": { "className": "w-24 h-24 rounded-full" },
  "component": {
    "type": "image",
    "src": { "path": "$.user.avatar" },
    "alt": { "literalString": "User avatar" },
    "fit": { "literalString": "cover" }
  }
}"#
        .to_string(),

        "icon" => r#"Icon - Lucide icon
Properties:
- type: "icon" (required)
- name: BoundValue (string) - Lucide icon name (e.g., "user", "settings", "chevron-right")
- size: BoundValue - "xs" | "sm" | "md" | "lg" | "xl" or number
- color: BoundValue (string) - Tailwind color class
- strokeWidth: BoundValue (number)

Example:
{
  "id": "settings-icon",
  "style": { "className": "text-muted-foreground" },
  "component": {
    "type": "icon",
    "name": { "literalString": "settings" },
    "size": { "literalString": "md" }
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
- actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]; interactions fire with _action_context { interaction: "create"|"move"|"resize"|"open"|"delete", id?, start, end, ... }

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
- actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]; interactions fire with _action_context { interaction: "create"|"move"|"resize"|"open"|"delete"|"link", id?, start?, end?, fromId?, toId? }

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
- label: BoundValue (string)
- disabled: BoundValue (boolean)
- indeterminate: BoundValue (boolean)
- Bind checked with a path to persist toggles. Optional on-change actions use the button's workflow_event contract; the event reads the current state with Get Element Value instead of a pushed payload

Example:
{
  "id": "terms-checkbox",
  "component": {
    "type": "checkbox",
    "checked": { "path": "$.form.acceptTerms" },
    "label": { "literalString": "I accept the terms and conditions" }
  }
}"#
        .to_string(),

        "switch" => r#"Switch - Toggle switch
Properties:
- type: "switch" (required)
- checked: BoundValue (boolean)
- label: BoundValue (string)
- disabled: BoundValue (boolean)
- Bind checked with a path to persist toggles. Optional on-change actions use the button's workflow_event contract; the event reads the current state with Get Element Value instead of a pushed payload

Example:
{
  "id": "notifications-switch",
  "component": {
    "type": "switch",
    "checked": { "path": "$.settings.notifications" },
    "label": { "literalString": "Enable notifications" }
  }
}"#
        .to_string(),

        "tabs" => r#"Tabs - Tabbed content container
Properties:
- type: "tabs" (required)
- value: BoundValue - Active tab id
- tabs: raw array of { id: string, label: BoundValue, icon?: BoundValue, disabled?: BoundValue, contentComponentId: string }
  contentComponentId references the component rendered as that tab's panel — without it the tab is empty
- orientation: BoundValue - "horizontal" | "vertical"
- variant: BoundValue - "default" | "pills" | "underline"

Example:
{
  "id": "settings-tabs",
  "component": {
    "type": "tabs",
    "value": { "path": "$.ui.activeTab", "defaultValue": "general" },
    "tabs": [
      { "id": "general", "label": { "literalString": "General" }, "icon": { "literalString": "settings" }, "contentComponentId": "general-panel" },
      { "id": "security", "label": { "literalString": "Security" }, "icon": { "literalString": "shield" }, "contentComponentId": "security-panel" }
    ]
  }
}"#
        .to_string(),

        "modal" => r#"Modal - Dialog overlay
Properties:
- type: "modal" (required)
- open: BoundValue (boolean) - bind with { "path": "$.ui.showConfirm" } so closing persists
- title: BoundValue
- description: BoundValue (optional)
- closeOnOverlay / closeOnEscape / showCloseButton / centered: BoundValue (boolean)
- size: BoundValue - "sm" | "md" | "lg" | "xl" | "full"
- children: { explicitList: [...] }
- Optional actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }] fire when the modal closes

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Component types with a detailed schema page (must match the fallback
    /// message in `get_component_schema`).
    const DETAILED_PAGE_TYPES: &[&str] = &[
        "column",
        "row",
        "grid",
        "text",
        "button",
        "feedback",
        "appLink",
        "card",
        "userProfile",
        "textField",
        "select",
        "image",
        "icon",
        "diffView",
        "calendar",
        "gantt",
        "checkbox",
        "switch",
        "tabs",
        "modal",
    ];

    const STYLE_CATEGORIES: &[&str] = &[
        "spacing",
        "colors",
        "effects",
        "layout",
        "responsive",
        "typography",
    ];

    /// Extract the trailing `Example:` JSON object from a schema page.
    fn example_json(page: &str) -> serde_json::Value {
        let example = page
            .split("Example:")
            .nth(1)
            .expect("page must contain an Example section");
        let start = example.find('{').expect("example must contain JSON");
        serde_json::from_str(example[start..].trim())
            .expect("example JSON must parse — the model copies it verbatim")
    }

    /// Props whose runtime contract is a raw (non-BoundValue) structure.
    fn raw_struct_prop(component_type: &str, key: &str) -> bool {
        matches!((component_type, key), ("tabs", "tabs"))
    }

    #[test]
    fn every_detailed_page_exists_and_fallback_lists_them_all() {
        let fallback = get_component_schema("definitely-not-a-component");
        assert!(fallback.starts_with("No detailed schema page"));
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            assert!(
                !page.starts_with("No detailed schema page"),
                "{component_type} must have a detailed page"
            );
            assert!(
                fallback.contains(component_type),
                "fallback message must list {component_type}"
            );
        }
        for category in STYLE_CATEGORIES {
            let doc = get_style_examples(category);
            assert!(
                !doc.starts_with("Unknown style category"),
                "{category} must have style examples"
            );
        }
    }

    #[test]
    fn pages_document_actions_as_name_context_arrays() {
        // The runtime contract is `component.actions: [{ "name": ..., "context": {...} }]`
        // (ComponentBase in packages/ui/components/a2ui/types.ts). The legacy
        // event-keyed object shape is silently dropped by serde and rejected by
        // the emit_ui validator.
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            for legacy in ["onClick", "onChange", "onClose", "\"emit\"", "\"update\""] {
                assert!(
                    !page.contains(legacy),
                    "{component_type} page still documents the legacy '{legacy}' action shape"
                );
            }
            if page.contains("actions:") {
                assert!(
                    page.contains(r#"[{ "name""#),
                    "{component_type} page must document actions as an array of {{name, context}}"
                );
            }
        }
    }

    #[test]
    fn example_component_props_are_bound_values() {
        // The emit_ui validator rejects bare string/number/bool values for
        // known props; every example must model the BoundValue discipline.
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            let example = example_json(&page);
            let component = example
                .get("component")
                .and_then(|c| c.as_object())
                .unwrap_or_else(|| panic!("{component_type} example must have a component object"));
            for (key, value) in component {
                if key == "type" || raw_struct_prop(component_type, key) {
                    continue;
                }
                if key == "actions" {
                    let actions = value
                        .as_array()
                        .unwrap_or_else(|| panic!("{component_type}: actions must be an array"));
                    for action in actions {
                        assert!(
                            action.get("name").and_then(|n| n.as_str()).is_some(),
                            "{component_type}: every example action needs a name"
                        );
                        assert!(
                            action.get("context").map(|c| c.is_object()) == Some(true),
                            "{component_type}: every example action needs a context object"
                        );
                    }
                    continue;
                }
                assert!(
                    value.is_object() || value.is_array(),
                    "{component_type}: example prop '{key}' is a bare value — wrap it as a BoundValue"
                );
            }
        }
    }

    #[test]
    fn example_actions_live_inside_the_component_object() {
        // SurfaceComponent (Rust + TS) has no top-level `actions` field; an
        // example placing actions next to `component` teaches a shape that
        // serde silently drops.
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            let example = example_json(&page);
            assert!(
                example.get("actions").is_none(),
                "{component_type}: example puts 'actions' outside 'component' where it is silently dropped"
            );
        }
    }

    #[test]
    fn card_page_matches_component_contract() {
        // CardComponent (types.ts) has no headerActions, and footer is a
        // BoundValue, not a child-id list.
        let page = get_component_schema("card");
        assert!(
            !page.contains("headerActions"),
            "card page advertises 'headerActions', which the renderer and validators reject"
        );
        assert!(
            !page.contains("footer: { explicitList"),
            "card.footer is a BoundValue, not an explicitList"
        );
    }

    #[test]
    fn tabs_page_matches_tab_definition_contract() {
        // TabDefinition (types.ts) is { id, label: BoundValue, icon?, disabled?,
        // contentComponentId } — tab content is referenced per tab, not matched
        // by children order, and tabs have `id`, not `value`.
        let page = get_component_schema("tabs");
        assert!(
            page.contains("contentComponentId"),
            "tabs page must document contentComponentId — without it tabs render empty"
        );
        let example = example_json(&page);
        let tabs = example["component"]["tabs"]
            .as_array()
            .expect("tabs example must include a tabs array");
        for tab in tabs {
            assert!(tab.get("id").is_some(), "each example tab needs an id");
            assert!(
                tab.get("contentComponentId")
                    .and_then(|v| v.as_str())
                    .is_some(),
                "each example tab needs a contentComponentId"
            );
            assert!(
                tab.get("label").map(|l| l.is_object()) == Some(true),
                "tab labels are BoundValues"
            );
        }
    }

    #[test]
    fn action_examples_use_the_workflow_event_contract() {
        // ActionHandler.tsx only wires "workflow_event" (context.nodeId),
        // "navigate_page" (context.route) and "external_link" (context.url) to
        // real behavior; any other name falls through to a no-op userAction.
        // The event body must fetch element state itself (Get Element Value /
        // Get File Input Files), so a workflow_event context carries routing
        // ids only — never element values or payloads.
        const BUILTIN_ACTIONS: &[&str] = &["workflow_event", "navigate_page", "external_link"];
        const WORKFLOW_EVENT_CONTEXT_KEYS: &[&str] = &["nodeId", "boardId", "appId"];
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            if page.contains("actions:") {
                assert!(
                    page.contains("workflow_event"),
                    "{component_type} page documents actions without the workflow_event contract"
                );
            }
            let example = example_json(&page);
            let Some(actions) = example["component"]["actions"].as_array() else {
                continue;
            };
            for action in actions {
                let name = action["name"].as_str().unwrap_or_default();
                assert!(
                    BUILTIN_ACTIONS.contains(&name),
                    "{component_type}: example action '{name}' is not a built-in action name"
                );
                if name == "workflow_event" {
                    let context = action["context"].as_object().unwrap_or_else(|| {
                        panic!("{component_type}: workflow_event needs a context object")
                    });
                    assert!(
                        context.get("nodeId").and_then(|v| v.as_str()).is_some(),
                        "{component_type}: workflow_event context must carry nodeId"
                    );
                    for key in context.keys() {
                        assert!(
                            WORKFLOW_EVENT_CONTEXT_KEYS.contains(&key.as_str()),
                            "{component_type}: workflow_event context key '{key}' pushes payload data — events fetch element state themselves"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn catalog_documents_every_interactive_component_and_how_to_choose() {
        // The historical failure: voiceInput (and feedback/appLink/geoMap) were absent from the
        // docs entirely, so the model imitated them from generic parts. Every registered
        // interactive type must appear, and the selection table must map intents to types.
        let docs = crate::a2ui::copilot::get_full_documentation();
        for component_type in [
            "`button`",
            "`textField`",
            "`select`",
            "`slider`",
            "`checkbox`",
            "`switch`",
            "`radioGroup`",
            "`dateTimeInput`",
            "`fileInput`",
            "`imageInput`",
            "`voiceInput`",
            "`feedback`",
            "`appLink`",
            "`link`",
            "`geoMap`",
        ] {
            assert!(
                docs.contains(component_type),
                "component docs must list {component_type}"
            );
        }
        assert!(docs.contains("## Choosing the Right Component"));
        assert!(docs.contains("push-to-talk"));
        assert!(docs.contains("## Voice Input (voiceInput)"));
        assert!(docs.contains("\"type\": \"voiceInput\""));
        assert!(docs.contains("multiline"));
        assert!(docs.contains("Never invent a type"));
    }

    #[test]
    fn style_guide_teaches_design_reflection_and_reliable_channels() {
        let docs = crate::a2ui::copilot::get_full_documentation();
        assert!(docs.contains("## Design Reflection (BEFORE emitting)"));
        assert!(docs.contains("Signature moment"));
        assert!(docs.contains("NO runtime Tailwind engine"));
        assert!(docs.contains("responsiveOverrides"));
        assert!(docs.contains("## Typography (three real families already exist - use them)"));
        assert!(docs.contains("var(--primary)"));
        assert!(docs.contains("Never style\n   `:root`") || docs.contains("Never style `:root`"));
        assert!(docs.contains("mobile-first"));
    }

    #[test]
    fn style_guide_offers_distinct_directions_and_the_real_type_roles() {
        // Diversity here cannot ride on hue (--primary is fixed by the app theme and hardcoded
        // palette classes break dark mode), so the guide must supply structural directions and
        // the three font families the theme actually ships.
        let docs = crate::a2ui::copilot::get_full_documentation();
        assert!(docs.contains("## Worked Direction Recipes"));
        for direction in [
            "### INSTRUMENT",
            "### LEDGER",
            "### LUMEN",
            "### ATELIER",
            "### BLUEPRINT",
            "### MARQUEE",
        ] {
            assert!(
                docs.contains(direction),
                "missing direction recipe: {direction}"
            );
        }
        for family in ["var(--font-serif)", "var(--font-mono)", "var(--font-sans)"] {
            assert!(docs.contains(family), "type roles must name {family}");
        }
        assert!(docs.contains("`text-5xl`/`text-6xl` are NOT\n  compiled"));
        assert!(docs.contains("fp-design: macro="));
    }

    #[test]
    fn style_guide_does_not_prescribe_the_defaults_the_contract_bans() {
        // The guide used to ship the exact recipes the design contract lists as banned tells,
        // so the catalog and the guidance contradicted each other inside one prompt.
        let docs = crate::a2ui::copilot::get_full_documentation();
        for slop in [
            "from-primary to-purple-500",
            "border-l-4 border-primary pl-4",
            "linear-gradient(135deg, var(--primary) 0%, purple 100%)",
            "rounded-full bg-primary/10 text-primary px-3 py-1",
        ] {
            assert!(
                !docs.contains(slop),
                "component docs still prescribe a banned default: {slop}"
            );
        }
        // A blanket hue scan cannot work here: the guide legitimately QUOTES `bg-[#ff00aa]` and
        // `bg-white` as counter-examples, and "purples" is a Nivo palette name. So assert on the
        // recipe forms above, and that every custom color in the guide is theme-derived.
        let style_guide = crate::a2ui::copilot::get_documentation_section("style")
            .expect("style section must exist");
        assert!(style_guide.contains("color-mix(in oklab, var(--"));
        assert!(style_guide.contains("NEVER hardcoded palette classes"));
        assert!(
            !style_guide.contains("rgba(0, 0, 0,"),
            "style guide still ships a non-theme shadow color"
        );
    }

    #[test]
    fn catalog_points_display_updates_at_element_setters_not_data_update() {
        // The product contract: workflow-driven display changes go through
        // element-level setters; Data Update is a last resort for `$.data.*`
        // bindings. Every doc mention of a2uiDataUpdate must carry that warning.
        let docs = crate::a2ui::copilot::get_full_documentation();
        for alias in [
            "a2uiSetElementText",
            "a2uiSetElementValue",
            "a2uiWriteCsvToTable",
            "a2uiUpdateTable",
            "a2uiPushCsvToChart",
            "a2uiGetElement",
            "a2uiGetElementValue",
            "a2uiGetFileInputFiles",
            "a2uiSetElementAction",
        ] {
            assert!(
                docs.contains(alias),
                "component docs must reference the element-level node {alias}"
            );
        }
        let mentions = docs
            .lines()
            .filter(|line| line.contains("a2uiDataUpdate"))
            .collect::<Vec<_>>();
        assert!(
            !mentions.is_empty(),
            "docs must warn about a2uiDataUpdate explicitly"
        );
        for line in mentions {
            assert!(
                line.contains("never") || line.contains("not "),
                "a2uiDataUpdate may only appear in a discouraging context, found: {line}"
            );
        }
    }

    #[test]
    fn docs_contain_no_email_addresses_or_private_hosts() {
        let mut all_docs = crate::a2ui::copilot::get_full_documentation();
        for component_type in DETAILED_PAGE_TYPES {
            all_docs.push_str(&get_component_schema(component_type));
        }
        for category in STYLE_CATEGORIES {
            all_docs.push_str(&get_style_examples(category));
        }
        for token in all_docs.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')')) {
            let Some(at) = token.find('@') else { continue };
            if at == 0 {
                continue; // CSS at-rules (@keyframes, @media) and decorators
            }
            let domain = &token[at + 1..];
            let looks_like_email = domain.contains('.')
                && domain
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
            assert!(
                !looks_like_email || domain.ends_with("example.com"),
                "docs contain a non-example email/host: {token}"
            );
        }
    }
}
