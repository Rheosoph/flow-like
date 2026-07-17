// UI — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === UI/Component ===

/**
 * Creates an A2UI component with ID, style, and component data
 * @param componentId — Unique identifier for the component
 * @param componentType (optional) — Component type (row, column, text, button, etc.)
 * @param props — Component properties as JSON
 * @param style — Optional style for the component
 * @returns component — The created component
 * @impure has side effects / drives control flow
 */
declare function a2uiCreateComponent({ componentId: string, componentType?: string, props: Struct, style: Struct }): Struct;


// === UI/Container ===

/**
 * Creates a new widget instance for dynamic insertion into containers. Select a widget from the dropdown to auto-generate input pins for its exposed props and customizations.
 * @param widgetSelector — Select a widget from the project
 * @param instanceId — Unique ID for this widget instance
 * @returns elementRef — Element reference for the instantiated widget (connect to Push To Container)
 * @impure has side effects / drives control flow
 */
declare function a2uiInstantiateWidget({ widgetSelector: string, instanceId: string }): Struct;

/**
 * Dynamically adds an element to a container's children list
 * @param containerRef — Reference to the container element (ID or element object)
 * @param elementRef — Reference to the element to add (e.g. from Instantiate Widget)
 * @param position (optional) — Position to insert: -1 for end, 0 for start, or specific index
 * @returns success — Whether the element was successfully added
 * @impure has side effects / drives control flow
 */
declare function a2uiPushToContainer({ containerRef: Struct, elementRef: Struct, position?: int }): bool;

/**
 * Removes an element from a container's children list
 * @param containerId — ID of the container element to remove from
 * @param elementId — ID of the element to remove
 * @returns success — Whether the element was successfully removed
 * @impure has side effects / drives control flow
 */
declare function a2uiRemoveFromContainer({ containerId: string, elementId: string }): bool;


// === UI/Data ===

/**
 * Updates data in a surface's data model
 * @param surfaceId (optional) — ID of the surface to update
 * @param path — Data path to update (e.g., 'user/name')
 * @param value — New value to set at the path
 * @impure has side effects / drives control flow
 */
declare function a2uiDataUpdate({ surfaceId?: string, path: string, value: any }): void;

/**
 * Requests element values from the frontend before processing
 * @param elementIds — Array of element IDs to request (e.g., ['main/input-field', 'main/checkbox'])
 * @impure has side effects / drives control flow
 */
declare function a2uiRequestElements({ elementIds: string[] }): void;

/**
 * Updates or inserts an element value in the frontend
 * @param elementId — ID of the element to update (e.g., 'main/status-text')
 * @param value — New value for the element
 * @impure has side effects / drives control flow
 */
declare function a2uiUpsertElement({ elementId: string, value: any }): void;


// === UI/Elements ===

/**
 * Clones an existing element and adds it to a container
 * @param sourceElement — The element to clone (format: surfaceId/elementId)
 * @param newElementId — ID for the cloned element
 * @param parentId — Container to add the cloned element to (optional, uses source parent if empty)
 * @param index — Position in parent container (-1 for end)
 * @returns clonedElementRef — Reference to the cloned element
 * @impure has side effects / drives control flow
 */
declare function a2uiCloneElement({ sourceElement: string, newElementId: string, parentId: string, index: int }): string;

/**
 * Creates a new element and adds it to a parent container
 * @param surfaceId — The surface to create the element in
 * @param parentId — Parent element ID string or element object from Get Element
 * @param elementId — Unique ID for the new element
 * @param componentType — The component type (e.g., 'Text', 'Button', 'Container')
 * @param props — Component properties as JSON object
 * @param index — Optional index to insert at (default: append at end)
 * @returns createdId — The ID of the created element
 * @impure has side effects / drives control flow
 */
declare function a2uiCreateElement({ surfaceId: string, parentId: any, elementId: string, componentType: string, props: any, index: int }): string;

/**
 * Gets an element's data from the page
 * @param elementRef — Reference to the page element
 * @returns element — The element data
 * @returns exists — Whether the element exists
 */
declare function a2uiGetElement({ elementRef: string }): { element: Struct, exists: bool };

/**
 * Gets the text content of an element
 * @param elementRef — Reference to the text element
 * @returns text — The text content of the element
 * @returns exists — Whether the element exists
 */
declare function a2uiGetElementText({ elementRef: Struct }): { text: string, exists: bool };

/**
 * Gets the value of an input element
 * @param elementRef — Reference to the input element
 * @returns value — The current value of the input
 * @returns exists — Whether the element exists
 */
declare function a2uiGetElementValue({ elementRef: Struct }): { value: any, exists: bool };

/**
 * Removes an element from the page
 * @param surfaceId — The surface containing the element
 * @param elementId — Element ID string or element object from Get Element
 * @impure has side effects / drives control flow
 */
declare function a2uiRemoveElement({ surfaceId: string, elementId: any }): void;

/**
 * Dynamically sets the action of an interactive element (button, link, etc.)
 * @param elementRef — Reference to the element (ID string or element object from Get Element)
 * @param actionType (optional) — Type of action: navigate_page, external_link, workflow_event, or clear to remove action
 * @param route — For navigate_page: the route path (e.g., /about, /products/123)
 * @param queryParams — For navigate_page: optional JSON object of query parameters
 * @param url — For external_link: the external URL to open
 * @param nodeId — For workflow_event: the ID of the workflow node to trigger
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementAction({ elementRef: Struct, actionType?: string, route: string, queryParams: string, url: string, nodeId: string }): void;

/**
 * Enables or disables an element
 * @param elementRef — Element ID string or element object from Get Element
 * @param disabled — Whether the element should be disabled
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementDisabled({ elementRef: Struct, disabled: bool }): void;

/**
 * Sets the loading state of a button element
 * @param elementRef — Element ID string or element object from Get Element
 * @param loading — Whether the element is in loading state
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementLoading({ elementRef: Struct, loading: bool }): void;

/**
 * Sets style properties of an element
 * @param elementRef — Element ID string or element object from Get Element
 * @param style — Style properties to set (JSON object)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementStyle({ elementRef: any, style: Struct }): void;

/**
 * Sets the text content of an element
 * @param elementRef — Reference to the text element (ID string or element object from Get Element)
 * @param text — The new text content
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementText({ elementRef: Struct, text: string }): void;

/**
 * Sets the value of an input element
 * @param elementRef — Element ID string or element object from Get Element
 * @param value — The new value for the input
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementValue({ elementRef: Struct, value: string }): void;

/**
 * Shows or hides an element
 * @param elementRef — Element ID string or element object from Get Element
 * @param visible — Whether the element should be visible
 * @impure has side effects / drives control flow
 */
declare function a2uiSetElementVisibility({ elementRef: any, visible: bool }): void;


// === UI/Elements/Button ===

/**
 * Gets whether a button element is disabled
 * @param elementRef — Reference to the button element
 * @returns disabled — Whether the button is disabled
 */
declare function a2uiGetButtonDisabled({ elementRef: Struct }): bool;

/**
 * Gets the label text of a button element
 * @param elementRef — Reference to the button element
 * @returns label — The button's label text
 */
declare function a2uiGetButtonLabel({ elementRef: Struct }): string;

/**
 * Gets whether a button element is in loading state
 * @param elementRef — Reference to the button element
 * @returns loading — Whether the button is loading
 */
declare function a2uiGetButtonLoading({ elementRef: Struct }): bool;

/**
 * Sets the label text of a button element
 * @param elementRef — Element ID string or element object from Get Element
 * @param label — The new label text
 * @impure has side effects / drives control flow
 */
declare function a2uiSetButtonLabel({ elementRef: Struct, label: string }): void;


// === UI/Elements/Calendar ===

/**
 * Add, remove, or update calendar events and view configuration
 * @param elementRef — Reference to the calendar element
 * @param operation (optional) — What operation to perform
 * @param events — Array of events
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateCalendar({ elementRef: Struct, operation?: string, events: Struct[] }): void;


// === UI/Elements/Charts ===

/**
 * Push data to a Nivo or Plotly chart. Select JSON for pre-formatted data or CSV for auto-transformation.
 * @param elementRef — Reference to the chart element
 * @param library (optional) — Nivo or Plotly
 * @param format (optional) — Data format: JSON (passthrough) or CSV (auto-transform)
 * @param data — Chart data as JSON array/object or JSON string
 * @impure has side effects / drives control flow
 */
declare function a2uiPushCsvToChart({ elementRef: Struct, library?: string, format?: string, data: Struct }): void;

/**
 * Sets the layout configuration for a Plotly chart
 * @param elementRef — Reference to the chart element (ID or element object)
 * @param layout — Chart layout object (Plotly layout format)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetChartLayout({ elementRef: Struct, layout: any }): void;

/**
 * Configure Nivo chart appearance
 * @param elementRef — Reference to the NivoChart element
 * @param chartType (optional) — Type of chart to style
 * @param barStyle — Bar chart styling options
 * @impure has side effects / drives control flow
 */
declare function a2uiSetChartStyle({ elementRef: Struct, chartType?: string, barStyle: Struct }): void;

/**
 * Sets configuration options for a Nivo chart
 * @param elementRef — Reference to the Nivo chart element
 * @param config — Full Nivo configuration object (merged with defaults)
 * @param chartType (optional) — Chart type (bar, line, pie, radar, etc.)
 * @param colors (optional) — Color scheme name or array of colors
 * @param height (optional) — Chart height (e.g., '400px')
 * @impure has side effects / drives control flow
 */
declare function a2uiSetNivoConfig({ elementRef: Struct, config: any, chartType?: string, colors?: any, height?: string }): void;


// === UI/Elements/Charts/Agent ===

/**
 * Uses an LLM to write and run SQL against a DataFusion session, returning chart-ready struct data.
 * @param model — LLM model (Bit)
 * @param session — DataFusion session to query
 * @param table — Table name within the session to query
 * @param description — Natural language task (e.g. 'monthly sales by region')
 * @param chartType (optional) — Target chart type
 * @param element — Chart element reference (from Get Element) to bind the data to
 * @returns data — Query results as an array of row structs (chart-ready)
 * @returns sql — Generated SQL query
 * @returns explanation — AI explanation of the query
 * @impure has side effects / drives control flow
 */
declare function a2uiChartDataAgent({ model: Struct, session: Struct, table: string, description: string, chartType?: string, element: Struct }): { data: Struct, sql: string, explanation: string };


// === UI/Elements/Checkbox ===

/**
 * Set or toggle checkbox/switch checked state
 * @param elementRef — Reference to checkbox or switch element
 * @param operation (optional) — What operation to perform
 * @param checked (optional) — New checked state
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateToggle({ elementRef: Struct, operation?: string, checked?: bool }): void;


// === UI/Elements/Containers ===

/**
 * Removes all children from a container element
 * @param containerRef — Reference to the container element (ID or element object)
 * @impure has side effects / drives control flow
 */
declare function a2uiClearChildren({ containerRef: any }): void;

/**
 * Gets a child element at a specific index from a container
 * @param containerRef — Reference to the container element
 * @param index — The index of the child to get (0-based)
 * @returns child — The child element at the specified index
 * @returns childId — The ID of the child element
 * @returns found — Whether a child was found at the index
 */
declare function a2uiGetChildAtIndex({ containerRef: string, index: int }): { child: Struct, childId: string, found: bool };

/**
 * Appends a child element to a container
 * @param containerRef — Reference to the container element (ID or element object)
 * @param childRef — Reference to the child element to append
 * @impure has side effects / drives control flow
 */
declare function a2uiPushChild({ containerRef: any, childRef: any }): void;

/**
 * Inserts a child element at a specific index in a container
 * @param containerRef — Reference to the container element (ID or element object)
 * @param childRef — Reference to the child element to insert
 * @param index — The index at which to insert the child (0-based)
 * @impure has side effects / drives control flow
 */
declare function a2uiPushChildAtIndex({ containerRef: any, childRef: any, index: int }): void;

/**
 * Removes a child element at a specific index from a container
 * @param containerRef — Reference to the container element (ID or element object)
 * @param index — The index of the child to remove (0-based)
 * @impure has side effects / drives control flow
 */
declare function a2uiRemoveChildAtIndex({ containerRef: any, index: int }): void;


// === UI/Elements/Display ===

/**
 * Sets the content/text of a badge element
 * @param elementRef — Reference to the badge element
 * @param content — The badge content (text or number)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetBadgeContent({ elementRef: Struct, content: string }): void;

/**
 * Sets the original and modified content of a diff view element
 * @param elementRef — Reference to the diff view element
 * @param original — Left / old content (text or document URL)
 * @param modified — Right / new content (text or document URL)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetDiffContent({ elementRef: Struct, original: string, modified: string }): void;

/**
 * Sets the icon name of an icon element
 * @param elementRef — Reference to the icon element
 * @param name — The icon name (e.g., 'check', 'x', 'star')
 * @impure has side effects / drives control flow
 */
declare function a2uiSetIcon({ elementRef: Struct, name: string }): void;

/**
 * Sets the markdown content of a markdown element
 * @param elementRef — Reference to the markdown element
 * @param content — The markdown content
 * @impure has side effects / drives control flow
 */
declare function a2uiSetMarkdownContent({ elementRef: Struct, content: string }): void;

/**
 * Sets the value of a progress bar (0-100)
 * @param elementRef — Reference to the progress bar element
 * @param value — Progress value (0-100)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetProgress({ elementRef: Struct, value: float }): void;


// === UI/Elements/Files ===

/**
 * Gets uploaded files, signed URLs, and FlowPaths from an A2UI fileInput or voiceInput element
 * @param elementRef — File or voice input element ID or element object from Get Element
 * @returns files — Uploaded file objects
 * @returns signedUrls — Signed or local URLs for the uploaded files
 * @returns flowPaths — Temporary FlowPaths for uploaded files when available
 * @returns exists — Whether the file input element exists
 */
declare function a2uiGetFileInputFiles({ elementRef: Struct }): { files: Struct[], signedUrls: string[], flowPaths: Struct[], exists: bool };


// === UI/Elements/Game ===

/**
 * Update any property of a 3D model
 * @param elementRef — Reference to the 3D model element
 * @param property (optional) — Which property to update
 * @param src — GLTF/GLB model URL
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateModel3d({ elementRef: Struct, property?: string, src: string }): void;

/**
 * Update any property of a 3D scene
 * @param elementRef — Reference to the 3D scene element
 * @param property (optional) — Which property to update
 * @param camera — Camera type, position, and target
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateScene3d({ elementRef: Struct, property?: string, camera: Struct }): void;

/**
 * Update any property of a sprite
 * @param elementRef — Reference to the sprite element
 * @param property (optional) — Which property to update
 * @param src — Image URL
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateSprite({ elementRef: Struct, property?: string, src: string }): void;


// === UI/Elements/Gantt ===

/**
 * Add, remove, or update gantt tasks, dependencies and configuration
 * @param elementRef — Reference to the gantt element
 * @param operation (optional) — What operation to perform
 * @param tasks — Array of tasks
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateGantt({ elementRef: Struct, operation?: string, tasks: Struct[] }): void;


// === UI/Elements/GeoMap ===

/**
 * Update markers, routes, or viewport of a map
 * @param elementRef — Reference to the map element
 * @param property (optional) — Which property to update
 * @param markers — Array of map markers
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateGeomap({ elementRef: Struct, property?: string, markers: Struct[] }): void;


// === UI/Elements/Get ===

/**
 * Gets the src URL of an iframe element
 * @param elementRef — Reference to the iframe element
 * @returns src — The iframe's source URL
 */
declare function a2uiGetIframeSrc({ elementRef: Struct }): string;

/**
 * Gets the content text of a tooltip element
 * @param elementRef — Reference to the tooltip element
 * @returns content — The tooltip's content text
 * @returns side — The tooltip's side position (top, bottom, left, right)
 */
declare function a2uiGetTooltipContent({ elementRef: Struct }): { content: string, side: string };


// === UI/Elements/Hotspot ===

/**
 * Add, remove, or manage hotspots on an ImageHotspot element
 * @param elementRef — Reference to the ImageHotspot element
 * @param operation (optional) — What operation to perform
 * @param hotspot — Hotspot to add
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateHotspot({ elementRef: Struct, operation?: string, hotspot: Struct }): void;


// === UI/Elements/Input ===

/**
 * Clears the value of an input element
 * @param elementRef — Element ID string or element object from Get Element
 * @impure has side effects / drives control flow
 */
declare function a2uiClearInput({ elementRef: any }): void;

/**
 * Gets the placeholder text of an input element
 * @param elementRef — Reference to the input element
 * @returns placeholder — The input's placeholder text
 */
declare function a2uiGetInputPlaceholder({ elementRef: Struct }): string;

/**
 * Sets the placeholder text of an input element
 * @param elementRef — Element ID string or element object from Get Element
 * @param placeholder — The new placeholder text
 * @impure has side effects / drives control flow
 */
declare function a2uiSetInputPlaceholder({ elementRef: Struct, placeholder: string }): void;

/**
 * Sets the error state or message of a text field
 * @param elementRef — Reference to the text field element
 * @param error — Error message (empty string clears error)
 * @impure has side effects / drives control flow
 */
declare function a2uiSetTextfieldError({ elementRef: Struct, error: string }): void;


// === UI/Elements/Labeler ===

/**
 * Add, remove, or manage bounding boxes on an ImageLabeler element
 * @param elementRef — Reference to the ImageLabeler element
 * @param operation (optional) — What operation to perform
 * @param box — Bounding box to add
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateLabeler({ elementRef: Struct, operation?: string, box: Struct }): void;


// === UI/Elements/Media ===

/**
 * Sets the source URL of an iframe element
 * @param elementRef — Reference to the iframe element
 * @param src — The URL to load in the iframe
 * @impure has side effects / drives control flow
 */
declare function a2uiSetIframeSrc({ elementRef: Struct, src: string }): void;

/**
 * Sets raw HTML content of an iframe element for previewing generated HTML
 * @param elementRef — Reference to the iframe element
 * @param html — Raw HTML content to render inside the iframe
 * @impure has side effects / drives control flow
 */
declare function a2uiSetIframeSrcdoc({ elementRef: Struct, html: string }): void;

/**
 * Signs a FlowPath and sets it as the source for image, video, avatar, iframe, lottie, or file preview elements
 * @param elementRef — Reference to the media element
 * @param file — FlowPath to sign and use as the element source
 * @param expiration (optional) — Expiration time for the signed URL
 * @returns signedUrl — The generated signed URL
 * @returns mimeType — Detected MIME type from the FlowPath extension
 * @returns mediaKind — Detected media kind: image, video, audio, pdf, text, or file
 * @impure has side effects / drives control flow
 */
declare function a2uiSetMediaSource({ elementRef: Struct, file: Struct, expiration?: int }): { signedUrl: string, mimeType: string, mediaKind: string };


// === UI/Elements/Overlay ===

/**
 * Set, push, or clear bounding boxes on a BoundingBoxOverlay element
 * @param elementRef — Reference to the BoundingBoxOverlay element
 * @param operation (optional) — What operation to perform
 * @param boxes — Array of detection bounding boxes
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateOverlay({ elementRef: Struct, operation?: string, boxes: Struct[] }): void;


// === UI/Elements/Query ===

/**
 * Gets all child elements of a container
 * @param elementRef — Reference to the container element
 * @returns children — Array of child elements
 * @returns childIds — Array of child element IDs
 * @returns count — Number of children
 */
declare function a2uiQueryChildren({ elementRef: string }): { children: Struct, childIds: string[], count: int };

/**
 * Gets elements whose IDs match a pattern
 * @param pattern — The pattern to match element IDs against
 * @param matchType — How to match: 'starts_with', 'ends_with', 'contains', or 'exact'
 * @returns elements — Array of matching elements
 * @returns elementIds — Array of matching element IDs
 * @returns count — Number of matching elements
 */
declare function a2uiQueryElementsById({ pattern: string, matchType: string }): { elements: Struct, elementIds: string[], count: int };

/**
 * Gets all elements of a specific component type
 * @param componentType — The type of component to query (e.g., 'button', 'text', 'textField')
 * @returns elements — Array of matching elements
 * @returns count — Number of matching elements
 */
declare function a2uiQueryElementsByType({ componentType: string }): { elements: Struct, count: int };

/**
 * Gets the parent element of an element
 * @param elementRef — Reference to the element to find parent of
 * @returns parent — The parent element data
 * @returns parentId — ID of the parent element
 * @returns hasParent — Whether a parent was found
 */
declare function a2uiQueryParent({ elementRef: string }): { parent: Struct, parentId: string, hasParent: bool };


// === UI/Elements/Select ===

/**
 * Gets the selected value of a select element
 * @param elementRef — Reference to the select element
 * @returns value — The currently selected value
 * @returns hasSelection — Whether a value is selected
 */
declare function a2uiGetSelectValue({ elementRef: Struct }): { value: string, hasSelection: bool };

/**
 * Sets the available options in a select element
 * @param elementRef — Reference to the select element
 * @param options — Array of options [{value, label}] or simple strings
 * @impure has side effects / drives control flow
 */
declare function a2uiSetSelectOptions({ elementRef: Struct, options: any }): void;

/**
 * Sets the selected value of a select element
 * @param elementRef — Element ID string or element object from Get Element
 * @param value — The value to select
 * @impure has side effects / drives control flow
 */
declare function a2uiSetSelectValue({ elementRef: Struct, value: string }): void;


// === UI/Elements/Set ===

/**
 * Sets the content text of a tooltip element
 * @param elementRef — Reference to the tooltip element
 * @param content — The content text to set
 * @impure has side effects / drives control flow
 */
declare function a2uiSetTooltipContent({ elementRef: Struct, content: string }): void;


// === UI/Elements/Slider ===

/**
 * Sets the value of a slider element
 * @param elementRef — Reference to the slider element
 * @param value — The new slider value
 * @impure has side effects / drives control flow
 */
declare function a2uiSetSliderValue({ elementRef: Struct, value: float }): void;


// === UI/Elements/Table ===

/**
 * Add, remove, or update table data and structure
 * @param elementRef — Reference to the table element
 * @param operation (optional) — What operation to perform
 * @param data — Array of row objects
 * @impure has side effects / drives control flow
 */
declare function a2uiUpdateTable({ elementRef: Struct, operation?: string, data: Struct }): void;

/**
 * Push CSV or Table data directly to a table element
 * @param elementRef — Reference to the table element
 * @param csv — CSV text with headers
 * @param table — Table data from DataFusion query
 * @param delimiter (optional) — CSV delimiter (default: comma)
 * @impure has side effects / drives control flow
 */
declare function a2uiWriteCsvToTable({ elementRef: Struct, csv: string, table: Struct, delimiter?: string }): void;


// === UI/Navigation ===

/**
 * Closes an open dialog. If no dialog ID is specified, closes the topmost dialog.
 * @param dialogId — Optional ID of the specific dialog to close. If empty, closes the topmost dialog.
 * @impure has side effects / drives control flow
 */
declare function a2uiCloseDialog({ dialogId: string }): void;

/**
 * Gets the current page route from the execution context
 * @returns route — The current route path
 * @impure has side effects / drives control flow
 */
declare function a2uiGetCurrentRoute(): string;

/**
 * Gets query parameters from the current URL
 * @param paramName — The name of the query parameter to get (optional - if empty, returns all params)
 * @returns value — The parameter value (string if param_name specified, object if all params)
 * @returns exists — Whether the parameter exists
 * @impure has side effects / drives control flow
 */
declare function a2uiGetQueryParams({ paramName: string }): { value: any, exists: bool };

/**
 * Gets route parameters from the current URL
 * @param paramName — The name of the route parameter to get (optional - if empty, returns all params)
 * @returns value — The parameter value (string if param_name specified, object if all params)
 * @returns exists — Whether the parameter exists
 * @impure has side effects / drives control flow
 */
declare function a2uiGetRouteParams({ paramName: string }): { value: any, exists: bool };

/**
 * Navigates to a page route
 * @param route — The route to navigate to (e.g., /dashboard, /users/123)
 * @param queryParams — Optional query parameters as key-value pairs (e.g., {"tab": "settings", "id": "123"})
 * @param replace — If true, replaces the current history entry instead of adding a new one
 * @impure has side effects / drives control flow
 */
declare function a2uiNavigateTo({ route: string, queryParams: Struct, replace: bool }): void;

/**
 * Opens a route/page as a modal dialog overlay
 * @param route — The route path to open in the dialog (e.g., /settings, /edit/123)
 * @param title — Optional dialog title (shown in header)
 * @param queryParams — Optional JSON object of query parameters to pass to the route
 * @param dialogId — Optional unique ID for the dialog (for closing specific dialogs)
 * @impure has side effects / drives control flow
 */
declare function a2uiOpenDialog({ route: string, title: string, queryParams: string, dialogId: string }): void;

/**
 * Sets or updates a query parameter in the URL
 * @param key — The query parameter key to set
 * @param value — The value to set (empty string removes the param)
 * @param replace — If true, replaces the current history entry instead of adding a new one
 * @impure has side effects / drives control flow
 */
declare function a2uiSetQueryParam({ key: string, value: string, replace: bool }): void;

/**
 * Decodes a URL-encoded (percent-encoded) string
 * @param input — The URL-encoded string to decode
 * @returns decoded — The decoded string
 * @returns success — Whether the decoding was successful
 */
declare function a2uiUrlDecode({ input: string }): { decoded: string, success: bool };

/**
 * Encodes a string for safe use in URLs (percent-encoding)
 * @param input — The string to URL-encode
 * @returns encoded — The URL-encoded string
 */
declare function a2uiUrlEncode({ input: string }): string;


// === UI/State ===

/**
 * Gets a value from global state by key
 * @param key — The key to retrieve from global state
 * @returns value — The value stored at the key
 * @returns exists — Whether the key exists in global state
 */
declare function a2uiGetGlobalState({ key: string }): { value: any, exists: bool };

/**
 * Gets a value from page-local state by key
 * @param key — The key to retrieve from page state
 * @returns value — The value stored at the key
 * @returns exists — Whether the key exists in page state
 */
declare function a2uiGetPageState({ key: string }): { value: any, exists: bool };

/**
 * Sets a value in global state by key
 * @param key — The key to store the value at
 * @param value — The value to store
 * @impure has side effects / drives control flow
 */
declare function a2uiSetGlobalState({ key: string, value: any }): void;

/**
 * Sets a value in page-local state by key
 * @param key — The key to store the value at
 * @param value — The value to store
 * @impure has side effects / drives control flow
 */
declare function a2uiSetPageState({ key: string, value: any }): void;


// === UI/Surface ===

/**
 * Sends a surface to the frontend to begin rendering
 * @param surface — The surface to render
 * @param components — Array of components to include
 * @param dataModel — Initial data model for bindings
 * @impure has side effects / drives control flow
 */
declare function a2uiBeginRendering({ surface: Struct, components: Struct[], dataModel: Struct }): void;

/**
 * Creates a new A2UI surface with an ID and root component
 * @param surfaceId (optional) — Unique identifier for the surface
 * @param rootComponentId (optional) — ID of the root component in the surface
 * @param catalogId — Optional custom component catalog
 * @returns surface — The created surface for adding components
 * @impure has side effects / drives control flow
 */
declare function a2uiCreateSurface({ surfaceId?: string, rootComponentId?: string, catalogId: string }): Struct;

/**
 * Removes a surface from the frontend
 * @param surfaceId (optional) — ID of the surface to delete
 * @impure has side effects / drives control flow
 */
declare function a2uiDeleteSurface({ surfaceId?: string }): void;

/**
 * Sets or clears scoped custom CSS for a custom UI surface at runtime
 * @param surfaceId (optional) — ID of the custom UI surface to update
 * @param customCss (optional) — CSS to apply to the surface. Leave empty to clear it.
 * @impure has side effects / drives control flow
 */
declare function a2uiSetSurfaceCustomCss({ surfaceId?: string, customCss?: string }): void;

/**
 * Shows the current frontend screen while the workflow continues running
 * @impure has side effects / drives control flow
 */
declare function a2uiShowScreen(): void;

/**
 * Updates components in an existing surface
 * @param surfaceId (optional) — ID of the surface to update
 * @param components — Components to add or update
 * @impure has side effects / drives control flow
 */
declare function a2uiSurfaceUpdate({ surfaceId?: string, components: Struct[] }): void;

