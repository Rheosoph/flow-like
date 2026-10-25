//! A2UI Copilot helper functions for component documentation
//!
//! Provides component schema docs and style examples used by the system prompt.

// ============================================================================
// Helper Functions
// ============================================================================

/// Get component schema documentation. Pages use the plain authoring form: flat components,
/// plain JSON values, `{ "path": ... }` bindings and child id arrays.
pub fn get_component_schema(component_type: &str) -> String {
    match component_type.to_lowercase().as_str() {
        "column" => r#"Column - Vertical flex container
Properties:
- type: "column" (required)
- gap: string - Space between children (e.g. "16px")
- align: "start" | "center" | "end" | "stretch" | "baseline"
- justify: "start" | "center" | "end" | "between" | "around" | "evenly"
- wrap: boolean - Whether children can wrap
- children: array of child component ids

Example:
{
  "id": "main-column",
  "type": "column",
  "gap": "16px",
  "style": { "className": "p-4" },
  "children": ["header", "content", "footer"]
}"#
        .to_string(),

        "row" => r#"Row - Horizontal flex container
Properties:
- type: "row" (required)
- gap: string - Space between children
- align: "start" | "center" | "end" | "stretch" | "baseline"
- justify: "start" | "center" | "end" | "between" | "around" | "evenly"
- wrap: boolean
- children: array of child component ids

Example:
{
  "id": "button-row",
  "type": "row",
  "gap": "8px",
  "justify": "end",
  "children": ["cancel-btn", "submit-btn"]
}"#
        .to_string(),

        "grid" => r#"Grid - CSS Grid container
Properties:
- type: "grid" (required)
- columns: string (e.g. "repeat(3, 1fr)")
- rows: string (optional)
- gap: string
- autoFlow: "row" | "column" | "dense" | "rowDense" | "columnDense"
- children: array of child component ids

Example:
{
  "id": "card-grid",
  "type": "grid",
  "columns": "repeat(auto-fill, minmax(250px, 1fr))",
  "gap": "16px",
  "children": ["card-1", "card-2", "card-3"]
}"#
        .to_string(),

        "text" => r#"Text - Text display component
Properties:
- type: "text" (required)
- content: string, or { "path": "$.data.title" } to bind data (required)
- variant: "body" | "heading" | "label" | "caption" | "code"
- size: "xs" | "sm" | "md" | "lg" | "xl" | "2xl" | "3xl" | "4xl"
- weight: "light" | "normal" | "medium" | "semibold" | "bold"
- color: string (Tailwind color like "text-primary")
- align: "left" | "center" | "right" | "justify"
- truncate: boolean; maxLines: number

Example:
{
  "id": "page-title",
  "type": "text",
  "content": "Welcome",
  "variant": "heading",
  "size": "3xl",
  "weight": "bold",
  "style": { "className": "text-primary" }
}"#
        .to_string(),

        "button" => r#"Button - Interactive button
Properties:
- type: "button" (required)
- label: string - Button text (required)
- variant: "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
- size: "sm" | "md" | "lg" | "icon"
- disabled: boolean
- loading: boolean - Shows loading spinner when true
- icon: string - Lucide icon name (e.g., "send", "plus", "trash")
- iconPosition: "left" | "right" (default: "left")
- tooltip: string - Tooltip text on hover
- actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }] - legacy default; only actions[0] fires when no named click handler exists
- eventHandlers: { "click": [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }] } - optional ordered actions for this exact component event

Action wiring (same contract for every interactive component):
- "workflow_event" invokes ONE named board event; context carries routing ids ONLY (nodeId, optional boardId/appId)
- NEVER copy element/dashboard values into the context - the event body reads current element state itself at runtime (Get Element -> Get Element Value / Get File Input Files)
- Other built-in names: "navigate_page" (context.route, optional context.queryParams), "external_link" (context.url) and "widget_event" (context.actionId, inside a widget only)
- The action "name" is one of those FIXED verbs - never a board node name, event name or widget action id; those go in the context. An unrecognized name is dropped at runtime (the control renders and does nothing) and emit_ui rejects it
- Exact eventHandlers entries override the legacy default; an explicit [] disables that event
- Events added after a component shipped (textField "input"/"submit"/"focus"/"blur", slider "input", select "open"/"close", table "rowClick"/"cellClick"/"selectionChange"/"sortChange", chart "pointClick", richText "change"/"blur"/"imageUploaded"/"imageUploadError", excalidraw "load"/"save"/"autoSave"/"dirtyChange"/"error", cameraView "ready"/"capture"/"frame"/"error"/"overlayClick") need an EXACT entry - they ignore both actions[0] and "*"
- A board can set or re-point the legacy default or a named event later with Set Element Action (a2uiSetElementAction; optional event_name)

Example:
{
  "id": "submit-btn",
  "type": "button",
  "label": "Submit",
  "variant": "default",
  "icon": "send",
  "iconPosition": "left",
  "actions": [{ "name": "workflow_event", "context": { "nodeId": "evt-submit-form" } }]
}"#
        .to_string(),

        "feedback" => r#"Feedback - Built-in thumbs up/down feedback control
Properties:
- type: "feedback" (required)
- mode: "icon" | "compact" | "segmented" | "rating" | "extended"
- size: "sm" | "md" | "lg"
- title: string
- positiveLabel / negativeLabel: string
- showComment: boolean
- commentMode: "none" | "inline" | "modal" (use "modal" to keep compact/icon controls simple while collecting a comment)
- commentTitle / commentDescription / commentSubmitLabel / commentCancelLabel: string
- feedbackId: string (optional stable id)
- includeState: boolean (include component/page state in feedback context)
- pageContextMode: "none" | "path" | "query" (default "path"; "query" sends query params)
- pageContextQueryParamAllowlist / pageContextQueryParamDenylist: string (comma-separated query param names)
- includePageHash: boolean (default false)

Example:
{
  "id": "page-feedback",
  "type": "feedback",
  "mode": "segmented",
  "title": "Was this helpful?",
  "showComment": true,
  "commentMode": "modal",
  "pageContextMode": "path"
}"#
        .to_string(),

        "applink" | "app_link" => r#"AppLink - Built-in link button to app shell screens
Properties:
- type: "appLink" (required)
- target: "config" | "settings" | "overview"
- label: string (optional; defaults from target)
- variant: "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
- size: "sm" | "md" | "lg" | "icon"
- icon: string (Lucide icon name)

Example:
{
  "id": "settings-link",
  "type": "appLink",
  "target": "settings"
}"#
        .to_string(),

        "card" => r#"Card - Content container with optional header/footer
Properties:
- type: "card" (required)
- title: string (optional)
- description: string (optional)
- footer: string (optional footer text)
- variant: "default" | "bordered" | "elevated"
- hoverable / clickable: boolean
- children: array of child component ids

Example:
{
  "id": "user-card",
  "type": "card",
  "title": "User Profile",
  "style": { "className": "p-6" },
  "children": ["avatar", "user-info"]
}"#
        .to_string(),

        "userprofile" | "user_profile" => r#"UserProfile - Fetch and display a Flow-Like user by subject/sub ID
Properties:
- type: "userProfile" (required)
- value: string - user subject/sub ID (required). Compatible with Set Element Value. The "local" sub of an unauthenticated execution renders the current user.
- variant: "avatar" | "chip" | "row" | "detailed" | "card"
- avatarSize: "xs" | "sm" | "md" | "lg" | "xl" | "2xl"
- showHover: boolean - enable hover details
- showEmail: boolean
- showDescription: boolean
- showUserId: boolean
- showProfileLink: boolean
- fallbackLabel: string

Example:
{
  "id": "assignee-profile",
  "type": "userProfile",
  "value": { "path": "$.assigneeSub" },
  "variant": "row",
  "showHover": true
}"#
        .to_string(),

        "textfield" | "text_field" => r#"TextField - Text input
Properties:
- type: "textField" (required)
- value: string, or { "path": "$.form.field" } to persist edits (required)
- placeholder: string
- inputType: "text" | "email" | "password" | "number" | "tel" | "url" | "search"
- multiline: boolean
- rows: number (for multiline)
- disabled: boolean
- error: string (error message)
- label: string
- debounceMs: number - pause before the "input" event fires (default 400, min 100)
- Bind value with { "path": "$.form.email" } to persist edits. Optional on-change actions use the button's workflow_event contract; the event reads the current value with Get Element Value instead of a pushed payload

Example:
{
  "id": "email-input",
  "type": "textField",
  "value": { "path": "$.form.email" },
  "placeholder": "Enter email",
  "inputType": "email",
  "label": "Email Address"
}"#
        .to_string(),

        "richtext" | "rich_text" => r#"RichText - Formatted document editor
Properties:
- type: "richText" (required)
- value: the document, a "plate_json::"-prefixed string (required). NOT markdown: convert with the Rich Text to Markdown (utils_md_plate_to_md) or Rich Text to HTML (utils_md_plate_to_html) node
- documentId: string - stable article identity, included in document and upload event context
- documentRevision: string or number - authoritative revision; changing it replaces local edits with value. Return it with saves to reject stale writes
- currentUser: object - reviewer identity { id, name, avatarUrl? }, supplied by the authenticated host
- reviewEnabled: boolean - enable document comments and tracked suggestions for the current reviewer
- label / helperText / placeholder: string
- readOnly / disabled: boolean
- error: boolean
- uploadPrefix: string - storage folder for pasted or dropped images (default "a2ui/<surface>/<component>")
- uploadScope: "app" (shared, default) or "user" (the viewer's private area)
- minHeight / maxHeight: string - CSS lengths
- debounceMs: number - pause before "change" fires (default 600, min 100)
- Images are uploaded into app storage and stored in the document as durable "storage://..." paths, so the document keeps working after signed URLs expire
- Events: "change", "blur", "imageUploaded", "imageUploadError" - all need EXACT eventHandlers entries

Example:
{
  "id": "article-body",
  "type": "richText",
  "value": { "path": "$.article.body" },
  "label": "Article",
  "uploadPrefix": "articles/images",
  "eventHandlers": {
    "change": [{ "name": "workflow_event", "context": { "nodeId": "<save-draft-event-id>" } }]
  }
}"#
        .to_string(),

        "select" => r#"Select - Dropdown selection
Properties:
- type: "select" (required)
- value: string, or { "path": "$.form.choice" } to persist the selection (required)
- options: [{ "value": "...", "label": "..." }] or { "path": "$.data.options" } (required)
- placeholder: string
- label: string
- disabled: boolean
- Bind value with a path to persist the selection. Optional on-change actions use the button's workflow_event contract; the event reads the current selection with Get Element Value instead of a pushed payload

Example:
{
  "id": "country-select",
  "type": "select",
  "value": { "path": "$.form.country" },
  "placeholder": "Select country",
  "options": [
    { "value": "us", "label": "United States" },
    { "value": "uk", "label": "United Kingdom" }
  ]
}"#
        .to_string(),

        "image" => r#"Image - Image display
Properties:
- type: "image" (required)
- src: string - Image URL (required)
- alt: string - Alt text (recommended for accessibility)
- fit: "contain" | "cover" | "fill" | "none" | "scaleDown"
- loading: "lazy" | "eager"
- fallback: string - Fallback image URL

Example:
{
  "id": "profile-avatar",
  "type": "image",
  "src": { "path": "$.user.avatar" },
  "alt": "User avatar",
  "fit": "cover",
  "style": { "className": "w-24 h-24 rounded-full" }
}"#
        .to_string(),

        "icon" => r#"Icon - Lucide icon
Properties:
- type: "icon" (required)
- name: string - Lucide icon name (e.g., "user", "settings", "chevron-right") (required)
- size: "xs" | "sm" | "md" | "lg" | "xl" or a number
- color: string - Tailwind color class
- strokeWidth: number

Example:
{
  "id": "settings-icon",
  "type": "icon",
  "name": "settings",
  "size": "md",
  "style": { "className": "text-muted-foreground" }
}"#
        .to_string(),

        "diffview" | "diff_view" => r#"DiffView - Side-by-side, unified or inline diff for text, code, markdown & documents
Properties:
- type: "diffView" (required)
- original: string (required) - Left/old content or document URL
- modified: string (required) - Right/new content or document URL
- mode: "split" | "unified" | "inline" (default "split")
- kind: "auto" | "text" | "code" | "markdown" | "json" | "document" (default "auto")
- language: string - Syntax language e.g. "typescript", "rust", "json", "markdown", "python" (default "plaintext")
- markdownMode: "source" | "rendered" (default "source")
- showLineNumbers: boolean (default true)
- wordWrap: boolean (default false)
- wordLevel: boolean (default true)
- collapseUnchanged: boolean (default false)
- contextLines: number (default 3)
- showStats: boolean (default true)
- originalLabel: string (default "Original")
- modifiedLabel: string (default "Modified")
- ignoreWhitespace: boolean (default false)
- ignoreCase: boolean (default false)
- trimTrailingWhitespace: boolean (default false)
- swapSides: boolean (default false)

Example:
{
  "id": "config-diff",
  "type": "diffView",
  "original": "const port = 3000",
  "modified": "const port = 8080",
  "mode": "split",
  "kind": "code",
  "language": "typescript",
  "showLineNumbers": true
}"#
        .to_string(),

        "calendar" => r#"Calendar - Interactive calendar for viewing & scheduling events
Properties:
- type: "calendar" (required)
- events: array of CalendarEvent { id, title, start, end?, allDay?, color?, description?, location?, calendarId?, editable?, link?, metadata? } (required) (link: relative path navigates in-app, absolute URL opens externally)
- view: "month" | "week" | "day" | "agenda" (default "month")
- date: string (ISO 8601) - Focused date
- editable: boolean (default true) - Allow drag/resize
- selectable: boolean (default true) - Allow slot selection to create
- firstDayOfWeek: number (0=Sunday, default 1)
- minTime / maxTime: string ("HH:MM") - Time-grid bounds
- slotDuration: number (minutes, default 30)
- showWeekends / showNowIndicator / showAllDay: boolean
- locale: string (e.g. "en-US")
- title: string - Header title
- density: "compact" | "default" | "comfortable"
- showViewSwitcher: boolean (default true)
- height: string (CSS value e.g. "600px")
- responsive: boolean - auto agenda on narrow widths
- compactBreakpoint: number (px)
- actions: legacy default action, same contract as button - [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]; interactions fire with _action_context { interaction: "create"|"update"|"move"|"resize"|"open"|"delete", id?, start, end, ... }
- eventHandlers: optional ordered action lists keyed by "create" | "update" | "move" | "resize" | "open" | "delete"

Example:
{
  "id": "planner",
  "type": "calendar",
  "events": { "path": "$.events" },
  "view": "week",
  "editable": true
}"#
        .to_string(),

        "gantt" => r#"Gantt - Interactive Gantt timeline for planning tasks
Properties:
- type: "gantt" (required)
- tasks: array of GanttTask { id, name, start, end, progress?, dependencies?, parent?, color?, assignee?, milestone?, collapsed?, link?, metadata? } (required) (link: relative path navigates in-app, absolute URL opens externally)
- view: "day" | "week" | "month" | "quarter" | "compact" (default "week")
- editable: boolean (default true) - Master switch for drag/resize/link
- draggable / resizable: boolean - Fine-grained edit controls
- showDependencies: boolean (default true) - Draw dependency arrows
- showProgress: boolean (default true) - Bar progress fill
- showToday: boolean (default true) - Today marker line
- rowHeight: number (px)
- columns: array of string - Extra left-panel columns e.g. ["assignee","progress"]
- title: string - Header title
- density: "compact" | "default" | "comfortable"
- showViewSwitcher: boolean (default true)
- showTaskList: boolean (default true) - Left task-list panel
- taskListWidth: number (px)
- shadeWeekends: boolean (default true)
- height: string (CSS value e.g. "600px")
- responsive: boolean - auto compact on narrow widths
- compactBreakpoint: number (px)
- actions: legacy default action, same contract as button - [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]; interactions fire with _action_context { interaction: "create"|"update"|"move"|"resize"|"open"|"delete"|"link"|"reorder", id?, start?, end?, fromId?, toId? }
- eventHandlers: optional ordered action lists keyed by "create" | "update" | "move" | "resize" | "open" | "delete" | "link" | "reorder"

Example:
{
  "id": "roadmap",
  "type": "gantt",
  "tasks": { "path": "$.tasks" },
  "view": "week",
  "showDependencies": true
}"#
        .to_string(),

        "checkbox" => r#"Checkbox - Boolean toggle with label
Properties:
- type: "checkbox" (required)
- checked: boolean, or { "path": "$.form.flag" } to persist toggles (required)
- label: string
- disabled: boolean
- Bind checked with a path to persist toggles. Optional on-change actions use the button's workflow_event contract; the event reads the current state with Get Element Value instead of a pushed payload

Example:
{
  "id": "terms-checkbox",
  "type": "checkbox",
  "checked": { "path": "$.form.acceptTerms" },
  "label": "I accept the terms and conditions"
}"#
        .to_string(),

        "switch" => r#"Switch - Toggle switch
Properties:
- type: "switch" (required)
- checked: boolean, or { "path": "$.settings.flag" } to persist toggles (required)
- label: string
- disabled: boolean
- Bind checked with a path to persist toggles. Optional on-change actions use the button's workflow_event contract; the event reads the current state with Get Element Value instead of a pushed payload

Example:
{
  "id": "notifications-switch",
  "type": "switch",
  "checked": { "path": "$.settings.notifications" },
  "label": "Enable notifications"
}"#
        .to_string(),

        "tabs" => r#"Tabs - Tabbed content container
Properties:
- type: "tabs" (required)
- value: string - Active tab id (required; bind it with { "path": ... } to persist the choice)
- tabs: array of { id: string, label: string, icon?: string, disabled?: boolean, contentComponentId: string } (required)
  contentComponentId references the component rendered as that tab's panel — without it the tab is empty
- orientation: "horizontal" | "vertical"
- variant: "default" | "pills" | "underline"

Example:
{
  "id": "settings-tabs",
  "type": "tabs",
  "value": { "path": "$.ui.activeTab", "defaultValue": "general" },
  "tabs": [
    { "id": "general", "label": "General", "icon": "settings", "contentComponentId": "general-panel" },
    { "id": "security", "label": "Security", "icon": "shield", "contentComponentId": "security-panel" }
  ]
}"#
        .to_string(),

        "forecastchart" => r#"ForecastChart - Saved Data Studio prediction
Properties:
- type: "forecastChart" (required)
- forecastId: string (required) - Existing saved prediction ID; choose it in the editor or bind a known ID
- appId: string - Owning project; defaults to the running project
- userScoped: boolean - Use the current viewer's personal database (default false); project scope supports collaborator access
- target / seriesId: string - Initially selected target and series
- view: "forecast" for outlook or "evaluation" for saved holdout results
- showCovariates / showMetrics / showControls / showHeader: boolean
- compact: boolean - Compact layout for small dashboard cards
- height: number - Plot height in pixels, clamped to 120 through 1200

The viewer's current database and source-column permissions apply. This component reads a saved result and never runs inference. Do not invent a forecast ID or put fetched chart values into page state. It emits no workflow events.

Example:
{
  "id": "saved-sales-outlook",
  "type": "forecastChart",
  "forecastId": { "path": "$.data.savedForecastId" },
  "view": "forecast",
  "showMetrics": true,
  "height": 320
}"#
        .to_string(),

        "modal" => r#"Modal - Dialog overlay
Properties:
- type: "modal" (required)
- open: boolean - bind with { "path": "$.ui.showConfirm" } so closing persists (required)
- title: string
- description: string (optional)
- closeOnOverlay / closeOnEscape / showCloseButton / centered: boolean
- size: "sm" | "md" | "lg" | "xl" | "full"
- children: array of child component ids
- Optional actions: [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }] fire when the modal closes

Example:
{
  "id": "confirm-modal",
  "type": "modal",
  "open": { "path": "$.ui.showConfirm" },
  "title": "Confirm Action",
  "children": ["modal-content", "modal-actions"]
}"#
        .to_string(),

        _ => format!(
            "No detailed schema page for component type: {}. Detailed pages exist for: column, row, grid, text, button, feedback, appLink, card, userProfile, textField, richText, select, image, icon, diffView, calendar, gantt, checkbox, switch, tabs, forecastChart, modal (plus style categories spacing, colors, effects, layout, responsive, typography). For every other component, use the component documentation embedded in your system prompt. It is the authoritative reference.",
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
        "richText",
        "select",
        "image",
        "icon",
        "diffView",
        "calendar",
        "gantt",
        "checkbox",
        "switch",
        "tabs",
        "forecastChart",
        "modal",
    ];

    /// Props every component accepts besides its own (the desktop emit_ui validator's base set).
    const BASE_PROPS: &[&str] = &[
        "type",
        "id",
        "style",
        "children",
        "actions",
        "eventHandlers",
        "hidden",
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

    /// The page's example lowered to the canonical shape every renderer and validator reads.
    fn lowered_example(component_type: &str) -> serde_json::Value {
        let example = example_json(&get_component_schema(component_type));
        crate::a2ui::authoring::lower_component(example)
            .unwrap_or_else(|error| panic!("{component_type}: example does not lower: {error}"))
    }

    fn manifest_spec(component_type: &str) -> &'static crate::a2ui::authoring::ComponentSpec {
        crate::a2ui::authoring::component_manifest()
            .component(component_type)
            .unwrap_or_else(|| panic!("{component_type} is not in the component manifest"))
    }

    /// `(prop names, line)` for every `- name:` / `- a / b:` property line of a page.
    fn page_prop_lines(page: &str) -> Vec<(Vec<&str>, &str)> {
        page.lines()
            .filter_map(|line| {
                let (names, _) = line.strip_prefix("- ")?.split_once(':')?;
                let names: Vec<&str> = names.split(" / ").map(str::trim).collect();
                names
                    .iter()
                    .all(|name| {
                        name.starts_with(|c: char| c.is_ascii_lowercase())
                            && name.chars().all(|c| c.is_ascii_alphanumeric())
                    })
                    .then_some((names, line))
            })
            .collect()
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

    /// Substring search that ignores matches inside a longer identifier, so a documented event
    /// name like `selectionChange` is not mistaken for the legacy `onChange` prop.
    fn contains_standalone(haystack: &str, needle: &str) -> bool {
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        let boundary_before = needle.starts_with(is_ident);
        let boundary_after = needle.ends_with(is_ident);
        haystack.match_indices(needle).any(|(index, matched)| {
            let before_ok = !boundary_before
                || haystack[..index]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !is_ident(c));
            let after_ok = !boundary_after
                || haystack[index + matched.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !is_ident(c));
            before_ok && after_ok
        })
    }

    #[test]
    fn legacy_shape_check_ignores_longer_identifiers() {
        assert!(!contains_standalone("\"selectionChange\"", "onChange"));
        assert!(contains_standalone("- onChange: BoundValue", "onChange"));
        assert!(contains_standalone("\"onChange\": []", "onChange"));
    }

    #[test]
    fn pages_document_actions_as_name_context_arrays() {
        // The runtime contract is `component.actions: [{ "name": ..., "context": {...} }]`
        // (ComponentBase in packages/ui/components/a2ui/types.ts). The legacy
        // event-keyed object shape is silently dropped by serde and rejected by
        // the emit_ui validator.
        for component_type in DETAILED_PAGE_TYPES {
            let page = get_component_schema(component_type);
            // Legacy event-keyed props. Matched on identifier boundaries so a documented event
            // name like `selectionChange` is not read as the old `onChange` prop.
            for legacy in ["onClick", "onChange", "onClose"] {
                assert!(
                    !contains_standalone(&page, legacy),
                    "{component_type} page still documents the legacy '{legacy}' action shape"
                );
            }
            // Legacy action names. Checked in action-name position only: calendar/gantt
            // legitimately document "update" as an `_action_context.interaction` value.
            for legacy in ["emit", "update"] {
                assert!(
                    !page.contains(&format!(r#""name": "{legacy}""#)),
                    "{component_type} page still documents '{legacy}' as an action name"
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
    fn examples_are_plain_authored_components_the_validator_accepts() {
        // The model copies these verbatim: each example must be the flat plain-value form, lower
        // without errors and pass the emit_ui prop allowlist (manifest props + base props).
        for component_type in DETAILED_PAGE_TYPES {
            let example = example_json(&get_component_schema(component_type));
            assert_eq!(
                example["type"].as_str(),
                Some(*component_type),
                "{component_type}: example must be a flat {component_type} component"
            );
            assert!(
                example.get("component").is_none() && !example.to_string().contains("\"literal"),
                "{component_type}: example must use plain values, not the nested wrapped form"
            );
            let spec = manifest_spec(component_type);
            let lowered = lowered_example(component_type);
            let component = lowered["component"]
                .as_object()
                .expect("a lowered component is an object");
            for (key, value) in component {
                let prop = spec.prop(key);
                assert!(
                    BASE_PROPS.contains(&key.as_str()) || prop.is_some(),
                    "{component_type}: example prop '{key}' is not a {component_type} prop"
                );
                let Some(prop) = prop.filter(|prop| !prop.values.is_empty()) else {
                    continue;
                };
                if let Some(literal) = value
                    .get("literalString")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| value.as_str())
                {
                    assert!(
                        prop.values.iter().any(|allowed| allowed == literal),
                        "{component_type}: example {key} '{literal}' is not one of {:?}",
                        prop.values
                    );
                }
            }
            for required in spec.props.iter().filter(|prop| prop.required) {
                assert!(
                    component.contains_key(&required.name),
                    "{component_type}: example omits required prop '{}'",
                    required.name
                );
            }
        }
    }

    #[test]
    fn page_prop_lines_match_the_component_manifest() {
        // Hand-written property lists drift: renderer-ignored props get advertised and
        // invented values (text variant "h1") get taught. Every listed prop must exist, none
        // may be unrendered, and every quoted value of a value-constrained prop must be real.
        for component_type in DETAILED_PAGE_TYPES {
            let spec = manifest_spec(component_type);
            let page = get_component_schema(component_type);
            for (names, line) in page_prop_lines(&page) {
                for name in names {
                    if BASE_PROPS.contains(&name) {
                        continue;
                    }
                    let prop = spec.prop(name).unwrap_or_else(|| {
                        panic!("{component_type} page lists '{name}', which is not a prop: {line}")
                    });
                    assert!(
                        !prop.unrendered,
                        "{component_type} page advertises '{name}', which the renderer ignores"
                    );
                    if prop.values.is_empty() {
                        continue;
                    }
                    for quoted in line.split('"').skip(1).step_by(2) {
                        assert!(
                            prop.values.iter().any(|allowed| allowed == quoted),
                            "{component_type}.{name}: '{quoted}' is not one of {:?}",
                            prop.values
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn example_actions_land_inside_the_component_object() {
        // SurfaceComponent (Rust + TS) has no top-level `actions` field. Authored components
        // carry actions next to their props; lowering must move them into `component`.
        for component_type in DETAILED_PAGE_TYPES {
            let authored = example_json(&get_component_schema(component_type));
            let lowered = lowered_example(component_type);
            assert!(
                lowered.get("actions").is_none(),
                "{component_type}: lowered example keeps 'actions' outside 'component'"
            );
            if authored.get("actions").is_some() {
                assert!(
                    lowered["component"]["actions"].is_array(),
                    "{component_type}: example actions were not lowered into 'component'"
                );
            }
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
        // by children order, and tabs have `id`, not `value`. Authored labels are plain
        // strings; lowering wraps them.
        let page = get_component_schema("tabs");
        assert!(
            page.contains("contentComponentId"),
            "tabs page must document contentComponentId — without it tabs render empty"
        );
        let example = example_json(&page);
        let tabs = example["tabs"]
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
                tab.get("label").is_some_and(serde_json::Value::is_string),
                "authored tab labels are plain strings"
            );
        }
        let lowered = lowered_example("tabs");
        for tab in lowered["component"]["tabs"]
            .as_array()
            .expect("lowered tabs stay an array")
        {
            assert!(
                tab["label"].get("literalString").is_some(),
                "lowered tab labels are BoundValues: {tab}"
            );
            assert!(
                tab["contentComponentId"].is_string(),
                "contentComponentId stays a plain id: {tab}"
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
            let lowered = lowered_example(component_type);
            let component = &lowered["component"];
            let handler_lists = component["eventHandlers"]
                .as_object()
                .into_iter()
                .flat_map(|handlers| handlers.values());
            let actions = std::iter::once(&component["actions"])
                .chain(handler_lists)
                .filter_map(serde_json::Value::as_array)
                .flatten();
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
            "`cameraView`",
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
        assert!(docs.contains("compiled at runtime against this theme"));
        assert!(!docs.contains("runtime Tailwind engine"));
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
        assert!(docs.contains("`text-5xl`..`text-9xl` compile"));
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
