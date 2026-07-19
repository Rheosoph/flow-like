# A2UI Components Reference

Complete reference for all A2UI components with their properties.

---

## LAYOUT COMPONENTS

### column
Vertical flex container.

| Property | Type | Description |
|----------|------|-------------|
| gap | BoundValue | "4px", "1rem", etc. |
| align | BoundValue | "start", "center", "end", "stretch", "baseline" |
| justify | BoundValue | "start", "center", "end", "between", "around", "evenly" |
| wrap | BoundValue | boolean |
| reverse | BoundValue | boolean |
| children | Children | Child component IDs |

### row
Horizontal flex container.

| Property | Type | Description |
|----------|------|-------------|
| gap | BoundValue | "4px", "1rem", etc. |
| align | BoundValue | "start", "center", "end", "stretch", "baseline" |
| justify | BoundValue | "start", "center", "end", "between", "around", "evenly" |
| wrap | BoundValue | boolean |
| reverse | BoundValue | boolean |
| children | Children | Child component IDs |

### grid
CSS Grid container.

| Property | Type | Description |
|----------|------|-------------|
| columns | BoundValue | "repeat(3, 1fr)", "1fr 2fr" |
| rows | BoundValue | "repeat(2, 1fr)", "auto" |
| gap | BoundValue | "16px", "1rem" |
| columnGap | BoundValue | Column-specific gap |
| rowGap | BoundValue | Row-specific gap |
| autoFlow | BoundValue | "row", "column", "dense", "rowDense", "columnDense" |
| children | Children | Child component IDs |

### stack
Z-axis layering. **REQUIRES min-height in style!**

| Property | Type | Description |
|----------|------|-------------|
| align | BoundValue | "start", "center", "end", "stretch" |
| children | Children | Stacked component IDs |

### scrollArea
Scrollable container.

| Property | Type | Description |
|----------|------|-------------|
| direction | BoundValue | "vertical", "horizontal", "both" |
| children | Children | Child component IDs |

### absolute
Free positioning container.

| Property | Type | Description |
|----------|------|-------------|
| width | BoundValue | "100px", "50%" |
| height | BoundValue | "100px", "50%" |
| children | Children | Child component IDs |

### aspectRatio
Maintain aspect ratio container.

| Property | Type | Description |
|----------|------|-------------|
| ratio | BoundValue | "16/9", "4/3", "1/1" **(REQUIRED)** |
| children | Children | Child component IDs |

### overlay
Position items over a base component.

| Property | Type | Description |
|----------|------|-------------|
| baseComponentId | string | Component ID for the base **(REQUIRED)** |
| overlays | array | `{componentId, anchor?, offsetX?, offsetY?, zIndex?}` |

**Anchor values:** "topLeft", "topCenter", "topRight", "centerLeft", "center", "centerRight", "bottomLeft", "bottomCenter", "bottomRight"

### box
Generic container with semantic HTML element.

| Property | Type | Description |
|----------|------|-------------|
| as | BoundValue | "div", "section", "header", "footer", "main", "aside", "nav", "article", "figure", "span" |
| children | Children | Child component IDs |

### center
Center content horizontally/vertically.

| Property | Type | Description |
|----------|------|-------------|
| inline | BoundValue | boolean - center inline elements |
| children | Children | Child component IDs |

### spacer
Flexible or fixed space.

| Property | Type | Description |
|----------|------|-------------|
| size | BoundValue | Fixed size: "20px", "2rem" |
| flex | BoundValue | Flex grow value (default: 1 if no size) |

---

## DISPLAY COMPONENTS

### text
Text display with typography control.

| Property | Type | Description |
|----------|------|-------------|
| content | BoundValue | Text content **(REQUIRED)** |
| variant | BoundValue | "body", "h1", "h2", "h3", "h4", "h5", "h6", "caption", "code", "label" |
| size | BoundValue | "xs", "sm", "md", "lg", "xl", "2xl", "3xl", "4xl" |
| weight | BoundValue | "light", "normal", "medium", "semibold", "bold" |
| color | BoundValue | Tailwind class like "text-primary" |
| align | BoundValue | "left", "center", "right", "justify" |
| truncate | BoundValue | boolean |
| maxLines | BoundValue | number |

### image
Image display.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | URL string **(REQUIRED)** |
| alt | BoundValue | Alt text |
| fit | BoundValue | "contain", "cover", "fill", "none", "scaleDown" |
| fallback | BoundValue | Fallback image URL |
| loading | BoundValue | "lazy", "eager" |
| aspectRatio | BoundValue | "16/9", "4/3", "1/1" |

### icon
Lucide icon display.

| Property | Type | Description |
|----------|------|-------------|
| name | BoundValue | Lucide icon name **(REQUIRED)** |
| size | BoundValue | "xs", "sm", "md", "lg", "xl" or number |
| color | BoundValue | Tailwind class |
| strokeWidth | BoundValue | number (default 2) |

**Common icon names:** "user", "settings", "chevron-right", "home", "search", "menu", "x", "plus", "minus", "check", "alert-circle", "info", "mail", "phone", "calendar", "clock", "star", "heart", "bookmark", "share", "download", "upload", "edit", "trash", "copy", "link", "external-link", "eye", "eye-off", "lock", "unlock", "filter", "sort", "refresh", "loader", "arrow-left", "arrow-right", "chevron-down", "chevron-up"

### video
Video player.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Video URL **(REQUIRED)** |
| poster | BoundValue | Poster image URL |
| autoplay | BoundValue | boolean |
| loop | BoundValue | boolean |
| muted | BoundValue | boolean |
| controls | BoundValue | boolean |
| width | BoundValue | string |
| height | BoundValue | string |

### lottie
Lottie animation player.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Lottie JSON URL **(REQUIRED)** |
| autoplay | BoundValue | boolean |
| loop | BoundValue | boolean |
| speed | BoundValue | number (1 = normal) |
| width | BoundValue | string |
| height | BoundValue | string |

### markdown
Markdown content renderer.

| Property | Type | Description |
|----------|------|-------------|
| content | BoundValue | Markdown string **(REQUIRED)** |
| allowHtml | BoundValue | boolean |

### badge
Small label/tag.

| Property | Type | Description |
|----------|------|-------------|
| content | BoundValue | Badge text **(REQUIRED)** |
| variant | BoundValue | "default", "secondary", "destructive", "outline" |
| color | BoundValue | Tailwind class |

### avatar
User avatar display.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Image URL |
| fallback | BoundValue | Fallback initials |
| size | BoundValue | "sm", "md", "lg", "xl" |

### userProfile
Flow-Like user profile lookup and display by subject/sub ID.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | User subject/sub ID **(REQUIRED)** |
| variant | BoundValue | "avatar", "chip", "row", "detailed", "card" |
| avatarSize | BoundValue | "xs", "sm", "md", "lg", "xl", "2xl" |
| showHover | BoundValue | boolean - show hover details |
| showEmail | BoundValue | boolean |
| showDescription | BoundValue | boolean |
| showUserId | BoundValue | boolean |
| showProfileLink | BoundValue | boolean |
| fallbackLabel | BoundValue | Text to show when lookup is unavailable |
| muted | BoundValue | boolean |

### progress
Progress indicator.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Current value **(REQUIRED)** |
| max | BoundValue | Maximum value (default 100) |
| showLabel | BoundValue | boolean |
| variant | BoundValue | "default", "success", "warning", "error" |
| color | BoundValue | Tailwind class |

### spinner
Loading spinner.

| Property | Type | Description |
|----------|------|-------------|
| size | BoundValue | "sm", "md", "lg" |
| color | BoundValue | Tailwind class |

### divider
Visual separator.

| Property | Type | Description |
|----------|------|-------------|
| orientation | BoundValue | "horizontal", "vertical" |
| thickness | BoundValue | string |
| color | BoundValue | Tailwind class |

### skeleton
Loading placeholder.

| Property | Type | Description |
|----------|------|-------------|
| width | BoundValue | string |
| height | BoundValue | string |
| rounded | BoundValue | boolean |

### iframe
Embed external pages or preview raw HTML content. Sandboxed by default — no access to parent app resources.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | URL (provide src or srcdoc) |
| srcdoc | BoundValue | Raw HTML string for preview (provide src or srcdoc) |
| title | BoundValue | Frame title |
| width | BoundValue | string |
| height | BoundValue | string |
| sandbox | BoundValue | Sandbox restrictions (defaults: src="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox", srcdoc="allow-scripts") |
| allow | BoundValue | Permissions policy |
| loading | BoundValue | "lazy", "eager" |
| referrerPolicy | BoundValue | "no-referrer" (default), "origin", etc. |
| border | BoundValue | boolean |

### table
Data table.

| Property | Type | Description |
|----------|------|-------------|
| columns | BoundValue | Array of `{id, header, accessor?, width?, align?, sortable?}` **(REQUIRED)** |
| data | BoundValue | Array of row objects **(REQUIRED)** |
| caption | BoundValue | Table caption |
| striped | BoundValue | boolean |
| bordered | BoundValue | boolean |
| hoverable | BoundValue | boolean |
| compact | BoundValue | boolean |
| stickyHeader | BoundValue | boolean |
| sortable | BoundValue | boolean |
| searchable | BoundValue | boolean |
| paginated | BoundValue | boolean |
| pageSize | BoundValue | number |
| selectable | BoundValue | boolean |
| onRowClick | BoundValue | Row action config |

### tableRow
Manual table row. Prefer `table` for data-driven tables unless you need custom row composition.

| Property | Type | Description |
|----------|------|-------------|
| cells | BoundValue | Array of cell values **(REQUIRED)** |
| selected | BoundValue | boolean |
| disabled | BoundValue | boolean |

### tableCell
Manual table cell. Prefer `table` columns/data for normal data tables.

| Property | Type | Description |
|----------|------|-------------|
| content | BoundValue | Cell content **(REQUIRED)** |
| isHeader | BoundValue | boolean |
| colSpan | BoundValue | number |
| rowSpan | BoundValue | number |
| align | BoundValue | "left", "center", "right" |

### plotlyChart
Interactive Plotly charts.

| Property | Type | Description |
|----------|------|-------------|
| chartType | BoundValue | "line", "bar", "scatter", "pie", "area", "histogram" |
| title | BoundValue | Chart title |
| data | BoundValue | Plotly data array |
| layout | BoundValue | Plotly layout object |
| width | BoundValue | Chart width |
| height | BoundValue | Chart height |
| responsive | BoundValue | boolean |
| showLegend | BoundValue | boolean |
| legendPosition | BoundValue | "top", "bottom", "left", "right" |

### nivoChart
Nivo chart library.

| Property | Type | Description |
|----------|------|-------------|
| chartType | BoundValue | "bar", "line", "pie", "radar", "heatmap", "scatter", "funnel", "treemap", "sunburst", "calendar", "bump", "areaBump", "sankey", "chord" **(REQUIRED)** |
| title | BoundValue | Chart title |
| data | BoundValue | Chart data (format depends on chartType) |
| height | BoundValue | Chart height (default "400px") |
| colors | BoundValue | Color scheme or array of colors |
| animate | BoundValue | boolean |
| showLegend | BoundValue | boolean |
| legendPosition | BoundValue | "top", "bottom", "left", "right" |
| indexBy | BoundValue | Key for indexing (bar, radar) |
| keys | BoundValue | Data keys to display (bar, radar) |
| margin | BoundValue | `{top, right, bottom, left}` |
| axisBottom | BoundValue | Bottom axis config |
| axisLeft | BoundValue | Left axis config |

### filePreview
Generic file preview.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | File URL **(REQUIRED)** |
| showControls | BoundValue | boolean |
| fit | BoundValue | "contain", "cover", "fill", "none", "scaleDown" |
| fallbackText | BoundValue | Fallback text |

### diffView
Beautiful diff viewer for text, code, markdown and documents — split, unified or inline, with syntax highlighting and word-level changes.

| Property | Type | Description |
|----------|------|-------------|
| original | BoundValue | Left / old content, or a document URL **(REQUIRED)** |
| modified | BoundValue | Right / new content, or a document URL **(REQUIRED)** |
| mode | BoundValue | "split" \| "unified" \| "inline" (default "split") |
| kind | BoundValue | "auto" \| "text" \| "code" \| "markdown" \| "json" \| "document" (default "auto") |
| language | BoundValue | Syntax language for code/json (e.g. "typescript", "rust", "python"); default "plaintext" |
| markdownMode | BoundValue | "source" (diff raw markdown) \| "rendered" (side-by-side preview); default "source" |
| showLineNumbers | BoundValue | boolean (default true) |
| wordWrap | BoundValue | boolean (default false) |
| wordLevel | BoundValue | boolean — intra-line word-level highlighting (default true) |
| collapseUnchanged | BoundValue | boolean — collapse long unchanged regions (default false) |
| contextLines | BoundValue | number — context kept around changes when collapsing (default 3) |
| showStats | BoundValue | boolean — show +adds/−dels header (default true) |
| originalLabel | BoundValue | string (default "Original") |
| modifiedLabel | BoundValue | string (default "Modified") |
| ignoreWhitespace | BoundValue | boolean (default false) |
| ignoreCase | BoundValue | boolean (default false) |
| trimTrailingWhitespace | BoundValue | boolean (default false) |
| swapSides | BoundValue | boolean (default false) |

### boundingBoxOverlay
Display bounding boxes on an image.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Image URL **(REQUIRED)** |
| alt | BoundValue | Alt text |
| boxes | BoundValue | Array of `{id?, x, y, width, height, label?, confidence?, color?}` **(REQUIRED)** |
| showLabels | BoundValue | boolean |
| showConfidence | BoundValue | boolean |
| strokeWidth | BoundValue | number |
| fontSize | BoundValue | number |
| fit | BoundValue | "contain", "cover", "fill" |
| normalized | BoundValue | boolean - if true, coordinates are 0-1 |
| interactive | BoundValue | boolean - enable click events |

---

## INTERACTIVE COMPONENTS

### button
Clickable button.

| Property | Type | Description |
|----------|------|-------------|
| label | BoundValue | Button text **(REQUIRED)** |
| variant | BoundValue | "default", "secondary", "outline", "ghost", "destructive", "link" |
| size | BoundValue | "sm", "md", "lg", "icon" |
| disabled | BoundValue | boolean |
| loading | BoundValue | boolean |
| icon | BoundValue | Lucide icon name |
| iconPosition | BoundValue | "left", "right" |
| tooltip | BoundValue | Hover tooltip |

### feedback
Built-in user feedback control for thumbs up/down, segmented, rating, or comment feedback.

| Property | Type | Description |
|----------|------|-------------|
| mode | BoundValue | "icon", "compact", "segmented", "rating", "extended" |
| size | BoundValue | "sm", "md", "lg" |
| title | BoundValue | Feedback prompt title |
| description | BoundValue | Supporting text |
| positiveLabel | BoundValue | Positive option label |
| negativeLabel | BoundValue | Negative option label |
| positiveRating | BoundValue | Positive rating value |
| negativeRating | BoundValue | Negative rating value |
| showComment | BoundValue | boolean |
| commentMode | BoundValue | "none", "inline", "modal" |
| commentLabel | BoundValue | Comment field label |
| commentPlaceholder | BoundValue | Comment field placeholder |
| commentTitle | BoundValue | Modal/inline comment title |
| commentDescription | BoundValue | Comment helper text |
| commentSubmitLabel | BoundValue | Submit button label |
| commentCancelLabel | BoundValue | Cancel button label |
| feedbackId | BoundValue | Stable feedback ID |
| includeState | BoundValue | boolean - include component/page state in feedback context |
| pageContextMode | BoundValue | "none", "path", "query" |
| pageContextQueryParamAllowlist | BoundValue | Comma-separated query parameter names |
| pageContextQueryParamDenylist | BoundValue | Comma-separated query parameter names |
| includePageHash | BoundValue | boolean |
| successMessage | BoundValue | Message after submission |
| disabled | BoundValue | boolean |

### appLink
Built-in link button to Flow-Like app shell screens.

| Property | Type | Description |
|----------|------|-------------|
| target | BoundValue | "config", "settings", "overview" |
| label | BoundValue | Button label |
| variant | BoundValue | "default", "secondary", "outline", "ghost", "destructive", "link" |
| size | BoundValue | "sm", "md", "lg", "icon" |
| icon | BoundValue | Lucide icon name |
| iconPosition | BoundValue | "left", "right" |
| appId | BoundValue | Target app ID |
| eventId | BoundValue | Target event ID |
| disabled | BoundValue | boolean |

### textField
Text input field.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Current value **(REQUIRED)** |
| placeholder | BoundValue | Placeholder text |
| label | BoundValue | Field label |
| helperText | BoundValue | Helper text below |
| error | BoundValue | Error message |
| disabled | BoundValue | boolean |
| inputType | BoundValue | "text", "email", "password", "number", "tel", "url", "search" |
| multiline | BoundValue | boolean - textarea |
| rows | BoundValue | number |
| maxLength | BoundValue | number |
| required | BoundValue | boolean |

### select
Dropdown selection.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Selected value **(REQUIRED)** |
| options | BoundValue | Array of `{value, label}` **(REQUIRED)** |
| placeholder | BoundValue | Placeholder text |
| label | BoundValue | Field label |
| disabled | BoundValue | boolean |
| multiple | BoundValue | boolean |
| searchable | BoundValue | boolean |

### slider
Range slider.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Current value **(REQUIRED)** |
| min | BoundValue | number |
| max | BoundValue | number |
| step | BoundValue | number |
| disabled | BoundValue | boolean |
| showValue | BoundValue | boolean |
| label | BoundValue | string |

### checkbox
Boolean toggle.

| Property | Type | Description |
|----------|------|-------------|
| checked | BoundValue | boolean **(REQUIRED)** |
| label | BoundValue | string |
| disabled | BoundValue | boolean |
| indeterminate | BoundValue | boolean |

### switch
Toggle switch.

| Property | Type | Description |
|----------|------|-------------|
| checked | BoundValue | boolean **(REQUIRED)** |
| label | BoundValue | string |
| disabled | BoundValue | boolean |

### radioGroup
Radio button group.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Selected value **(REQUIRED)** |
| options | BoundValue | Array of `{value, label}` **(REQUIRED)** |
| disabled | BoundValue | boolean |
| orientation | BoundValue | "horizontal", "vertical" |
| label | BoundValue | Group label |

### dateTimeInput
Date/time picker.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | ISO string **(REQUIRED)** |
| mode | BoundValue | "date", "time", "datetime" |
| min | BoundValue | ISO string |
| max | BoundValue | ISO string |
| disabled | BoundValue | boolean |
| label | BoundValue | string |

### fileInput
File upload.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | File data |
| label | BoundValue | Field label |
| helperText | BoundValue | Helper text |
| accept | BoundValue | ".pdf,.doc" etc. |
| multiple | BoundValue | boolean |
| maxSize | BoundValue | number (bytes) |
| maxFiles | BoundValue | number |
| disabled | BoundValue | boolean |
| error | BoundValue | Error message |

### imageInput
Image upload with preview.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Image data |
| label | BoundValue | Field label |
| accept | BoundValue | Accepted types |
| multiple | BoundValue | boolean |
| maxSize | BoundValue | number |
| aspectRatio | BoundValue | Crop ratio |
| showPreview | BoundValue | boolean |
| disabled | BoundValue | boolean |

### voiceInput
Voice recording input.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Audio/file data **(REQUIRED)** |
| label | BoundValue | Field label |
| helperText | BoundValue | Helper text |
| maxDuration | BoundValue | Maximum recording duration in seconds |
| autoStop | BoundValue | boolean - stop automatically after silence |
| silenceThreshold | BoundValue | Silence detection threshold |
| silenceDuration | BoundValue | Silence duration before auto-stop |
| disabled | BoundValue | boolean |
| error | BoundValue | Error message |
| visualizer | BoundValue | boolean - show audio visualizer |

### link
Navigation link.

| Property | Type | Description |
|----------|------|-------------|
| href | BoundValue | URL **(REQUIRED)** |
| label | BoundValue | Link text |
| route | BoundValue | Internal route |
| external | boolean | External link |
| target | string | "_blank", "_self" |
| variant | string | "default", "muted", "primary", "destructive" |
| underline | string | "always", "hover", "none" |
| disabled | BoundValue | boolean |

### imageLabeler
Draw bounding boxes on images for labeling.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Image URL **(REQUIRED)** |
| alt | BoundValue | Alt text |
| boxes | BoundValue | Initial boxes: `{id, x, y, width, height, label}[]` |
| labels | BoundValue | Available labels: `string[]` **(REQUIRED)** |
| disabled | BoundValue | boolean |
| showLabels | BoundValue | boolean |
| minBoxSize | BoundValue | Minimum box size in pixels |

### imageHotspot
Interactive image with clickable hotspots.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Image URL **(REQUIRED)** |
| alt | BoundValue | Alt text |
| hotspots | BoundValue | Array of `{id, x, y, size?, color?, icon?, label?, description?, action?, disabled?}` **(REQUIRED)** |
| showMarkers | BoundValue | boolean |
| markerStyle | BoundValue | "pulse", "dot", "ring", "square", "diamond", "none" |
| fit | BoundValue | "contain", "cover", "fill" |
| normalized | BoundValue | boolean - if true, coordinates are 0-1 |
| showTooltips | BoundValue | boolean |

---

## CONTAINER COMPONENTS

### card
Content container.

| Property | Type | Description |
|----------|------|-------------|
| title | BoundValue | Card title |
| description | BoundValue | Card description |
| footer | BoundValue | Footer content |
| hoverable | BoundValue | boolean |
| clickable | BoundValue | boolean |
| variant | BoundValue | "default", "bordered", "elevated" |
| padding | BoundValue | string |
| headerImage | BoundValue | URL |
| headerIcon | BoundValue | Icon name |
| children | Children | Card body content |

### modal
Dialog overlay.

| Property | Type | Description |
|----------|------|-------------|
| open | BoundValue | boolean **(REQUIRED)** |
| title | BoundValue | string |
| description | BoundValue | string |
| closeOnOverlay | BoundValue | boolean |
| closeOnEscape | BoundValue | boolean |
| showCloseButton | BoundValue | boolean |
| size | BoundValue | "sm", "md", "lg", "xl", "full" |
| centered | BoundValue | boolean |
| children | Children | Modal content |

### tabs
Tabbed content.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Active tab ID **(REQUIRED)** |
| tabs | array | `{id, label, icon?, disabled?, contentComponentId}` **(REQUIRED)** |
| orientation | BoundValue | "horizontal", "vertical" |
| variant | BoundValue | "default", "pills", "underline" |

### accordion
Collapsible sections.

| Property | Type | Description |
|----------|------|-------------|
| items | array | `{id, title, contentComponentId}` **(REQUIRED)** |
| multiple | BoundValue | boolean |
| defaultExpanded | BoundValue | array of IDs |
| collapsible | BoundValue | boolean |

### drawer
Slide-out panel.

| Property | Type | Description |
|----------|------|-------------|
| open | BoundValue | boolean **(REQUIRED)** |
| side | BoundValue | "left", "right", "top", "bottom" |
| title | BoundValue | string |
| size | BoundValue | string |
| overlay | BoundValue | boolean |
| closable | BoundValue | boolean |
| children | Children | Drawer content |

### tooltip
Hover tooltip.

| Property | Type | Description |
|----------|------|-------------|
| content | BoundValue | Tooltip text **(REQUIRED)** |
| side | BoundValue | "top", "right", "bottom", "left" |
| delayMs | BoundValue | number |
| maxWidth | BoundValue | string |
| children | Children | Trigger element |

### popover
Click popover.

| Property | Type | Description |
|----------|------|-------------|
| open | BoundValue | boolean |
| contentComponentId | string | Component ID **(REQUIRED)** |
| side | BoundValue | "top", "right", "bottom", "left" |
| trigger | BoundValue | "click", "hover" |
| closeOnClickOutside | BoundValue | boolean |
| children | Children | Trigger element |

---

## GAME / VISUAL COMPONENTS

### canvas2d
2D drawing canvas container.

| Property | Type | Description |
|----------|------|-------------|
| width | BoundValue | Canvas width **(REQUIRED)** |
| height | BoundValue | Canvas height **(REQUIRED)** |
| backgroundColor | BoundValue | Background color |
| pixelPerfect | BoundValue | boolean |
| children | Children | Sprite/shape children |

### sprite
2D sprite for canvas.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | Image URL **(REQUIRED)** |
| x | BoundValue | X position **(REQUIRED)** |
| y | BoundValue | Y position **(REQUIRED)** |
| width | BoundValue | Sprite width |
| height | BoundValue | Sprite height |
| rotation | BoundValue | Rotation in degrees |
| scale | BoundValue | Scale factor |
| opacity | BoundValue | 0-1 opacity |
| flipX | BoundValue | boolean |
| flipY | BoundValue | boolean |
| zIndex | BoundValue | Layer order |

### shape
2D vector shape for canvas.

| Property | Type | Description |
|----------|------|-------------|
| shapeType | BoundValue | "rectangle", "circle", "ellipse", "line", "polygon" **(REQUIRED)** |
| x | BoundValue | X position **(REQUIRED)** |
| y | BoundValue | Y position **(REQUIRED)** |
| width | BoundValue | Shape width |
| height | BoundValue | Shape height |
| radius | BoundValue | Circle/corner radius |
| points | BoundValue | Polygon points array |
| fill | BoundValue | Fill color |
| stroke | BoundValue | Stroke color |
| strokeWidth | BoundValue | Stroke width |

### scene3d
3D scene container (Three.js).

| Property | Type | Description |
|----------|------|-------------|
| width | BoundValue | Scene width **(REQUIRED)** |
| height | BoundValue | Scene height **(REQUIRED)** |
| cameraType | BoundValue | "perspective", "orthographic" |
| cameraPosition | BoundValue | `{x, y, z}` |
| backgroundColor | BoundValue | Background color |
| controlMode | BoundValue | "orbit", "fly", "firstPerson" |
| fixedView | BoundValue | boolean |
| autoRotateSpeed | BoundValue | number |
| enableControls | BoundValue | boolean |
| enableZoom | BoundValue | boolean |
| enablePan | BoundValue | boolean |
| fov | BoundValue | Field of view |
| target | BoundValue | Look-at target `{x, y, z}` |
| ambientLight | BoundValue | Ambient light config |
| directionalLight | BoundValue | Directional light config |
| showGrid | BoundValue | boolean |
| showAxes | BoundValue | boolean |
| children | Children | 3D model children |

### model3d
3D model viewer.

| Property | Type | Description |
|----------|------|-------------|
| src | BoundValue | GLTF/GLB URL **(REQUIRED)** |
| position | BoundValue | `{x, y, z}` |
| rotation | BoundValue | `{x, y, z}` |
| scale | BoundValue | `{x, y, z}` or number |
| castShadow | BoundValue | boolean |
| receiveShadow | BoundValue | boolean |
| animation | BoundValue | Animation name |
| autoRotate | BoundValue | boolean |
| rotateSpeed | BoundValue | number |
| viewerHeight | BoundValue | string |
| backgroundColor | BoundValue | Background color |
| cameraDistance | BoundValue | number |
| fov | BoundValue | Field of view |
| enableControls | BoundValue | boolean |
| enableZoom | BoundValue | boolean |
| lightingPreset | BoundValue | "default", "studio", "outdoor", "dramatic" |
| environment | BoundValue | Environment map |
| showGround | BoundValue | boolean |

### dialogue
Visual novel dialogue box.

| Property | Type | Description |
|----------|------|-------------|
| text | BoundValue | Dialogue text **(REQUIRED)** |
| speakerName | BoundValue | Speaker name |
| typewriter | BoundValue | boolean - typewriter effect |
| speed | BoundValue | Typing speed |
| portrait | BoundValue | Speaker portrait URL |
| children | Children | Additional content |

### characterPortrait
Character portrait display.

| Property | Type | Description |
|----------|------|-------------|
| image | BoundValue | Image URL **(REQUIRED)** |
| expression | BoundValue | Expression name |
| position | BoundValue | "left", "center", "right" |
| width | BoundValue | string |
| height | BoundValue | string |
| flip | BoundValue | boolean |

### choiceMenu
Interactive choice selection for games or visual novels.

| Property | Type | Description |
|----------|------|-------------|
| choices | BoundValue | Array of `{id, text, disabled?, icon?}` **(REQUIRED)** |
| title | BoundValue | Menu title |
| layout | BoundValue | "vertical", "horizontal", "grid" |
| columns | BoundValue | Grid columns |

### inventoryGrid
Game-style item grid.

| Property | Type | Description |
|----------|------|-------------|
| items | BoundValue | Array of `{id, name, icon?, quantity?, description?}` **(REQUIRED)** |
| columns | BoundValue | Grid columns |
| rows | BoundValue | Grid rows |
| cellSize | BoundValue | Cell size string |
| showTooltips | BoundValue | boolean |

### healthBar
HP or resource bar display.

| Property | Type | Description |
|----------|------|-------------|
| value | BoundValue | Current value **(REQUIRED)** |
| maxValue | BoundValue | Maximum value **(REQUIRED)** |
| label | BoundValue | Bar label |
| fillColor | BoundValue | Fill color |
| variant | BoundValue | "default", "danger", "warning", "success" |
| showLabel | BoundValue | boolean |
| size | BoundValue | "sm", "md", "lg" |
| animated | BoundValue | boolean |

### miniMap
Game mini-map display.

| Property | Type | Description |
|----------|------|-------------|
| mapImage | BoundValue | Map image URL **(REQUIRED)** |
| width | BoundValue | Map width **(REQUIRED)** |
| height | BoundValue | Map height **(REQUIRED)** |
| markers | BoundValue | Array of `{id, x, y, icon?, color?, label?}` |
| playerX | BoundValue | Player X position |
| playerY | BoundValue | Player Y position |
| viewportWidth | BoundValue | Viewport width |
| viewportHeight | BoundValue | Viewport height |
| zoom | BoundValue | Zoom level |

---

## GEO COMPONENTS

### geoMap
Interactive map display.

| Property | Type | Description |
|----------|------|-------------|
| center | BoundValue | `{lat, lng}` |
| zoom | BoundValue | Zoom level |
| markers | BoundValue | Array of `{id, lat, lng, label?, icon?, color?}` |
| style | BoundValue | Map style config |
| width | BoundValue | Map width |
| height | BoundValue | Map height |
| mapStyle | BoundValue | "streets", "satellite", "dark", "light" |
| interactive | BoundValue | boolean |

---

## SPECIAL COMPONENTS

### widgetInstance
Embed a reusable widget defined in Flow-Like.

| Property | Type | Description |
|----------|------|-------------|
| widgetId | BoundValue | Widget ID **(REQUIRED)** |
| widgetInputs | BoundValue | Input bindings |
| bindOutputs | BoundValue | Output bindings |
