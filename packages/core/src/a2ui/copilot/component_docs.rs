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
- `button` - Clickable button (variants, sizes, loading state, icon)
- `textField` - Text input; set `multiline` (+ `rows`) for textarea-style long text - there is NO separate textarea/richtext component
- `select` - Dropdown selection (single value from `options`)
- `slider` - Numeric range slider (min/max/step)
- `checkbox` - Boolean checkbox
- `switch` - Toggle switch (on/off setting)
- `radioGroup` - Radio buttons (single choice among few visible options)
- `dateTimeInput` - Date/time picker (`mode`: date | time | datetime)
- `fileInput` - File upload (`accept`, `multiple`, `maxSize`, `maxFiles`)
- `imageInput` - Image upload with preview (`aspectRatio`, `showPreview`)
- `voiceInput` - Microphone recording / dictation - THE component for every record-audio, voice-note, speech or push-to-talk request (full docs in the Voice Input section)
- `feedback` - Rating input: thumbs up/down (default) or a numeric 0-5 button scale (`mode`: "rating"/"scale" with `positiveRating`/`negativeRating`), optional comment dialog - the only rating component (no star rating exists)
- `appLink` - Button-styled navigation that opens an app/event (`appId`, `eventId`, `target`)
- `link` - Plain hyperlink navigation (href/route + query params)

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
- `graph` - Node/edge network graph on a WebGL canvas with legend, search and inspectors (props: nodes, edges, labelStyles, showToolbar, showSearch, showLegend, showInspector, height)
- `ontologyGraph` - Live explorer for one of the project's ontologies: real data, neighbour expansion, path finding and governed ontology actions (props: ontologyId, appId, limit, allowExpand, allowSearch, allowPaths, allowActions, allowCypher, allowStyleEdit, allowLimitChange, showToolbar, showLegend, height)

### Planning Components
- `calendar` - Interactive calendar (month/week/day/agenda) with detail/edit dialogs and right-click menus; fires create/update/move/resize/open/delete actions (props: events, view, date, title, density, editable, selectable, ...)
- `gantt` - Interactive Gantt timeline with drag/resize/dependency-link, detail/edit dialogs, right-click menus and task-list drag-reordering; fires create/update/move/resize/open/delete/link/reorder actions (props: tasks, view, title, density, editable, showDependencies, showTaskList, ...)

### Media Components
- `iframe` - Embedded external content or HTML preview (supports src URL and srcdoc HTML)
- `filePreview` - Generic file preview
- `geoMap` - Interactive map with markers, routes and viewport control

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

## Choosing the Right Component (intent -> type)
Match the user's intent to the purpose-built component. Rebuilding one of these from generic
parts (a button + fileInput standing in for voiceInput, a hand-made tab bar, a div-drawn rating)
is a defect:
- record audio / voice memo / dictation / push-to-talk / talk to the app -> `voiceInput`
- play a workflow's audio response (conversational voice loop) -> `voiceInput` with `resultMode: "autoplay"`
- long or multi-line text -> `textField` with `multiline: true` (no textarea/richtext type exists)
- thumbs, like/dislike, "was this helpful", 0-5 score -> `feedback` (thumbs or numeric scale mode + comment; no star-rating component exists)
- choose one of few visible options -> `radioGroup`; one of many -> `select`; on/off setting -> `switch`; form consent/multi-pick -> `checkbox`
- upload images -> `imageInput`; other files -> `fileInput` (no camera-capture component exists - say so rather than faking one)
- date, time, or both -> `dateTimeInput` (`mode`)
- navigate to another app or trigger its event -> `appLink`; plain URL/route -> `link`
- user draws/edits boxes ON an image (annotation input) -> `imageLabeler`; SHOW detection results -> `boundingBoxOverlay`; predefined clickable regions -> `imageHotspot`
- maps/geodata -> `geoMap`; schedules/bookings -> `calendar`; project timelines -> `gantt`
- values over time/categories -> `nivoChart` or `plotlyChart`; row-and-column records -> `table`
- things connected to things (networks, relationships, dependency maps) -> `graph` with your own nodes/edges; the project's OWN ontology/knowledge graph -> `ontologyGraph` with its `ontologyId` (it loads live data itself — never re-fetch the ontology into a `graph`)
- loading placeholder shaped like the layout -> `skeleton`; inline waiting -> `spinner`; known fraction -> `progress`
Only types from this catalog exist. Never invent a type; if nothing fits, compose layout +
display + input primitives and say in your summary what was approximated.

## Voice Input (voiceInput) - audio capture
The only audio-capture component: voice memos, dictation, voice commands, conversational voice
assistants. Any request mentioning record, microphone, speech, dictate, or audio input maps here.
- `value` - binding path where the result lands: {name, size, type, duration, url, flowPath, transcript}
- `mode` - "record" (default): captures audio and uploads it; the workflow receives the file
  reference and does its own transcription. "stt": browser speech-to-text delivering {transcript}
  text only; support depends on the runtime and it silently falls back to record, so prefer
  "record" + workflow-side transcription for anything that must work everywhere.
- `invoke` - "manual" (tap start/stop) | "hold" (push-to-talk) | "auto" (tap once, stops on silence)
- `variant` - visualizer style: "conservative" | "waveform" | "orb" | "vortex" | "shader" |
  "aurora" | "pulse". Pick one that serves the design direction ("orb"/"aurora"/"shader" for
  expressive surfaces, "conservative"/"waveform" for dense tools). The old `visualizer` prop is
  deprecated - do not use it.
- `size` - "sm" | "md" | "lg"; `color` / `recordingColor` - CSS accent colors for idle/recording
- `resultMode` - "player" (playback of the user's recording) | "summary" (compact name/duration
  row) | "autoplay" (plays audio the WORKFLOW pushes to `src` - the voice-assistant reply loop)
- `maxDuration` (seconds, default 300), `autoStop`, `silenceThreshold`, `silenceDuration`,
  `label`, `helperText`, `disabled`

Example:
{
  "id": "voice-note",
  "component": {
    "type": "voiceInput",
    "label": { "literalString": "Record a note" },
    "value": { "path": "$.data.voiceNote" },
    "mode": { "literalString": "record" },
    "invoke": { "literalString": "hold" },
    "variant": { "literalString": "waveform" },
    "resultMode": { "literalString": "player" },
    "actions": [{ "name": "workflow_event", "context": { "nodeId": "<transcribe-note-event-id>" } }]
  }
}
The bound workflow event fetches the recording itself (Get Element Value); transcription,
storage, and AI processing happen in the flow.

## Wiring UI to Workflows

### Actions -> named board events
Interactive components carry actions INSIDE the component object:
`"actions": [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]`
(legacy fallback: only actions[0] fires when no named handler exists).
- Components with multiple interactions use `eventHandlers`, keyed by the documented event name:
  `"eventHandlers": { "open": [{ "name": "workflow_event", "context": { "nodeId": "<open-event>" } }], "delete": [{ "name": "workflow_event", "context": { "nodeId": "<delete-event>" } }] }`.
  Each list executes in order. An exact named list overrides `actions[0]`; an explicit empty list
  disables that event without falling back. Keep `actions` intact when updating older surfaces.
- Create ONE named board event per purpose (e.g. dashboard-load, add-target, refresh-status).
  Never route several buttons through one generic catch-all event.
- The action context carries routing ids ONLY (nodeId, optional boardId/appId). Do NOT push
  element values, form payloads, or target ids through the context: the event body fetches
  current element state itself at runtime via Get Element (`a2uiGetElement`), Get Element Value
  (`a2uiGetElementValue`), and Get File Input Files (`a2uiGetFileInputFiles`).
- Other built-in action names: "navigate_page" (context.route, optional context.queryParams)
  and "external_link" (context.url).
- A board can set or re-point an element's default or named action later with Set Element Action
  (`a2uiSetElementAction`, optional event_name + action_type "workflow_event" + node_id).

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
- package widget: Instantiate Widget (`a2uiInstantiateWidget`) with the record's fields on its
  generated `dyn*` inputs, pushed in with Push Child (`a2uiPushChild`); Update Widget Inputs
  (`a2uiWidgetUpdateInputs`) to patch a mounted instance
Data Update (`a2uiDataUpdate`) is never the right node for display updates - a `$.data.*` write is
not observed by elements or widget instances, so use the setters and widget nodes above.

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
# A2UI Design & Styling Guide

## Design Reflection (BEFORE emitting)
Declare a design tuple (macro / surface / type / density - see the DESIGN CONTRACT below the
catalog), then hold every decision to it:
1. Mood - what should this surface FEEL like? Calm analytics, playful game, dense admin tool,
   warm onboarding, futuristic console. Name it; let every choice serve it.
2. Direction - the declared tuple IS the direction. Different apps get different tuples; reaching
   for the same white-card grid every time is a design failure. Worked recipes below.
3. Hierarchy - one focal element per screen. Everything else is subordinate through smaller
   size, lighter weight, and muted color. If everything is bold, nothing is.
4. Rhythm - one spacing unit (a 4px multiple) applied consistently. Related items sit close;
   groups are separated by 2-3x the base gap. Aligned edges everywhere.
5. Signature moment - EXACTLY ONE deliberate flourish, and only when the treatment earns it. A
   utilitarian surface earns none; its craft is information design.
6. Responsive plan - how columns collapse, what hides on small screens, how touch targets grow.

Plain default cards in a plain grid with default padding, no typographic hierarchy, and no
intentional structure is a DEFECT even when the data wiring is correct. So is a gradient hero on
a settings screen.

## What Actually Renders - the three styling channels
1. `style.className` (Tailwind utilities): only STANDARD utilities exist at runtime. The
   stylesheet is compiled ahead of time and there is NO runtime Tailwind engine, so arbitrary
   values (`w-[437px]`, `bg-[#ff00aa]`) and exotic variants silently render NOTHING - an
   arbitrary value only works if that exact literal happens to exist in first-party source,
   which you cannot rely on. Use standard-scale utilities and the theme tokens below; for any
   custom value use channel 2 or 3.
   Shadow gotcha: the standard `shadow-sm`/`shadow-md`/`shadow-lg` utilities are TRANSPARENT in
   this theme (alpha 0 by design) and render no elevation. For real shadows use the
   `shadow-floating` token, the typed `shadow` style field, or customCss box-shadow.
2. Typed `style` fields: always render (inline CSS). Use them for every value outside the
   standard scale: custom gradients, exact sizes, bespoke shadows, filters, animation values.
   Available fields: background, border, shadow, padding, margin, width/height (+ min/max),
   position, zIndex, transform, opacity, overflow, filter, backdropFilter, transition,
   animation, aspectRatio, display, gap, flex/grid placement, typography (color, fontSize,
   fontWeight, fontFamily, lineHeight, letterSpacing, textAlign, textTransform), and
   responsiveOverrides. Key shapes:
   "background": { "gradient": { "type": "linear", "angle": 135, "stops": [
     { "color": "color-mix(in oklab, var(--primary) 35%, transparent)", "position": 0 },
     { "color": "transparent", "position": 100 } ] } }
   "border": { "width": "1px", "style": "solid", "color": "var(--border)", "radius": "16px" }
   "shadow": { "y": "12px", "blur": "40px", "color": "rgba(0,0,0,0.18)" }
   "padding": { "top": "24px", "right": "24px", "bottom": "24px", "left": "24px" }
   Gradient stop positions are percentages 0-100. Build custom colors on the theme variables
   (`var(--primary)`, `var(--background)`, `var(--border)`, `var(--muted)`) so they stay
   correct in both light and dark mode.
3. `canvasSettings.customCss`: a scoped stylesheet for what the other two cannot do - keyframe
   animations, hover/focus states, pseudo-elements (::before/::after), extra media queries.
   Classes it defines apply only where a component's `className` references them. Never style
   `:root` in it (it leaks outside this surface). Keep customCss under 12000 chars and any
   single style string under 1000.

## Theme Colors (default vocabulary - always correct in light AND dark mode)

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

NEVER hardcoded palette classes (bg-white, text-black, bg-gray-*) - they break dark mode. When
the design direction needs colors beyond the theme, use typed style fields or customCss with
values built on the theme variables.

## Typography (three real families already exist - use them)
Custom webfonts are impossible (`@import` is stripped), and you do not need them. The theme ships
three genuinely different faces, reachable by class or by typed `fontFamily`:
- `font-serif` / `var(--font-serif)` - Playfair Display -> Didot -> Georgia. A real display serif.
  Use it for ONE role (display headings, or a single pull quote). Never for body text.
- `font-mono` / `var(--font-mono)` - JetBrains Mono -> Menlo. Metrics, IDs, timestamps, eyebrows,
  table numerals, code.
- `font-sans` / `var(--font-sans)` - Inter -> Open Sans. Body and UI.
A surface that is 100% font-sans has skipped its type decision. Max two visible families (three
only when mono is confined to numerals).

Scale, then roles:
- Display / hero: typed `fontSize` with clamp - `"fontSize": "clamp(2.25rem, 6vw, 4.5rem)"`,
  `"lineHeight": "0.95"`, `"letterSpacing": "-0.03em"`. NOTE `text-5xl`/`text-6xl` are NOT
  compiled - anything above `text-4xl` must go through typed `fontSize`.
- Section heading: text-2xl font-semibold tracking-tight
- Card title: text-lg font-semibold
- Body: text-sm/text-base text-foreground, typed `"maxWidth": "68ch"` on prose
- Eyebrow / meta: typed `{"fontFamily": "var(--font-mono)", "fontSize": "11px",
  "textTransform": "uppercase", "letterSpacing": "0.16em", "color": "var(--muted-foreground)"}`
- Big metric: font-mono + `.tnum` (see customCss) so digits align in columns
Use extremes rather than the middle: weight 300 against 800, not 400 against 600; a 2.5x size jump,
not 1.4x. Two levels of contrast minimum between focal and supporting text (size AND weight AND
color).

## Spacing Scale
- p-1 = 4px, p-2 = 8px, p-3 = 12px, p-4 = 16px
- p-5 = 20px, p-6 = 24px, p-8 = 32px, p-10 = 40px
- gap-1 through gap-10 (same scale)
- m-1 through m-10 (margin, same scale)
Generous beats cramped: sections p-6/p-8, cards p-4/p-6, grouped gaps gap-2/gap-3, section
gaps gap-6/gap-8.

## Worked Direction Recipes
Six tuples, each internally consistent and mutually distinct on at least three axes. Use them as
worked examples, not as a menu to mix - and never ship two neighbouring surfaces from the same one.
The taxonomy has 8x6x4x3 combinations; these are six of them.

### INSTRUMENT - macro=dense-board surface=hairline-flat type=mono-led density=dense
Ops dashboards, monitoring, queues. Utilitarian: no shadow, no gradient, no display type.
- Grid: `className: "grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-px bg-border"` - the
  hairlines ARE the gaps
- Panel: `className: "bg-card"`, typed `{"border": {"radius": "0px"}}`
- Label: typed `{"fontFamily": "var(--font-mono)", "fontSize": "11px", "textTransform":
  "uppercase", "letterSpacing": "0.16em", "color": "var(--muted-foreground)"}`
- Metric: `className: "font-mono tnum"`, typed `{"fontSize": "38px", "fontWeight": "500",
  "letterSpacing": "-0.02em"}`
- Accent: one 2px primary rule on the single focal panel

### LEDGER - macro=single-column-doc surface=paper-tint type=serif-display density=airy
Reports, summaries, changelogs, narrative onboarding. No cards anywhere.
- Root: `className: "min-h-screen px-5 py-16 md:py-24"`, typed
  `{"background": {"color": "color-mix(in oklab, var(--primary) 3%, var(--background))"}}`
- Column: typed `{"maxWidth": "70ch", "margin": {"left": "auto", "right": "auto"}}`
- Display: typed `{"fontFamily": "var(--font-serif)", "fontSize": "clamp(2.5rem, 6.5vw, 4.75rem)",
  "fontWeight": "400", "lineHeight": "0.95", "letterSpacing": "-0.03em"}`
- Body: `className: "text-muted-foreground"`, typed `{"fontSize": "17px", "lineHeight": "1.72"}`
- Section breaks: `className: "border-t border-border"` - rules, not boxes

### LUMEN - macro=stacked-panels surface=translucent-layered type=sans-weight-extremes density=standard
Assistant shells, live consoles, media/AI surfaces. Expressive.
- Root atmosphere (typed background on the ROOT component, with `className: "min-h-screen"`):
  `{"background": {"gradient": {"type": "radial", "direction": "120% 90% at 18% -10%", "stops": [
    {"color": "color-mix(in oklab, var(--primary) 24%, transparent)", "position": 0},
    {"color": "color-mix(in oklab, var(--tertiary) 10%, transparent)", "position": 38},
    {"color": "transparent", "position": 72}]}}}`
- Panel: `className: "rise bg-card/55 border border-border/50"`, typed `{"border": {"radius":
  "18px"}}` - the /55 fill must carry it alone where backdrop blur is disabled
- Type: display `{"fontWeight": "800", "letterSpacing": "-0.035em"}` against body
  `{"fontWeight": "300", "color": "var(--muted-foreground)"}`
- The ONE flourish: a staggered page-load reveal. Delay goes in the typed `animation` shorthand
  (there is no animationDelay field): `{"animation": "rise .55s cubic-bezier(.2,.8,.2,1) .09s both"}`

### ATELIER - macro=split-pane surface=tinted-fill-no-border type=sans-weight-extremes density=airy
Onboarding, profile, settings-as-product. Soft and borderless - no borders at all.
- Surface: typed `{"background": {"color": "color-mix(in oklab, var(--muted) 72%,
  var(--background))"}, "border": {"radius": "22px"}, "shadow": {"y": "18px", "blur": "48px",
  "spread": "-20px", "color": "color-mix(in oklab, var(--foreground) 20%, transparent)"}}`
- Rhythm: `className: "p-8 gap-8"` at section level, `gap-3` within groups
- Accent: exactly one filled `bg-primary` button; everything else neutral

### BLUEPRINT - macro=tab-workbench surface=hairline-flat type=uppercase-tracked-lead density=standard
Technical tools, schema/config editors, labeling surfaces.
- Root: `className: "grid-paper min-h-screen bg-background"` (texture in customCss below)
- Panels: transparent, `className: "border border-border"`, typed `{"border": {"radius": "0px"}}`
- Section lead: typed `{"fontSize": "12px", "fontWeight": "600", "textTransform": "uppercase",
  "letterSpacing": "0.2em"}` over a hairline `border-t border-border`
- Values: `className: "font-mono tnum text-sm"`; accent is a single 2px underline on the active tab

### MARQUEE - macro=marquee-band surface=full-bleed-gradient type=serif-display density=standard
Launch/campaign pages, app landing shells. Expressive only.
- Band: typed `{"background": {"gradient": {"type": "linear", "angle": 104, "stops": [
    {"color": "color-mix(in oklab, var(--primary) 92%, var(--background))", "position": 0},
    {"color": "color-mix(in oklab, var(--tertiary) 78%, var(--background))", "position": 58},
    {"color": "color-mix(in oklab, var(--background) 88%, var(--primary))", "position": 100}]}},
   "minHeight": "58vh"}`
- Display, deliberately NOT centered: container `className: "flex flex-col items-start justify-end
  p-6 md:p-12"`, typed `{"fontFamily": "var(--font-serif)", "fontSize": "clamp(3rem, 10vw, 7.5rem)",
  "fontWeight": "700", "lineHeight": "0.88", "letterSpacing": "-0.045em",
  "color": "var(--primary-foreground)"}`
- Below the band: plain `bg-background`, indented against the band's full bleed - that contrast IS
  the structure

## Custom CSS Patterns (use in canvasSettings.customCss)
The FIRST line of customCss is always the design stamp:
`/* fp-design: macro=... surface=... type=... density=... */`

These are NOT a menu to apply all at once - each belongs to a surface language. Pick the ones your
declared tuple calls for and leave the rest out. Never `:root` (it is not scoped and leaks into the
host app); `@import` is stripped, so no webfonts.

### Always safe - aligned numerals (any tuple with metrics or tables)
.tnum { font-variant-numeric: tabular-nums; }

### Always safe - focus ring for custom interactive elements
.focus-card:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

### hairline-flat - inset rim instead of a drop shadow
.panel { position: relative; }
.panel::after {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  box-shadow: inset 0 1px 0 color-mix(in oklab, var(--foreground) 7%, transparent);
}

### hairline-flat / blueprint - engineering grid texture
.grid-paper {
  background-image:
    linear-gradient(color-mix(in oklab, var(--foreground) 6%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in oklab, var(--foreground) 6%, transparent) 1px, transparent 1px);
  background-size: 32px 32px;
  background-position: -1px -1px;
}

### translucent-layered - ONE orchestrated page-load reveal (stagger via the typed animation delay)
@keyframes rise {
  from { opacity: 0; transform: translateY(14px); }
  to { opacity: 1; transform: none; }
}
@media (prefers-reduced-motion: reduce) {
  .rise { animation: none !important; }
}

### translucent-layered - focal glow (ONE element, never a set)
.bloom {
  box-shadow:
    0 0 0 1px color-mix(in oklab, var(--primary) 28%, transparent),
    0 18px 60px -20px color-mix(in oklab, var(--primary) 55%, transparent);
}

### elevated-soft / tinted-fill - hover lift (layered shadow needs customCss, not the typed field)
.lift { transition: transform .26s cubic-bezier(.2,.8,.2,1), box-shadow .26s ease; }
.lift:hover {
  transform: translateY(-3px);
  box-shadow: 0 26px 60px -22px color-mix(in oklab, var(--foreground) 26%, transparent);
}
@media (prefers-reduced-motion: reduce) { .lift { transition: none; } }

### paper-tint - editorial drop cap (serif-display only)
.lede::first-letter {
  float: left;
  font-family: var(--font-serif);
  font-size: 4.1em;
  line-height: 0.78;
  padding-right: 0.07em;
  color: var(--primary);
}
@media (max-width: 480px) { .lede::first-letter { font-size: 3.1em; } }

### Loading shimmer (pair with `skeleton`, not with content)
.shimmer {
  background: linear-gradient(90deg, transparent, color-mix(in oklab, var(--foreground) 8%, transparent), transparent);
  background-size: 200% 100%;
  animation: shimmer 1.8s ease infinite;
}
@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
@media (prefers-reduced-motion: reduce) { .shimmer { animation: none; } }

## Responsive Design (every surface, mobile-first)
Breakpoints: sm >=640px, md >=768px, lg >=1024px, xl >=1280px, 2xl >=1536px (viewport-based).
- className route (standard utilities): grid-cols-1 sm:grid-cols-2 lg:grid-cols-3,
  flex-col md:flex-row, hidden md:block, p-4 md:p-6 lg:p-8, text-sm md:text-base
- Guaranteed typed route (per component, breakpoint keys sm/md/lg/xl/xxl):
  "responsiveOverrides": { "md": { "gridCols": 2 }, "xl": { "gridCols": 4 } }
  Per-breakpoint fields: className, display, flexDirection, justifyContent, alignItems, gap,
  gridCols, width, height, padding, margin, hidden, fontSize, textAlign, order.
- Media queries inside customCss are scoped to the surface and work normally.
Base styles are the MOBILE layout; the surface must stay usable at 360px wide, with touch
targets at least 40px tall.
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
