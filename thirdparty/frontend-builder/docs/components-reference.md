# A2UI component reference

This catalog mirrors the 71 component types registered by the FlowLike A2UI runtime. Type names and prop names are case-sensitive.

Contents: [layout](#layout), [display/data/media/planning](#display-data-media-and-planning), [interactive](#interactive), [containers](#containers), [game and 3D](#game-and-3d), and [widget system](#widget-system).

## Reading the tables

- `Required` lists props that must be present.
- `Accepted props` lists every component-specific prop accepted by the current validator.
- Component props use BoundValue unless a later section marks them raw.
- All components also support `children`, `actions`, and `hidden` inside `component`; place `id`, `style`, and optional `eventRelevant` on the outer surface-component wrapper.

## Layout

| Type | Required | Accepted props |
|---|---|---|
| `row` | — | `gap`, `align`, `justify`, `wrap`, `reverse` |
| `column` | — | `gap`, `align`, `justify`, `reverse`, `wrap` |
| `stack` | — | `align`, `width`, `height` |
| `grid` | — | `columns`, `rows`, `gap`, `columnGap`, `rowGap`, `autoFlow` |
| `scrollArea` | — | `direction` |
| `aspectRatio` | `ratio` | `ratio` |
| `overlay` | `baseComponentId`, `overlays` | `baseComponentId`, `overlays` |
| `absolute` | — | `width`, `height` |
| `box` | — | `as` |
| `center` | — | `inline` |
| `spacer` | — | `size`, `flex` |

Layout values:

- `align`: `start`, `center`, `end`, `stretch`, or `baseline` where supported.
- `justify`: `start`, `center`, `end`, `between`, `around`, or `evenly`.
- `grid.autoFlow`: `row`, `column`, `dense`, `rowDense`, or `columnDense`.
- `scrollArea.direction`: `vertical`, `horizontal`, or `both`.
- `box.as`: use an allowlisted semantic tag such as `div`, `section`, `header`, `footer`, `main`, `aside`, `nav`, `article`, `figure`, `figcaption`, or `span`.
- Give `stack` explicit `width` and `height`, or stable sizing in wrapper `style.className`.
- `overlay.baseComponentId` is a raw component ID. `overlays` is a raw array of `{componentId, anchor?, offsetX?, offsetY?, zIndex?}`; the optional values use BoundValue. Anchors are `topLeft`, `topCenter`, `topRight`, `centerLeft`, `center`, `centerRight`, `bottomLeft`, `bottomCenter`, and `bottomRight`.

## Display, data, media, and planning

| Type | Required | Accepted props |
|---|---|---|
| `text` | `content` | `content`, `variant`, `size`, `weight`, `color`, `align`, `truncate`, `maxLines` |
| `image` | `src` | `src`, `alt`, `fit`, `fallback`, `loading`, `aspectRatio` |
| `icon` | `name` | `name`, `size`, `color`, `strokeWidth` |
| `video` | `src` | `src`, `poster`, `autoplay`, `loop`, `muted`, `controls`, `width`, `height` |
| `lottie` | `src` | `src`, `autoplay`, `loop`, `speed`, `width`, `height` |
| `markdown` | `content` | `content`, `allowHtml` |
| `divider` | — | `orientation`, `thickness`, `color` |
| `badge` | `content` | `content`, `variant`, `color` |
| `avatar` | — | `src`, `fallback`, `size` |
| `userProfile` | `value` | `value`, `variant`, `avatarSize`, `showHover`, `showEmail`, `showDescription`, `showUserId`, `showProfileLink`, `fallbackLabel`, `muted` |
| `progress` | `value` | `value`, `max`, `showLabel`, `variant`, `color` |
| `spinner` | — | `size`, `color` |
| `skeleton` | — | `width`, `height`, `rounded` |
| `table` | `columns`, `data` | `columns`, `data`, `caption`, `striped`, `bordered`, `hoverable`, `compact`, `stickyHeader`, `sortable`, `searchable`, `paginated`, `pageSize`, `selectable`, `onRowClick` |
| `tableRow` | `cells` | `cells`, `selected`, `disabled` |
| `tableCell` | `content` | `content`, `isHeader`, `colSpan`, `rowSpan`, `align` |
| `iframe` | one of `src` / `srcdoc` | `src`, `srcdoc`, `width`, `height`, `sandbox`, `allow`, `title`, `loading`, `referrerPolicy`, `border` |
| `filePreview` | — | `src`, `url`, `filename`, `mimeType`, `fileType`, `showControls`, `fit`, `fallbackText`, `height`, `showDownload`, `loading`, `variant`, `autoPlay` |
| `diffView` | `original`, `modified` | `original`, `modified`, `mode`, `kind`, `language`, `markdownMode`, `showLineNumbers`, `wordWrap`, `wordLevel`, `collapseUnchanged`, `contextLines`, `showStats`, `originalLabel`, `modifiedLabel`, `ignoreWhitespace`, `ignoreCase`, `trimTrailingWhitespace`, `swapSides` |
| `plotlyChart` | — | `chartType`, `title`, `series`, `xAxis`, `yAxis`, `data`, `layout`, `config`, `width`, `height`, `responsive`, `showLegend`, `legendPosition` |
| `nivoChart` | `chartType` | `chartType`, `title`, `data`, `height`, `colors`, `animate`, `showLegend`, `legendPosition`, `indexBy`, `keys`, `margin`, `axisBottom`, `axisLeft`, `axisTop`, `axisRight`, `config`, `barStyle`, `lineStyle`, `pieStyle`, `radarStyle`, `heatmapStyle`, `scatterStyle`, `funnelStyle`, `treemapStyle`, `sankeyStyle`, `calendarStyle`, `chordStyle` |
| `boundingBoxOverlay` | `src`, `boxes` | `src`, `alt`, `boxes`, `showLabels`, `showConfidence`, `strokeWidth`, `fontSize`, `fit`, `normalized`, `interactive` |
| `geoMap` | — | `viewport`, `markers`, `routes`, `showControls`, `showZoom`, `showCompass`, `showLocate`, `showFullscreen`, `interactive`, `controlPosition`, `clusterMarkers`, `clusterRadius`, `clusterMaxZoom` |
| `graph` | `nodes` | `edges`, `labelStyles`, `showToolbar`, `showSearch`, `showLegend`, `showInspector`, `height` |
| `ontologyGraph` | `ontologyId` | `appId`, `limit`, `allowExpand`, `allowSearch`, `allowPaths`, `allowActions`, `allowCypher`, `allowStyleEdit`, `allowLimitChange`, `showToolbar`, `showLegend`, `height` |
| `calendar` | `events` | `events`, `view`, `date`, `title`, `density`, `editable`, `selectable`, `firstDayOfWeek`, `minTime`, `maxTime`, `slotDuration`, `showWeekends`, `showNowIndicator`, `showAllDay`, `showViewSwitcher`, `locale`, `height`, `responsive`, `compactBreakpoint` |
| `gantt` | `tasks` | `tasks`, `view`, `title`, `density`, `editable`, `draggable`, `resizable`, `showDependencies`, `showProgress`, `showToday`, `showViewSwitcher`, `showTaskList`, `taskListWidth`, `shadeWeekends`, `rowHeight`, `columns`, `height`, `responsive`, `compactBreakpoint` |

Common display values:

- `text.variant`: `body`, `heading`, `label`, `caption`, or `code`; `size`: `xs` through `4xl`; `weight`: `light`, `normal`, `medium`, `semibold`, or `bold`.
- `image.fit` / `filePreview.fit`: `contain`, `cover`, `fill`, `none`, or `scaleDown`.
- `badge.variant`: `default`, `secondary`, `destructive`, or `outline`.
- `userProfile.variant`: `avatar`, `chip`, `row`, `detailed`, or `card`; `avatarSize`: `xs`, `sm`, `md`, `lg`, `xl`, or `2xl`.
- `progress.variant`: `default`, `success`, `warning`, or `error`.
- `diffView.mode`: `split`, `unified`, or `inline`; `kind`: `auto`, `text`, `code`, `markdown`, `json`, or `document`; `markdownMode`: `source` or `rendered`.
- `filePreview.fileType`: `pdf`, `image`, `video`, `audio`, `code`, or `text`. For audio, `variant` can be `conservative`, `waveform`, `orb`, `vortex`, `shader`, `aurora`, or `pulse`; `autoPlay` applies to an animated audio variant.
- Markdown HTML is disabled by validation even if `allowHtml` is requested.
- An iframe is sandboxed. Prefer `srcdoc` for an HTML preview and set a useful `title` and stable height.

### Tables

Encode static `columns` and `data` as `literalJson`; bind workflow- or state-owned rows with `path`.

A column object supports:

```json
{
  "id": "status",
  "header": { "literalString": "Status" },
  "accessor": { "literalString": "status" },
  "width": { "literalString": "10rem" },
  "align": { "literalString": "left" },
  "sortable": { "literalBool": true },
  "hidden": { "literalBool": false }
}
```

### Charts

- Plotly `chartType`: `line`, `bar`, `scatter`, `pie`, `area`, or `histogram`. `series`, `xAxis`, and `yAxis` are raw structured values; advanced `data`, `layout`, and `config` use BoundValue, usually `literalJson` or `path`.
- Nivo `chartType`: `bar`, `line`, `pie`, `radar`, `heatmap`, `scatter`, `funnel`, `treemap`, `sunburst`, `calendar`, `bump`, `areaBump`, `circlePacking`, `network`, `sankey`, `stream`, `swarmplot`, `voronoi`, `waffle`, `marimekko`, `parallelCoordinates`, `radialBar`, `boxplot`, `bullet`, or `chord`.
- Give charts stable `height`. Use `literalJson` for static arrays/objects and `path` for data-model bindings.

### Geo map

Use these JSON shapes inside `literalJson` or bound data:

```json
{
  "viewport": {
    "center": { "latitude": 52.52, "longitude": 13.405 },
    "zoom": 11,
    "bearing": 0,
    "pitch": 0
  },
  "marker": {
    "id": "berlin",
    "coordinate": { "latitude": 52.52, "longitude": 13.405 },
    "color": "blue",
    "label": "Berlin",
    "icon": "map-pin",
    "popup": "Berlin",
    "draggable": false
  },
  "route": {
    "id": "route-1",
    "coordinates": [
      { "latitude": 52.52, "longitude": 13.405 },
      { "latitude": 52.5, "longitude": 13.37 }
    ],
    "color": "#2563eb",
    "width": 4,
    "opacity": 0.9,
    "dashArray": [4, 2],
    "label": "Route"
  }
}
```

`controlPosition` is a map-control position such as `bottom-right`. Marker and route interactions augment the configured action context with `event`, IDs, and coordinates.

### Calendar

`events` resolves to an array of:

```json
{
  "id": "event-1",
  "title": "Planning",
  "start": "2026-07-22T09:00:00+02:00",
  "end": "2026-07-22T10:00:00+02:00",
  "allDay": false,
  "color": "hsl(var(--primary))",
  "description": "Quarterly planning",
  "location": "Room 4",
  "calendarId": "team",
  "editable": true,
  "link": "/planning/event-1",
  "metadata": { "projectId": "p-1" }
}
```

- `view`: `month`, `week`, `day`, or `agenda`.
- `density`: `compact`, `default`, or `comfortable`.
- `date` and event dates use ISO 8601. `firstDayOfWeek` is `0` for Sunday. `slotDuration` is minutes.
- Interactions: `open`, `create`, `update`, `move`, `resize`, and `delete`.

### Gantt

`tasks` resolves to an array of:

```json
{
  "id": "task-1",
  "name": "Design",
  "start": "2026-07-20",
  "end": "2026-07-24",
  "progress": 60,
  "dependencies": [],
  "parent": "phase-1",
  "color": "hsl(var(--primary))",
  "assignee": "Alex",
  "milestone": false,
  "collapsed": false,
  "link": "/tasks/task-1",
  "metadata": { "ticket": "UI-42" }
}
```

- `view`: `day`, `week`, `month`, `quarter`, or `compact`.
- `density`: `compact`, `default`, or `comfortable`.
- Interactions: `open`, `create`, `update`, `move`, `resize`, `delete`, `link`, and `reorder`.

## Interactive

| Type | Required | Accepted props |
|---|---|---|
| `button` | `label` | `label`, `variant`, `size`, `disabled`, `loading`, `icon`, `iconPosition`, `tooltip` |
| `feedback` | — | `mode`, `size`, `title`, `description`, `positiveLabel`, `negativeLabel`, `positiveRating`, `negativeRating`, `showComment`, `commentMode`, `commentLabel`, `commentPlaceholder`, `commentTitle`, `commentDescription`, `commentSubmitLabel`, `commentCancelLabel`, `feedbackId`, `includeState`, `pageContextMode`, `pageContextQueryParamAllowlist`, `pageContextQueryParamDenylist`, `includePageHash`, `successMessage`, `disabled` |
| `appLink` | — | `target`, `label`, `variant`, `size`, `icon`, `iconPosition`, `appId`, `eventId`, `disabled` |
| `textField` | `value` | `value`, `placeholder`, `label`, `helperText`, `error`, `disabled`, `inputType`, `multiline`, `rows`, `maxLength`, `required` |
| `select` | `value`, `options` | `value`, `options`, `placeholder`, `label`, `disabled`, `multiple`, `searchable` |
| `slider` | `value` | `value`, `min`, `max`, `step`, `disabled`, `showValue`, `label` |
| `checkbox` | `checked` | `checked`, `label`, `disabled`, `indeterminate` |
| `switch` | `checked` | `checked`, `label`, `disabled` |
| `radioGroup` | `value`, `options` | `value`, `options`, `disabled`, `orientation`, `label` |
| `dateTimeInput` | `value` | `value`, `mode`, `min`, `max`, `disabled`, `label` |
| `fileInput` | `value` | `value`, `label`, `helperText`, `accept`, `multiple`, `maxSize`, `maxFiles`, `disabled`, `error` |
| `imageInput` | `value` | `value`, `label`, `helperText`, `accept`, `multiple`, `maxSize`, `maxFiles`, `disabled`, `error`, `aspectRatio`, `showPreview` |
| `voiceInput` | `value` | `value`, `label`, `helperText`, `maxDuration`, `autoStop`, `silenceThreshold`, `silenceDuration`, `disabled`, `error`, `visualizer`, `variant`, `size`, `mode`, `invoke`, `color`, `recordingColor`, `resultMode`, `src`, `url` |
| `link` | `href` | `href`, `label`, `route`, `queryParams`, `external`, `target`, `variant`, `underline`, `disabled` |
| `imageLabeler` | `src`, `labels` | `src`, `alt`, `boxes`, `labels`, `disabled`, `showLabels`, `minBoxSize` |
| `imageHotspot` | `src`, `hotspots` | `src`, `alt`, `hotspots`, `showMarkers`, `markerStyle`, `fit`, `normalized`, `showTooltips` |

Interactive values:

- `button.variant`: `default`, `secondary`, `outline`, `ghost`, `destructive`, or `link`; `size`: `sm`, `md`, `lg`, or `icon`.
- `feedback.mode`: `icon`, `compact`, `segmented`, `rating`, or `extended`; `commentMode`: `none`, `inline`, or `modal`.
- `appLink.target`: `config`, `settings`, or `overview`.
- `textField.inputType`: `text`, `email`, `password`, `number`, `tel`, `url`, or `search`.
- `dateTimeInput.mode`: `date`, `time`, or `datetime`.
- `voiceInput.variant`: `conservative`, `waveform`, `orb`, `vortex`, `shader`, `aurora`, or `pulse`; `size`: `sm`, `md`, or `lg`; `mode`: `record` or `stt`; `invoke`: `manual`, `hold`, or `auto`; `resultMode`: `player`, `autoplay`, or `summary`. `visualizer` is a deprecated alias for `variant`.
- `link.external`, `target`, `variant`, and `underline` are raw values. `queryParams` is BoundValue, normally `literalJson`.
- `imageHotspot.markerStyle`: `pulse`, `dot`, `ring`, `square`, `diamond`, or `none`.

## Containers

| Type | Required | Accepted props |
|---|---|---|
| `card` | — | `title`, `description`, `footer`, `hoverable`, `clickable`, `variant`, `padding`, `headerImage`, `headerIcon` |
| `modal` | `open` | `open`, `title`, `description`, `closeOnOverlay`, `closeOnEscape`, `showCloseButton`, `size`, `centered` |
| `tabs` | `value`, `tabs` | `value`, `tabs`, `orientation`, `variant`, `listStyle`, `triggerStyle`, `contentStyle` |
| `accordion` | `items` | `items`, `multiple`, `defaultExpanded`, `collapsible` |
| `drawer` | `open` | `open`, `side`, `title`, `size`, `overlay`, `closable` |
| `tooltip` | `content` | `content`, `side`, `delayMs`, `maxWidth` |
| `popover` | `contentComponentId` | `open`, `contentComponentId`, `side`, `trigger`, `closeOnClickOutside` |

Container structures:

- `tabs.tabs` is a raw array of `{id, label, icon?, disabled?, contentComponentId}`. `label`, `icon`, and `disabled` use BoundValue. `listStyle`, `triggerStyle`, and `contentStyle` are raw Style objects.
- `accordion.items` is a raw array of `{id, title, contentComponentId}`; `title` uses BoundValue.
- `popover.contentComponentId` is a raw component ID. Its `children` identify the trigger.
- `card.variant`: `default`, `bordered`, or `elevated`.
- `modal.size`: `sm`, `md`, `lg`, `xl`, or `full`.
- `drawer.side`: `left`, `right`, `top`, or `bottom`.
- Tooltip/popover `side`: `top`, `right`, `bottom`, or `left`.

## Game and 3D

| Type | Required | Accepted props |
|---|---|---|
| `canvas2d` | `width`, `height` | `width`, `height`, `backgroundColor`, `pixelPerfect` |
| `sprite` | `src`, `x`, `y` | `src`, `x`, `y`, `width`, `height`, `rotation`, `scale`, `opacity`, `flipX`, `flipY`, `zIndex` |
| `shape` | `shapeType`, `x`, `y` | `shapeType`, `x`, `y`, `width`, `height`, `radius`, `points`, `fill`, `stroke`, `strokeWidth` |
| `scene3d` | `width`, `height` | `width`, `height`, `cameraType`, `cameraPosition`, `backgroundColor`, `controlMode`, `fixedView`, `autoRotateSpeed`, `enableControls`, `enableZoom`, `enablePan`, `fov`, `near`, `far`, `target`, `ambientLight`, `directionalLight`, `showGrid`, `showAxes` |
| `model3d` | `src` | `src`, `position`, `rotation`, `scale`, `castShadow`, `receiveShadow`, `animation`, `autoRotate`, `rotateSpeed`, `viewerHeight`, `backgroundColor`, `cameraDistance`, `fov`, `cameraAngle`, `cameraPosition`, `cameraTarget`, `enableControls`, `enableZoom`, `enablePan`, `autoRotateCamera`, `cameraRotateSpeed`, `ambientLight`, `directionalLight`, `fillLight`, `rimLight`, `lightColor`, `lightingPreset`, `showGround`, `groundColor`, `enableReflections`, `environment`, `environmentSource`, `useHdrBackground`, `polyhavenHdri`, `polyhavenResolution`, `hdriUrl`, `groundSize`, `groundOffsetY`, `groundFollowCamera` |
| `dialogue` | `text` | `text`, `speakerName`, `speakerPortraitId`, `typewriter`, `typewriterSpeed` |
| `characterPortrait` | `image` | `image`, `expression`, `position`, `size`, `dimmed` |
| `choiceMenu` | `choices` | `choices`, `title`, `layout` |
| `inventoryGrid` | `items` | `items`, `columns`, `rows`, `cellSize` |
| `healthBar` | `value`, `maxValue` | `value`, `maxValue`, `label`, `showValue`, `fillColor`, `backgroundColor`, `variant` |
| `miniMap` | `width`, `height` | `mapImage`, `width`, `height`, `markers`, `playerX`, `playerY`, `playerRotation` |

Game values and structures:

- `shape.shapeType`: `rectangle`, `circle`, `ellipse`, `polygon`, `line`, or `path`. Encode `points` as `literalJson`.
- `scene3d.cameraType`: `perspective` or `orthographic`; `controlMode`: `orbit`, `fly`, `fixed`, or `auto-rotate`; `fixedView`: `front`, `back`, `left`, `right`, `top`, `bottom`, or `isometric`.
- 3D vectors such as `position`, `rotation`, `cameraPosition`, and `target` are JSON arrays like `[0, 1, 3]`, encoded with `literalJson`.
- `model3d.cameraAngle`: `front`, `side`, `top`, or `isometric`; `lightingPreset`: `neutral`, `warm`, `cool`, `studio`, or `dramatic`; `environmentSource`: `local`, `preset`, `polyhaven`, or `custom`; `polyhavenResolution`: `1k`, `2k`, `4k`, or `8k`.
- `characterPortrait.position`: `left`, `right`, or `center`; `size`: `small`, `medium`, or `large`.
- `choiceMenu.choices` resolves to `[{id, text, disabled?}]`; `inventoryGrid.items` resolves to `[{id, icon, name, quantity?}]`.
- `healthBar.variant`: `bar`, `segmented`, or `circular`.
- `miniMap.markers` resolves to `[{id, x, y, icon?, color?, label?}]`.

## Widget system

| Type | Required | Accepted props |
|---|---|---|
| `widgetInstance` | `instanceId`, `widgetId` | `instanceId`, `widgetId`, `appId`, `inlineWidgetDef`, `exposedPropValues`, `actionBindings`, `styleOverride` |

All `widgetInstance` props are raw wiring data, not BoundValue.

```json
{
  "type": "widgetInstance",
  "widgetId": "stat-card",
  "instanceId": "stat-card-template",
  "inlineWidgetDef": {
    "name": "Stat Card",
    "rootComponentId": "stat-root",
    "components": [
      {
        "id": "stat-root",
        "component": {
          "type": "card",
          "children": { "explicitList": ["stat-label"] }
        }
      },
      {
        "id": "stat-label",
        "component": {
          "type": "text",
          "content": { "path": "$item.label", "defaultValue": "Metric" }
        }
      }
    ],
    "exposedProps": [
      {
        "id": "accent",
        "label": "Accent",
        "targetComponentId": "stat-root",
        "propertyPath": "style.className",
        "propType": "TailwindClass"
      }
    ]
  },
  "exposedPropValues": { "accent": "border-l-4 border-primary" },
  "actionBindings": {}
}
```

Supported exposed prop types include `String`, `Number`, `Boolean`, `Color`, `TailwindClass`, `StyleObject`, and `BoundValue`. An inline widget's component IDs are local to its own tree.
