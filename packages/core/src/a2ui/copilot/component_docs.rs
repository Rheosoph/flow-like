//! A2UI Component Documentation for AI Copilot
//! This module contains comprehensive documentation for all A2UI components
//! that can be used by the AI copilot to generate UIs.

pub const COMPONENT_CATALOG: &str = r##"
# A2UI Component Catalog

## Quick Reference - All Component Types

### Layout Components
- `column` - Vertical flex container
- `row` - Horizontal flex container
- `grid` - CSS Grid container
- `stack` - Z-axis layering (overlapping elements)
- `scrollArea` - Scrollable container
- `absolute` - Free positioning container
- `aspectRatio` - Maintain aspect ratio
- `overlay` - Position items over base component
- `box` - Generic semantic container
- `center` - Center content
- `spacer` - Flexible/fixed spacing

### Display Components
- `text` - Typography with variants
- `image` - Image display
- `icon` - Lucide icons
- `video` - Video player
- `lottie` - Lottie animations
- `markdown` - Markdown renderer
- `diffView` - Side-by-side, unified or inline diff for text, code, markdown & documents (props: original, modified, mode, kind, language, ...)
- `badge` - Small label/tag
- `avatar` - User avatar
- `userProfile` - User lookup display by sub with avatar, chip, row, detailed, and card variants
- `progress` - Progress bar
- `spinner` - Loading spinner
- `divider` - Visual separator
- `skeleton` - Loading placeholder

### Interactive Components
- `button` - Clickable button
- `textField` - Text input
- `select` - Dropdown selection
- `slider` - Range slider
- `checkbox` - Boolean toggle
- `switch` - Toggle switch
- `radioGroup` - Radio buttons
- `dateTimeInput` - Date/time picker
- `fileInput` - File upload
- `imageInput` - Image upload with preview
- `link` - Navigation link

### Container Components
- `card` - Content card
- `modal` - Dialog overlay
- `tabs` - Tabbed content
- `accordion` - Collapsible sections
- `drawer` - Slide-out panel
- `tooltip` - Hover tooltip
- `popover` - Click popover

### Data Visualization
- `table` - Data table with sorting/pagination
- `plotlyChart` - Plotly.js charts (line, bar, scatter, pie, area, histogram)
- `nivoChart` - Nivo charts (25+ chart types)

### Planning Components
- `calendar` - Interactive calendar (month/week/day/agenda) with detail/edit dialogs and right-click menus; fires create/update/move/resize/open/delete actions (props: events, view, date, title, density, editable, selectable, ...)
- `gantt` - Interactive Gantt timeline with drag/resize/dependency-link, detail/edit dialogs, right-click menus and task-list drag-reordering; fires create/update/move/resize/open/delete/link/reorder actions (props: tasks, view, title, density, editable, showDependencies, showTaskList, ...)

### Media Components
- `iframe` - Embedded external content or HTML preview (supports src URL and srcdoc HTML)
- `filePreview` - Generic file preview

### Computer Vision / ML
- `boundingBoxOverlay` - Display bounding boxes on images
- `imageLabeler` - Draw bounding boxes for labeling
- `imageHotspot` - Interactive clickable hotspots on images

### Game / Interactive Media Components
- `canvas2d` - 2D canvas for sprites/shapes
- `sprite` - 2D sprite with position/rotation
- `shape` - 2D shapes (rect, circle, polygon, etc.)
- `scene3d` - 3D scene container
- `model3d` - 3D model viewer (GLB/GLTF)
- `dialogue` - Visual novel dialogue box
- `characterPortrait` - Character portrait with expressions
- `choiceMenu` - Choice/decision menu
- `inventoryGrid` - Game inventory grid
- `healthBar` - Health/resource bar
- `miniMap` - Mini-map with markers

### Widget System
- `widgetInstance` - Reusable widget component instance
- An INTERACTIVE widget (rows/cards with buttons the user acts on) MUST declare its named actions
  at the WIDGET level inside its `inlineWidgetDef` — a widget with an empty `actions` list cannot
  be bound to any workflow. Use the exact action names the request asks for as the action ids:
  `"inlineWidgetDef": { ..., "actions": [{ "id": "approve", "label": "Approve", "contextSchema": [{ "name": "itemId", "label": "Item Id", "fieldType": "string", "defaultPath": "$.item.id" }] }] }`
  Components INSIDE the widget trigger a declared action by id:
  `"actions": [{ "name": "approve" }]`
  The board binds `eventsWidgetAction` handlers to these declared widget action ids.

## Wiring UI to Workflows

### Actions -> named board events
Interactive components carry actions INSIDE the component object:
`"actions": [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]`
(actions[0] fires on the component's primary interaction).
- Create ONE named board event per purpose (e.g. dashboard-load, add-target, refresh-status).
  Never route several buttons through one generic catch-all event.
- The action context carries routing ids ONLY (nodeId, optional boardId/appId). Do NOT push
  element values, form payloads, or target ids through the context: the event body fetches
  current element state itself at runtime via Get Element (`a2uiGetElement`), Get Element Value
  (`a2uiGetElementValue`), and Get File Input Files (`a2uiGetFileInputFiles`).
- Other built-in action names: "navigate_page" (context.route, optional context.queryParams)
  and "external_link" (context.url).
- A board can set or re-point an element's action later with Set Element Action
  (`a2uiSetElementAction`, action_type "workflow_event" + node_id).

### Displaying workflow data -> element-level setters
When a workflow must change what a component SHOWS, target the element directly:
- text / badge / markdown: Set Element Text (`a2uiSetElementText`), Set Badge Content
  (`a2uiSetBadgeContent`), Set Markdown Content (`a2uiSetMarkdownContent`)
- inputs: Set Element Value (`a2uiSetElementValue`)
- table: Push CSV to Table (`a2uiWriteCsvToTable`) for full loads, Update Table
  (`a2uiUpdateTable`) for incremental row edits
- chart: Push Data to Chart (`a2uiPushCsvToChart`), styled via `a2uiSetNivoConfig` /
  `a2uiSetChartLayout`
- progress: Set Progress (`a2uiSetProgress`)
Data Update (`a2uiDataUpdate`) is almost never the right node for display updates - reach for an
element-level setter first and reserve it for a `$.data.*` binding no setter covers.

"##;

pub const CHART_DOCUMENTATION: &str = r##"
# Chart Components Documentation

Literal `data` props are for static/design-time data only. When a WORKFLOW supplies the data,
push it into the element: Push Data to Chart (`a2uiPushCsvToChart`) for nivoChart/plotlyChart and
Push CSV to Table (`a2uiWriteCsvToTable`) / Update Table (`a2uiUpdateTable`) for table -
not Data Update (`a2uiDataUpdate`).

## Nivo Charts (nivoChart)

Nivo provides 25+ chart types with beautiful defaults and animations.

### Bar Chart
Data format: Array of objects with category field and numeric value fields.

Example data: [{"country": "USA", "sales": 120, "profit": 45}]

Properties:
- chartType: "bar"
- data: Array of category objects
- indexBy: Field to use as category (e.g., "country")
- keys: Array of value field names (e.g., ["sales", "profit"])
- height: Chart height (e.g., "400px")
- colors: Color scheme (e.g., "paired")
- showLegend: Show legend (boolean)
- barStyle: JSON object with groupMode, layout, padding, borderRadius, etc.

Bar Style Options:
- layout: "vertical" or "horizontal"
- groupMode: "grouped" or "stacked"
- padding: 0-1 (space between groups)
- innerPadding: 0-1 (space within groups)
- borderRadius: number (rounded corners)
- enableLabel: boolean
- enableGridX/Y: boolean

### Line Chart
Data format: Array of series objects, each with id and data array of {x, y} points.

Example data: [{"id": "Revenue", "data": [{"x": "Jan", "y": 10}, {"x": "Feb", "y": 15}]}]

Properties:
- chartType: "line"
- data: Array of series
- height: Chart height
- lineStyle: JSON object with curve, enableArea, enablePoints, etc.

Line Style Options:
- curve: "linear", "monotoneX", "natural", "step", "stepBefore", "stepAfter", "basis", "cardinal", "catmullRom"
- lineWidth: number
- enableArea: boolean (fill under line)
- areaOpacity: 0-1
- enablePoints: boolean
- pointSize: number
- enableSlices: "x", "y", or false (crosshair on hover)
- enableCrosshair: boolean

### Pie / Donut Chart
Data format: Array of objects with id and value fields.

Example data: [{"id": "Desktop", "value": 45}, {"id": "Mobile", "value": 35}]

Properties:
- chartType: "pie"
- data: Array of slices
- height: Chart height
- pieStyle: JSON object with innerRadius, padAngle, cornerRadius, etc.

Pie Style Options:
- innerRadius: 0-1 (0 = pie, >0 = donut)
- padAngle: number (gap between slices)
- cornerRadius: number
- startAngle/endAngle: degrees
- sortByValue: boolean
- enableArcLabels: boolean (labels on slices)
- enableArcLinkLabels: boolean (labels with lines)
- activeOuterRadiusOffset: number (hover effect)

### Radar Chart
Data format: Array of dimension objects with category field and numeric series values.

Example data: [{"skill": "JavaScript", "Alice": 90, "Bob": 70}]

Properties:
- chartType: "radar"
- data: Array of dimension objects
- indexBy: Dimension field name
- keys: Array of series names
- radarStyle: JSON object with gridShape, dotSize, fillOpacity, etc.

Radar Style Options:
- gridShape: "circular" or "linear"
- gridLevels: number
- dotSize: number
- enableDots: boolean
- fillOpacity: 0-1
- borderWidth: number

### Heatmap
Data format: Array of row objects, each with id and data array of {x, y} cells.

Example data: [{"id": "Monday", "data": [{"x": "9am", "y": 10}, {"x": "10am", "y": 25}]}]

Properties:
- chartType: "heatmap"
- data: Array of rows
- heatmapStyle: JSON object with forceSquare, cellOpacity, enableLabels, etc.

### Scatter Plot
Data format: Same as line chart - array of series with {x, y} numeric points.

Example data: [{"id": "Group A", "data": [{"x": 10, "y": 20}, {"x": 15, "y": 35}]}]

Properties:
- chartType: "scatter"
- data: Array of series
- scatterStyle: JSON object with nodeSize, useMesh, etc.

### Funnel Chart
Data format: Array of step objects with id and value.

Example data: [{"id": "Visitors", "value": 10000}, {"id": "Leads", "value": 3000}]

Properties:
- chartType: "funnel"
- data: Array of funnel steps
- funnelStyle: JSON object with direction, interpolation, shapeBlending, etc.

### Treemap
Data format: Hierarchical object with name and children array.

Example data: {"name": "root", "children": [{"name": "Category A", "value": 100}]}

Properties:
- chartType: "treemap"
- data: Hierarchical tree object
- treemapStyle: JSON object with tile, innerPadding, enableLabel, etc.

### Sunburst
Data format: Same as treemap - hierarchical object.

Properties:
- chartType: "sunburst"
- data: Hierarchical tree object

### Calendar Heatmap
Data format: Array of day-value objects.

Example data: [{"day": "2024-01-01", "value": 10}, {"day": "2024-01-15", "value": 45}]

Properties:
- chartType: "calendar"
- data: Array of day entries
- calendarStyle: JSON object with direction, emptyColor, yearSpacing, etc.

### Sankey Diagram
Data format: Object with nodes array and links array.

Example data: {"nodes": [{"id": "A"}, {"id": "B"}], "links": [{"source": "A", "target": "B", "value": 100}]}

Properties:
- chartType: "sankey"
- data: Object with nodes and links
- sankeyStyle: JSON object with layout, enableLinkGradient, enableLabels, etc.

### Chord Diagram
Data format: 2D matrix of flow values between categories.

Example data: [[100, 30], [30, 80]] with keys ["A", "B"]

Properties:
- chartType: "chord"
- data: 2D matrix
- keys: Array of category names
- chordStyle: JSON object with padAngle, innerRadiusRatio, ribbonOpacity, etc.

### Bump Chart (Rankings over time)
Data format: Array of series with ranking data points.

Example data: [{"id": "Team A", "data": [{"x": "Week 1", "y": 1}, {"x": "Week 2", "y": 2}]}]

Properties:
- chartType: "bump"
- data: Array of ranking series

### Area Bump Chart
Similar to bump but with area fills.

Properties:
- chartType: "areaBump"
- data: Array of series

### Stream Chart
Data format: Array of time-slice objects with values for each category.

Example data: [{"cat1": 10, "cat2": 20}, {"cat1": 15, "cat2": 25}]

Properties:
- chartType: "stream"
- data: Array of time slices
- keys: Array of category keys

### Radial Bar
Data format: Array of metric objects with data arrays.

Example data: [{"id": "Metric A", "data": [{"x": "Target", "y": 80}]}]

Properties:
- chartType: "radialBar"
- data: Array of metrics

### Waffle Chart
Data format: Array of category objects with id, label, and value.

Example data: [{"id": "cats", "label": "Cats", "value": 35}]

Properties:
- chartType: "waffle"
- data: Array of categories

---

## Color Schemes

### Nivo Color Schemes
- nivo - Default Nivo palette
- category10 - D3 category10
- paired - D3 paired (good for comparisons)
- dark2 - D3 dark palette
- pastel1, pastel2 - Soft colors
- set1, set2, set3 - D3 sets
- accent - Accent colors
- spectral - Rainbow gradient
- blues, greens, oranges, reds, purples - Sequential

### Custom Colors
Use colors property with JSON array: ["#3b82f6", "#10b981", "#f59e0b"]

---

## Plotly Charts (plotlyChart)

Plotly provides interactive scientific charts with zoom, pan, and export.

### Common Properties
- chartType: "line", "bar", "scatter", "pie", "area", "histogram"
- data: Plotly trace array (JSON)
- title: Chart title
- layout: Plotly layout object (optional)
- config: Plotly config object (optional)
- height: Chart height
- responsive: Enable responsive sizing

### Line/Scatter Chart
Data format: Plotly trace with x, y arrays.

Example: [{"x": ["Jan", "Feb"], "y": [10, 15], "type": "scatter", "mode": "lines+markers"}]

### Bar Chart
Data format: Plotly trace with x, y arrays.

Example: [{"x": ["A", "B"], "y": [20, 30], "type": "bar"}]

### Pie Chart
Data format: Plotly trace with values and labels.

Example: [{"values": [40, 30, 20], "labels": ["A", "B", "C"], "type": "pie", "hole": 0.4}]

### Area Chart
Data format: Scatter trace with fill property.

Example: [{"x": [1, 2, 3], "y": [10, 20, 30], "fill": "tozeroy", "type": "scatter"}]

### Histogram
Data format: Trace with x array of values.

Example: [{"x": [1, 2, 2, 3, 3, 3, 4, 4, 4, 4], "type": "histogram"}]

"##;

pub const GAME_COMPONENT_DOCUMENTATION: &str = r##"
# Game & Interactive Media Components

## 2D Canvas System

### canvas2d
Container for 2D sprites and shapes. Creates a canvas context for game rendering.

Properties:
- width: Canvas width (required, e.g., "800px")
- height: Canvas height (required, e.g., "600px")
- backgroundColor: Background color
- pixelPerfect: Disable antialiasing for pixel art (boolean)
- children: Array of sprite and shape component IDs

### sprite
2D image with position, rotation, scale.

Properties:
- src: Image URL (required)
- x: X position in pixels (required)
- y: Y position in pixels (required)
- width: Width in pixels
- height: Height in pixels
- rotation: Rotation in degrees
- scale: Scale factor (1 = 100%)
- opacity: 0-1
- flipX: Mirror horizontally (boolean)
- flipY: Mirror vertically (boolean)
- zIndex: Stacking order

### shape
2D geometric shapes for simple graphics.

Properties:
- shapeType: "rectangle", "circle", "ellipse", "polygon", "line", "path" (required)
- x: X position (required)
- y: Y position (required)
- width: Width (for rectangle, ellipse)
- height: Height (for rectangle, ellipse)
- radius: Radius (for circle)
- points: Array of [x,y] for polygon
- fill: Fill color
- stroke: Stroke color
- strokeWidth: Stroke width

---

## 3D Scene System

### scene3d
3D scene container with camera, lighting, and controls.

Properties:
- width: Scene width (required, e.g., "100%")
- height: Scene height (required, e.g., "500px")
- backgroundColor: Background color
- cameraType: "perspective" or "orthographic"
- cameraPosition: [x, y, z] array
- controlMode: "orbit", "fly", "fixed", "auto-rotate"
- fixedView: "front", "back", "left", "right", "top", "bottom", "isometric"
- autoRotateSpeed: Degrees per second
- enableControls: Enable user controls (boolean)
- enableZoom: Enable zoom (boolean)
- enablePan: Enable panning (boolean)
- fov: Field of view (degrees)
- target: [x, y, z] look-at target
- ambientLight: Ambient light intensity (0-1)
- directionalLight: Main light intensity (0-1)
- showGrid: Show ground grid (boolean)
- showAxes: Show XYZ axes (boolean)
- children: Array of model3d component IDs

### model3d
3D model viewer. Can be standalone or inside a scene3d.

**Standalone Properties (auto-creates viewer):**
- src: GLB/GLTF URL (required)
- viewerHeight: Viewer height
- backgroundColor: Background color
- cameraAngle: "front", "side", "top", "isometric"
- cameraDistance: Distance from model
- cameraPosition: [x, y, z] override
- enableControls: Enable orbit controls (boolean)
- enableZoom: Enable zoom (boolean)
- autoRotateCamera: Camera orbits model (boolean)
- lightingPreset: "neutral", "warm", "cool", "studio", "dramatic"
- environment: "studio", "sunset", "dawn", "night", "warehouse", "forest", "city"
- showGround: Show ground plane (boolean)

**Inside scene3d Properties:**
- src: GLB/GLTF URL (required)
- position: [x, y, z] position
- rotation: [x, y, z] rotation in radians
- scale: Scale factor or [x, y, z]
- animation: Animation name to play
- autoRotate: Model auto-rotates (boolean)
- castShadow: Cast shadows (boolean)

---

## Visual Novel / Dialogue Components

### dialogue
Dialogue box with typewriter effect.

Properties:
- text: Dialogue text (required)
- speakerName: Speaker name
- speakerPortraitId: Component ID of portrait
- typewriter: Enable typewriter effect (boolean)
- typewriterSpeed: Characters per second

### characterPortrait
Character portrait with expressions.

Properties:
- image: Portrait image URL (required)
- expression: Expression key for sprite sheet
- position: "left", "right", "center"
- size: "small", "medium", "large"
- dimmed: Dim when not speaking (boolean)

### choiceMenu
Interactive choice/decision menu.

Properties:
- choices: Array of {id, text, disabled?} (required)
- title: Menu title
- layout: "vertical", "horizontal", "grid"

---

## Game UI Components

### inventoryGrid
Grid-based inventory display.

Properties:
- items: Array of {id, icon, name, quantity?} (required)
- columns: Grid columns
- rows: Grid rows
- cellSize: Cell size (e.g., "64px")

### healthBar
Health/resource bar with variants.

Properties:
- value: Current value (required)
- maxValue: Maximum value (required)
- label: Label text
- showValue: Show numeric value (boolean)
- fillColor: Fill color
- backgroundColor: Background color
- variant: "bar", "segmented", "circular"

### miniMap
Mini-map with markers.

Properties:
- mapImage: Map background image
- width: Map width (required)
- height: Map height (required)
- markers: Array of {id, x, y, icon?, color?, label?}
- playerX: Player X position (0-1 normalized)
- playerY: Player Y position (0-1 normalized)
- playerRotation: Player rotation (degrees)

"##;

pub const ML_VISION_DOCUMENTATION: &str = r##"
# Computer Vision / ML Components

## boundingBoxOverlay
Display bounding boxes on images for object detection visualization.

Properties:
- src: Image URL (required)
- boxes: Array of bounding boxes (required)
- showLabels: Show class labels (boolean)
- showConfidence: Show confidence scores (boolean)
- strokeWidth: Box stroke width
- fontSize: Label font size
- fit: "contain", "cover", "fill"
- normalized: If true, coords are 0-1 (boolean)
- interactive: Enable click events (boolean)

Box Format (normalized=true):
- id: Unique identifier
- x: X position (0-1, percentage from left)
- y: Y position (0-1, percentage from top)
- width: Width (0-1, percentage of image width)
- height: Height (0-1, percentage of image height)
- label: Class label (e.g., "Person")
- confidence: Confidence score (0-1)
- color: Box color

Box Format (normalized=false, pixels):
Same format but x, y, width, height are in pixels.

## imageLabeler
Interactive component for drawing bounding boxes (annotation tool).

Properties:
- src: Image URL (required)
- labels: Array of available label options (required, e.g., ["Person", "Car", "Dog"])
- boxes: Initial boxes array
- showLabels: Show labels on boxes (boolean)
- minBoxSize: Minimum box size in pixels
- disabled: Disable editing (boolean)

## imageHotspot
Interactive image with clickable hotspots (point-and-click).

Properties:
- src: Image URL (required)
- hotspots: Array of hotspots (required)
- showMarkers: Show hotspot markers (boolean)
- markerStyle: "pulse", "dot", "ring", "square", "diamond", "none"
- fit: "contain", "cover", "fill"
- normalized: If true, coords are 0-1 (boolean)
- showTooltips: Show tooltips on hover (boolean)

Hotspot Format:
- id: Unique identifier
- x: X position (normalized or pixels)
- y: Y position (normalized or pixels)
- size: Marker size in pixels
- color: Marker color
- icon: Lucide icon name
- label: Label text
- description: Description shown in tooltip
- action: Action name for click handler
- disabled: Disable hotspot (boolean)

"##;

pub const STYLE_GUIDE: &str = r##"
# A2UI Styling Guide

## Theme Colors (Always Use These)

### Backgrounds
- bg-background - Main background
- bg-muted - Subtle background
- bg-muted/50 - Semi-transparent muted
- bg-card - Card background
- bg-primary - Primary brand color
- bg-secondary - Secondary color
- bg-accent - Accent color
- bg-destructive - Error/danger

### Text
- text-foreground - Main text
- text-muted-foreground - Secondary text
- text-primary - Primary colored text
- text-primary-foreground - Text on primary bg
- text-secondary-foreground - Text on secondary bg
- text-destructive - Error text

### Borders
- border-border - Default border
- border-primary - Primary border
- border-destructive - Error border
- ring-ring - Focus ring

## Spacing Scale
- p-1 = 4px, p-2 = 8px, p-3 = 12px, p-4 = 16px
- p-5 = 20px, p-6 = 24px, p-8 = 32px, p-10 = 40px
- gap-1 through gap-10 (same scale)
- m-1 through m-10 (margin, same scale)

## Responsive Breakpoints
- sm: - >=640px (tablet)
- md: - >=768px (small laptop)
- lg: - >=1024px (desktop)
- xl: - >=1280px (large desktop)
- 2xl: - >=1536px (extra large)

Example: grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4

## Common Patterns

### Card with hover effect
className: "bg-card border border-border rounded-lg p-4 hover:shadow-lg transition-shadow"

### Gradient text
className: "bg-gradient-to-r from-primary to-purple-500 bg-clip-text text-transparent"

### Glass effect
className: "bg-background/80 backdrop-blur-lg border border-border/50"

### Subtle shadow
className: "shadow-sm hover:shadow-md transition-shadow"

## Custom CSS Patterns (use in canvasSettings.customCss)

### Animated gradient background
.gradient-bg {
  background: linear-gradient(135deg, var(--primary) 0%, purple 100%);
  animation: gradient 3s ease infinite;
  background-size: 200% 200%;
}
@keyframes gradient {
  0% { background-position: 0% 50%; }
  50% { background-position: 100% 50%; }
  100% { background-position: 0% 50%; }
}

### Glow effect
.glow {
  box-shadow: 0 0 20px rgba(102, 126, 234, 0.5);
}

### Pulse animation
.pulse {
  animation: pulse 2s infinite;
}
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

### Hover lift
.hover-lift {
  transition: transform 0.2s, box-shadow 0.2s;
}
.hover-lift:hover {
  transform: translateY(-4px);
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.15);
}

"##;

/// Get the full component documentation for AI copilot
pub fn get_full_documentation() -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n{}",
        COMPONENT_CATALOG,
        CHART_DOCUMENTATION,
        GAME_COMPONENT_DOCUMENTATION,
        ML_VISION_DOCUMENTATION,
        STYLE_GUIDE
    )
}

/// Get a specific section of documentation
pub fn get_documentation_section(section: &str) -> Option<&'static str> {
    match section.to_lowercase().as_str() {
        "catalog" | "components" | "all" => Some(COMPONENT_CATALOG),
        "charts" | "nivo" | "plotly" | "visualization" => Some(CHART_DOCUMENTATION),
        "game" | "3d" | "2d" | "interactive" => Some(GAME_COMPONENT_DOCUMENTATION),
        "ml" | "vision" | "cv" | "detection" => Some(ML_VISION_DOCUMENTATION),
        "style" | "styling" | "css" | "theme" => Some(STYLE_GUIDE),
        _ => None,
    }
}
