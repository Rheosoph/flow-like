// UI — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ui {
    // === UI/Component ===

    /**
     * Creates an A2UI component with ID, style, and component data
     * @node a2ui_create_component @alias a2uiCreateComponent
     * @param componentId — Unique identifier for the component
     * @param componentType (optional) — Component type (row, column, text, button, etc.)
     * @param props — Component properties as JSON
     * @param style — Optional style for the component
     * @returns component — The created component
     * @impure has side effects / drives control flow
     */
    function createComponent({ componentId: string, componentType?: string, props: Struct, style: Struct }): Struct;

    // === UI/Container ===

    /**
     * Creates a new widget instance for dynamic insertion into containers. The dropdown lists project widgets and widgets from packages added to the project; selecting one auto-generates typed input pins (exposed props and customizations for project widgets, contract inputs for package widgets).
     * @node a2ui_instantiate_widget @alias a2uiInstantiateWidget
     * @param widgetSelector — Select a widget from the project or from packages added to the project
     * @param instanceId — Unique ID for this widget instance
     * @returns elementRef — Element reference for the instantiated widget (connect to Push To Container)
     * @impure has side effects / drives control flow
     */
    function instantiateWidget({ widgetSelector: string, instanceId: string }): Struct;

    /**
     * Dynamically adds an element to a container's children list
     * @node a2ui_push_to_container @alias a2uiPushToContainer
     * @param containerRef — Reference to the container element (ID or element object)
     * @param elementRef — Reference to the element to add (e.g. from Instantiate Widget)
     * @param position (optional) — Position to insert: -1 for end, 0 for start, or specific index
     * @returns success — Whether the element was successfully added
     * @impure has side effects / drives control flow
     */
    function pushToContainer({ containerRef: Struct, elementRef: Struct, position?: int }): bool;

    /**
     * Removes an element from a container's children list
     * @node a2ui_remove_from_container @alias a2uiRemoveFromContainer
     * @param containerId — ID of the container element to remove from
     * @param elementId — ID of the element to remove
     * @returns success — Whether the element was successfully removed
     * @impure has side effects / drives control flow
     */
    function removeFromContainer({ containerId: string, elementId: string }): bool;

    /**
     * Resolves an element inside a widget instance (from Instantiate Widget). The output plugs into any element node (Set Element Value, Update GeoMap, Push CSV To Chart, …).
     * @node a2ui_widget_get_element @alias a2uiWidgetGetElement
     * @param elementRef — Widget instance reference (from Instantiate Widget)
     * @param elementId — ID of the element inside the widget (e.g. 'chart-1')
     * @returns element — The element reference (connect to element nodes)
     * @returns exists — Whether the element exists in the widget
     */
    function widgetGetElement({ elementRef: Struct, elementId: string }): { element: Struct, exists: bool };

    /**
     * Reads a typed query result from a package widget instance. Connect Element Ref from Instantiate Widget, or Element from Get Element for a widget placed in the visual builder, then select a contract query.
     * @node a2ui_widget_query @alias a2uiWidgetQuery
     * @param elementRef — Package widget reference from Instantiate Widget, or a visual-builder widget from Get Element
     * @param query — Contract query to run on the widget instance
     * @returns value — The query result, typed by the contract's result schema
     * @impure has side effects / drives control flow
     */
    function widgetQuery({ elementRef: Struct, query: string }): any;

    /**
     * Sets the text of an element inside a widget instance (from Instantiate Widget) before it is pushed to the frontend
     * @node a2ui_widget_set_text @alias a2uiWidgetSetText
     * @param elementRef — Widget instance reference (from Instantiate Widget)
     * @param elementId — ID of the element inside the widget (e.g. 'title-text')
     * @param text (optional) — The text to set
     * @returns elementRefOut — The updated widget instance reference (connect to Push Widget / Push To Container)
     * @impure has side effects / drives control flow
     */
    function widgetSetText({ elementRef: Struct, elementId: string, text?: string }): Struct;

    /**
     * Sends a typed input patch to a package widget instance. Connect the Element Ref from Instantiate Widget to generate one optional pin per contract input; only set pins are included in the patch.
     * @node a2ui_widget_update_inputs @alias a2uiWidgetUpdateInputs
     * @param elementRef — Element reference of a package widget instance (from Instantiate Widget)
     * @impure has side effects / drives control flow
     */
    function widgetUpdateInputs({ elementRef: Struct }): void;

    // === UI/Data ===

    /**
     * Updates data in a surface's data model
     * @node a2ui_data_update @alias a2uiDataUpdate
     * @param surfaceId (optional) — ID of the surface to update
     * @param path — Data path to update (e.g., 'user/name')
     * @param value — New value to set at the path
     * @impure has side effects / drives control flow
     */
    function dataUpdate({ surfaceId?: string, path: string, value: any }): void;

    /**
     * Requests element values from the frontend before processing
     * @node a2ui_request_elements @alias a2uiRequestElements
     * @param elementIds — Array of element IDs to request (e.g., ['main/input-field', 'main/checkbox'])
     * @impure has side effects / drives control flow
     */
    function requestElements({ elementIds: string[] }): void;

    /**
     * Updates or inserts an element value in the frontend
     * @node a2ui_upsert_element @alias a2uiUpsertElement
     * @param elementId — ID of the element to update (e.g., 'main/status-text')
     * @param value — New value for the element
     * @impure has side effects / drives control flow
     */
    function upsertElement({ elementId: string, value: any }): void;

    // === UI/Elements ===

    /**
     * Clones an existing element and adds it to a container
     * @node a2ui_clone_element @alias a2uiCloneElement
     * @param sourceElement — The element to clone (format: surfaceId/elementId)
     * @param newElementId — ID for the cloned element
     * @param parentId — Container to add the cloned element to (optional, uses source parent if empty)
     * @param index — Position in parent container (-1 for end)
     * @returns clonedElementRef — Reference to the cloned element
     * @impure has side effects / drives control flow
     */
    function cloneElement({ sourceElement: string, newElementId: string, parentId: string, index: int }): string;

    /**
     * Creates a new element and adds it to a parent container
     * @node a2ui_create_element @alias a2uiCreateElement
     * @param surfaceId — The surface to create the element in
     * @param parentId — Parent element ID string or element object from Get Element
     * @param elementId — Unique ID for the new element
     * @param componentType — The component type (e.g., 'Text', 'Button', 'Container')
     * @param props — Component properties as JSON object
     * @param index — Optional index to insert at (default: append at end)
     * @returns createdId — The ID of the created element
     * @impure has side effects / drives control flow
     */
    function createElement({ surfaceId: string, parentId: any, elementId: string, componentType: string, props: any, index: int }): string;

    /**
     * Gets an element's data from the page
     * @node a2ui_get_element @alias a2uiGetElement
     * @param elementRef — Reference to the page element
     * @returns element — The element data
     * @returns exists — Whether the element exists
     */
    function getElement({ elementRef: string }): { element: Struct, exists: bool };

    /**
     * Gets the text content of an element
     * @node a2ui_get_element_text @alias a2uiGetElementText
     * @param elementRef — Reference to the text element
     * @returns text — The text content of the element
     * @returns exists — Whether the element exists
     */
    function getElementText({ elementRef: Struct }): { text: string, exists: bool };

    /**
     * Gets the value of an input element
     * @node a2ui_get_element_value @alias a2uiGetElementValue
     * @param elementRef — Reference to the input element
     * @returns value — The current value of the input
     * @returns exists — Whether the element exists
     */
    function getElementValue({ elementRef: Struct }): { value: any, exists: bool };

    /**
     * Removes an element from the page
     * @node a2ui_remove_element @alias a2uiRemoveElement
     * @param surfaceId — The surface containing the element
     * @param elementId — Element ID string or element object from Get Element
     * @impure has side effects / drives control flow
     */
    function removeElement({ surfaceId: string, elementId: any }): void;

    /**
     * Dynamically sets the legacy default action or a named event action of an interactive element
     * @node a2ui_set_element_action @alias a2uiSetElementAction
     * @param elementRef — Reference to the element (ID string or element object from Get Element)
     * @param eventName (optional) — Optional named component event (for example click, change, open, or delete). Leave empty to update the legacy default action.
     * @param actionType (optional) — Type of action: navigate_page, external_link, workflow_event, or clear to remove action
     * @param route — For navigate_page: the route path (e.g., /about, /products/123)
     * @param queryParams — For navigate_page: optional JSON object of query parameters
     * @param url — For external_link: the external URL to open
     * @param nodeId — For workflow_event: the ID of the workflow node to trigger
     * @impure has side effects / drives control flow
     */
    function setElementAction({ elementRef: Struct, eventName?: string, actionType?: string, route: string, queryParams: string, url: string, nodeId: string }): void;

    /**
     * Enables or disables an element
     * @node a2ui_set_element_disabled @alias a2uiSetElementDisabled
     * @param elementRef — Element ID string or element object from Get Element
     * @param disabled — Whether the element should be disabled
     * @impure has side effects / drives control flow
     */
    function setElementDisabled({ elementRef: Struct, disabled: bool }): void;

    /**
     * Sets the loading state of a button element
     * @node a2ui_set_element_loading @alias a2uiSetElementLoading
     * @param elementRef — Element ID string or element object from Get Element
     * @param loading — Whether the element is in loading state
     * @impure has side effects / drives control flow
     */
    function setElementLoading({ elementRef: Struct, loading: bool }): void;

    /**
     * Sets style properties of an element
     * @node a2ui_set_element_style @alias a2uiSetElementStyle
     * @param elementRef — Element ID string or element object from Get Element
     * @param style — Style properties to set (JSON object)
     * @impure has side effects / drives control flow
     */
    function setElementStyle({ elementRef: any, style: Struct }): void;

    /**
     * Sets the text content of an element
     * @node a2ui_set_element_text @alias a2uiSetElementText
     * @param elementRef — Reference to the text element (ID string or element object from Get Element)
     * @param text — The new text content
     * @impure has side effects / drives control flow
     */
    function setElementText({ elementRef: Struct, text: string }): void;

    /**
     * Sets the value of an input element
     * @node a2ui_set_element_value @alias a2uiSetElementValue
     * @param elementRef — Element ID string or element object from Get Element
     * @param value — The new value for the input
     * @impure has side effects / drives control flow
     */
    function setElementValue({ elementRef: Struct, value: string }): void;

    /**
     * Shows or hides an element
     * @node a2ui_set_element_visibility @alias a2uiSetElementVisibility
     * @param elementRef — Element ID string or element object from Get Element
     * @param visible (optional) — Whether the element should be visible
     * @impure has side effects / drives control flow
     */
    function setElementVisibility({ elementRef: any, visible?: bool }): void;

    // === UI/Elements/Button ===

    /**
     * Gets whether a button element is disabled
     * @node a2ui_get_button_disabled @alias a2uiGetButtonDisabled
     * @param elementRef — Reference to the button element
     * @returns disabled — Whether the button is disabled
     */
    function getButtonDisabled({ elementRef: Struct }): bool;

    /**
     * Gets the label text of a button element
     * @node a2ui_get_button_label @alias a2uiGetButtonLabel
     * @param elementRef — Reference to the button element
     * @returns label — The button's label text
     */
    function getButtonLabel({ elementRef: Struct }): string;

    /**
     * Gets whether a button element is in loading state
     * @node a2ui_get_button_loading @alias a2uiGetButtonLoading
     * @param elementRef — Reference to the button element
     * @returns loading — Whether the button is loading
     */
    function getButtonLoading({ elementRef: Struct }): bool;

    /**
     * Sets the label text of a button element
     * @node a2ui_set_button_label @alias a2uiSetButtonLabel
     * @param elementRef — Element ID string or element object from Get Element
     * @param label — The new label text
     * @impure has side effects / drives control flow
     */
    function setButtonLabel({ elementRef: Struct, label: string }): void;

    // === UI/Elements/Calendar ===

    /**
     * Add, remove, or update calendar events and view configuration
     * @node a2ui_update_calendar @alias a2uiUpdateCalendar
     * @param elementRef — Reference to the calendar element
     * @param operation (optional) — What operation to perform
     * @param events — Array of events
     * @impure has side effects / drives control flow
     */
    function updateCalendar({ elementRef: Struct, operation?: string, events: Struct[] }): void;

    // === UI/Elements/Charts ===

    /**
     * Push data to a Nivo or Plotly chart. Select JSON for pre-formatted data or CSV for auto-transformation.
     * @node a2ui_push_csv_to_chart @alias a2uiPushCsvToChart
     * @param elementRef — Reference to the chart element
     * @param library (optional) — Nivo or Plotly
     * @param format (optional) — Data format: JSON (passthrough) or CSV (auto-transform)
     * @param data — Chart data as JSON array/object or JSON string
     * @impure has side effects / drives control flow
     */
    function pushCsvToChart({ elementRef: Struct, library?: string, format?: string, data: Struct }): void;

    /**
     * Sets the layout configuration for a Plotly chart
     * @node a2ui_set_chart_layout @alias a2uiSetChartLayout
     * @param elementRef — Reference to the chart element (ID or element object)
     * @param layout — Chart layout object (Plotly layout format)
     * @impure has side effects / drives control flow
     */
    function setChartLayout({ elementRef: Struct, layout: any }): void;

    /**
     * Configure Nivo chart appearance
     * @node a2ui_set_chart_style @alias a2uiSetChartStyle
     * @param elementRef — Reference to the NivoChart element
     * @param chartType (optional) — Type of chart to style
     * @param barStyle — Bar chart styling options
     * @impure has side effects / drives control flow
     */
    function setChartStyle({ elementRef: Struct, chartType?: string, barStyle: Struct }): void;

    /**
     * Sets configuration options for a Nivo chart
     * @node a2ui_set_nivo_config @alias a2uiSetNivoConfig
     * @param elementRef — Reference to the Nivo chart element
     * @param config — Full Nivo configuration object (merged with defaults)
     * @param chartType (optional) — Chart type (bar, line, pie, radar, etc.)
     * @param colors (optional) — Color scheme name or array of colors
     * @param height (optional) — Chart height (e.g., '400px')
     * @impure has side effects / drives control flow
     */
    function setNivoConfig({ elementRef: Struct, config: any, chartType?: string, colors?: any, height?: string }): void;

    // === UI/Elements/Charts/Agent ===

    /**
     * Uses an LLM to write and run SQL against a DataFusion session, returning chart-ready struct data.
     * @node a2ui_chart_data_agent @alias a2uiChartDataAgent
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
    function chartDataAgent({ model: Struct, session: Struct, table: string, description: string, chartType?: string, element: Struct }): { data: Struct, sql: string, explanation: string };

    // === UI/Elements/Checkbox ===

    /**
     * Set or toggle checkbox/switch checked state
     * @node a2ui_update_toggle @alias a2uiUpdateToggle
     * @param elementRef — Reference to checkbox or switch element
     * @param operation (optional) — What operation to perform
     * @param checked (optional) — New checked state
     * @impure has side effects / drives control flow
     */
    function updateToggle({ elementRef: Struct, operation?: string, checked?: bool }): void;

    // === UI/Elements/Containers ===

    /**
     * Removes all children from a container element
     * @node a2ui_clear_children @alias a2uiClearChildren
     * @param containerRef — Reference to the container element (ID or element object)
     * @impure has side effects / drives control flow
     */
    function clearChildren({ containerRef: any }): void;

    /**
     * Gets a child element at a specific index from a container
     * @node a2ui_get_child_at_index @alias a2uiGetChildAtIndex
     * @param containerRef — Reference to the container element
     * @param index — The index of the child to get (0-based)
     * @returns child — The child element at the specified index
     * @returns childId — The ID of the child element
     * @returns found — Whether a child was found at the index
     */
    function getChildAtIndex({ containerRef: string, index: int }): { child: Struct, childId: string, found: bool };

    /**
     * Appends a child element to a container
     * @node a2ui_push_child @alias a2uiPushChild
     * @param containerRef — Reference to the container element (ID or element object)
     * @param childRef — Reference to the child element to append
     * @impure has side effects / drives control flow
     */
    function pushChild({ containerRef: any, childRef: any }): void;

    /**
     * Inserts a child element at a specific index in a container
     * @node a2ui_push_child_at_index @alias a2uiPushChildAtIndex
     * @param containerRef — Reference to the container element (ID or element object)
     * @param childRef — Reference to the child element to insert
     * @param index — The index at which to insert the child (0-based)
     * @impure has side effects / drives control flow
     */
    function pushChildAtIndex({ containerRef: any, childRef: any, index: int }): void;

    /**
     * Removes a child element at a specific index from a container
     * @node a2ui_remove_child_at_index @alias a2uiRemoveChildAtIndex
     * @param containerRef — Reference to the container element (ID or element object)
     * @param index — The index of the child to remove (0-based)
     * @impure has side effects / drives control flow
     */
    function removeChildAtIndex({ containerRef: any, index: int }): void;

    // === UI/Elements/Display ===

    /**
     * Sets the content/text of a badge element
     * @node a2ui_set_badge_content @alias a2uiSetBadgeContent
     * @param elementRef — Reference to the badge element
     * @param content — The badge content (text or number)
     * @impure has side effects / drives control flow
     */
    function setBadgeContent({ elementRef: Struct, content: string }): void;

    /**
     * Sets the original and modified content of a diff view element
     * @node a2ui_set_diff_content @alias a2uiSetDiffContent
     * @param elementRef — Reference to the diff view element
     * @param original — Left / old content (text or document URL)
     * @param modified — Right / new content (text or document URL)
     * @impure has side effects / drives control flow
     */
    function setDiffContent({ elementRef: Struct, original: string, modified: string }): void;

    /**
     * Sets the icon name of an icon element
     * @node a2ui_set_icon @alias a2uiSetIcon
     * @param elementRef — Reference to the icon element
     * @param name — The icon name (e.g., 'check', 'x', 'star')
     * @impure has side effects / drives control flow
     */
    function setIcon({ elementRef: Struct, name: string }): void;

    /**
     * Sets the markdown content of a markdown element
     * @node a2ui_set_markdown_content @alias a2uiSetMarkdownContent
     * @param elementRef — Reference to the markdown element
     * @param content — The markdown content
     * @impure has side effects / drives control flow
     */
    function setMarkdownContent({ elementRef: Struct, content: string }): void;

    /**
     * Sets the value of a progress bar (0-100)
     * @node a2ui_set_progress @alias a2uiSetProgress
     * @param elementRef — Reference to the progress bar element
     * @param value — Progress value (0-100)
     * @impure has side effects / drives control flow
     */
    function setProgress({ elementRef: Struct, value: float }): void;

    // === UI/Elements/Files ===

    /**
     * Gets uploaded files, signed URLs, and FlowPaths from an A2UI fileInput or voiceInput element
     * @node a2ui_get_file_input_files @alias a2uiGetFileInputFiles
     * @param elementRef — File or voice input element ID or element object from Get Element
     * @returns files — Uploaded file objects
     * @returns signedUrls — Signed or local URLs for the uploaded files
     * @returns flowPaths — Temporary FlowPaths for uploaded files when available
     * @returns exists — Whether the file input element exists
     */
    function getFileInputFiles({ elementRef: Struct }): { files: Struct[], signedUrls: string[], flowPaths: Struct[], exists: bool };

    // === UI/Elements/Game ===

    /**
     * Update any property of a 3D model
     * @node a2ui_update_model3d @alias a2uiUpdateModel3d
     * @param elementRef — Reference to the 3D model element
     * @param property (optional) — Which property to update
     * @param src — GLTF/GLB model URL
     * @impure has side effects / drives control flow
     */
    function updateModel3d({ elementRef: Struct, property?: string, src: string }): void;

    /**
     * Update any property of a 3D scene
     * @node a2ui_update_scene3d @alias a2uiUpdateScene3d
     * @param elementRef — Reference to the 3D scene element
     * @param property (optional) — Which property to update
     * @param camera — Camera type, position, and target
     * @impure has side effects / drives control flow
     */
    function updateScene3d({ elementRef: Struct, property?: string, camera: Struct }): void;

    /**
     * Update any property of a sprite
     * @node a2ui_update_sprite @alias a2uiUpdateSprite
     * @param elementRef — Reference to the sprite element
     * @param property (optional) — Which property to update
     * @param src — Image URL
     * @impure has side effects / drives control flow
     */
    function updateSprite({ elementRef: Struct, property?: string, src: string }): void;

    // === UI/Elements/Gantt ===

    /**
     * Add, remove, or update gantt tasks, dependencies and configuration
     * @node a2ui_update_gantt @alias a2uiUpdateGantt
     * @param elementRef — Reference to the gantt element
     * @param operation (optional) — What operation to perform
     * @param tasks — Array of tasks
     * @impure has side effects / drives control flow
     */
    function updateGantt({ elementRef: Struct, operation?: string, tasks: Struct[] }): void;

    // === UI/Elements/GeoMap ===

    /**
     * Update markers, routes, or viewport of a map
     * @node a2ui_update_geomap @alias a2uiUpdateGeomap
     * @param elementRef — Reference to the map element
     * @param property (optional) — Which property to update
     * @param markers — Array of map markers
     * @impure has side effects / drives control flow
     */
    function updateGeomap({ elementRef: Struct, property?: string, markers: Struct[] }): void;

    // === UI/Elements/Get ===

    /**
     * Gets the src URL of an iframe element
     * @node a2ui_get_iframe_src @alias a2uiGetIframeSrc
     * @param elementRef — Reference to the iframe element
     * @returns src — The iframe's source URL
     */
    function getIframeSrc({ elementRef: Struct }): string;

    /**
     * Gets the content text of a tooltip element
     * @node a2ui_get_tooltip_content @alias a2uiGetTooltipContent
     * @param elementRef — Reference to the tooltip element
     * @returns content — The tooltip's content text
     * @returns side — The tooltip's side position (top, bottom, left, right)
     */
    function getTooltipContent({ elementRef: Struct }): { content: string, side: string };

    // === UI/Elements/Graph ===

    /**
     * Update the nodes, edges or label styles of a graph
     * @node a2ui_update_graph @alias a2uiUpdateGraph
     * @param elementRef — Reference to the graph element
     * @param property (optional) — Which property to update
     * @param nodes — Array of graph nodes
     * @impure has side effects / drives control flow
     */
    function updateGraph({ elementRef: Struct, property?: string, nodes: Struct[] }): void;

    // === UI/Elements/Hotspot ===

    /**
     * Add, remove, or manage hotspots on an ImageHotspot element
     * @node a2ui_update_hotspot @alias a2uiUpdateHotspot
     * @param elementRef — Reference to the ImageHotspot element
     * @param operation (optional) — What operation to perform
     * @param hotspot — Hotspot to add
     * @impure has side effects / drives control flow
     */
    function updateHotspot({ elementRef: Struct, operation?: string, hotspot: Struct }): void;

    // === UI/Elements/Input ===

    /**
     * Clears the value of an input element
     * @node a2ui_clear_input @alias a2uiClearInput
     * @param elementRef — Element ID string or element object from Get Element
     * @impure has side effects / drives control flow
     */
    function clearInput({ elementRef: any }): void;

    /**
     * Gets the placeholder text of an input element
     * @node a2ui_get_input_placeholder @alias a2uiGetInputPlaceholder
     * @param elementRef — Reference to the input element
     * @returns placeholder — The input's placeholder text
     */
    function getInputPlaceholder({ elementRef: Struct }): string;

    /**
     * Sets the placeholder text of an input element
     * @node a2ui_set_input_placeholder @alias a2uiSetInputPlaceholder
     * @param elementRef — Element ID string or element object from Get Element
     * @param placeholder — The new placeholder text
     * @impure has side effects / drives control flow
     */
    function setInputPlaceholder({ elementRef: Struct, placeholder: string }): void;

    /**
     * Sets the error state or message of a text field
     * @node a2ui_set_textfield_error @alias a2uiSetTextfieldError
     * @param elementRef — Reference to the text field element
     * @param error — Error message (empty string clears error)
     * @impure has side effects / drives control flow
     */
    function setTextfieldError({ elementRef: Struct, error: string }): void;

    // === UI/Elements/Labeler ===

    /**
     * Add, remove, or manage bounding boxes on an ImageLabeler element
     * @node a2ui_update_labeler @alias a2uiUpdateLabeler
     * @param elementRef — Reference to the ImageLabeler element
     * @param operation (optional) — What operation to perform
     * @param box — Bounding box to add
     * @impure has side effects / drives control flow
     */
    function updateLabeler({ elementRef: Struct, operation?: string, box: Struct }): void;

    // === UI/Elements/Media ===

    /**
     * Sets the source URL of an iframe element
     * @node a2ui_set_iframe_src @alias a2uiSetIframeSrc
     * @param elementRef — Reference to the iframe element
     * @param src — The URL to load in the iframe
     * @impure has side effects / drives control flow
     */
    function setIframeSrc({ elementRef: Struct, src: string }): void;

    /**
     * Sets raw HTML content of an iframe element for previewing generated HTML
     * @node a2ui_set_iframe_srcdoc @alias a2uiSetIframeSrcdoc
     * @param elementRef — Reference to the iframe element
     * @param html — Raw HTML content to render inside the iframe
     * @impure has side effects / drives control flow
     */
    function setIframeSrcdoc({ elementRef: Struct, html: string }): void;

    /**
     * Signs a FlowPath and sets it as the source for image, video, avatar, iframe, lottie, or file preview elements
     * @node a2ui_set_media_source @alias a2uiSetMediaSource
     * @param elementRef — Reference to the media element
     * @param file — FlowPath to sign and use as the element source
     * @param expiration (optional) — Expiration time for the signed URL
     * @returns signedUrl — The generated signed URL
     * @returns mimeType — Detected MIME type from the FlowPath extension
     * @returns mediaKind — Detected media kind: image, video, audio, pdf, text, or file
     * @impure has side effects / drives control flow
     */
    function setMediaSource({ elementRef: Struct, file: Struct, expiration?: int }): { signedUrl: string, mimeType: string, mediaKind: string };

    // === UI/Elements/Overlay ===

    /**
     * Set, push, or clear bounding boxes on a BoundingBoxOverlay element
     * @node a2ui_update_overlay @alias a2uiUpdateOverlay
     * @param elementRef — Reference to the BoundingBoxOverlay element
     * @param operation (optional) — What operation to perform
     * @param boxes — Array of detection bounding boxes
     * @impure has side effects / drives control flow
     */
    function updateOverlay({ elementRef: Struct, operation?: string, boxes: Struct[] }): void;

    // === UI/Elements/Query ===

    /**
     * Gets all child elements of a container
     * @node a2ui_query_children @alias a2uiQueryChildren
     * @param elementRef — Reference to the container element
     * @returns children — Array of child elements
     * @returns childIds — Array of child element IDs
     * @returns count — Number of children
     */
    function queryChildren({ elementRef: string }): { children: Struct, childIds: string[], count: int };

    /**
     * Gets elements whose IDs match a pattern
     * @node a2ui_query_elements_by_id @alias a2uiQueryElementsById
     * @param pattern — The pattern to match element IDs against
     * @param matchType — How to match: 'starts_with', 'ends_with', 'contains', or 'exact'
     * @returns elements — Array of matching elements
     * @returns elementIds — Array of matching element IDs
     * @returns count — Number of matching elements
     */
    function queryElementsById({ pattern: string, matchType: string }): { elements: Struct, elementIds: string[], count: int };

    /**
     * Gets all elements of a specific component type
     * @node a2ui_query_elements_by_type @alias a2uiQueryElementsByType
     * @param componentType — The type of component to query (e.g., 'button', 'text', 'textField')
     * @returns elements — Array of matching elements
     * @returns count — Number of matching elements
     */
    function queryElementsByType({ componentType: string }): { elements: Struct, count: int };

    /**
     * Gets the parent element of an element
     * @node a2ui_query_parent @alias a2uiQueryParent
     * @param elementRef — Reference to the element to find parent of
     * @returns parent — The parent element data
     * @returns parentId — ID of the parent element
     * @returns hasParent — Whether a parent was found
     */
    function queryParent({ elementRef: string }): { parent: Struct, parentId: string, hasParent: bool };

    // === UI/Elements/Select ===

    /**
     * Gets the selected value of a select element
     * @node a2ui_get_select_value @alias a2uiGetSelectValue
     * @param elementRef — Reference to the select element
     * @returns value — The currently selected value
     * @returns hasSelection — Whether a value is selected
     */
    function getSelectValue({ elementRef: Struct }): { value: string, hasSelection: bool };

    /**
     * Sets the available options in a select element
     * @node a2ui_set_select_options @alias a2uiSetSelectOptions
     * @param elementRef — Reference to the select element
     * @param options — Array of options [{value, label}] or simple strings
     * @impure has side effects / drives control flow
     */
    function setSelectOptions({ elementRef: Struct, options: any }): void;

    /**
     * Sets the selected value of a select element
     * @node a2ui_set_select_value @alias a2uiSetSelectValue
     * @param elementRef — Element ID string or element object from Get Element
     * @param value — The value to select
     * @impure has side effects / drives control flow
     */
    function setSelectValue({ elementRef: Struct, value: string }): void;

    // === UI/Elements/Set ===

    /**
     * Sets the content text of a tooltip element
     * @node a2ui_set_tooltip_content @alias a2uiSetTooltipContent
     * @param elementRef — Reference to the tooltip element
     * @param content — The content text to set
     * @impure has side effects / drives control flow
     */
    function setTooltipContent({ elementRef: Struct, content: string }): void;

    // === UI/Elements/Slider ===

    /**
     * Sets the value of a slider element
     * @node a2ui_set_slider_value @alias a2uiSetSliderValue
     * @param elementRef — Reference to the slider element
     * @param value — The new slider value
     * @impure has side effects / drives control flow
     */
    function setSliderValue({ elementRef: Struct, value: float }): void;

    // === UI/Elements/Table ===

    /**
     * Add, remove, or update table data and structure
     * @node a2ui_update_table @alias a2uiUpdateTable
     * @param elementRef — Reference to the table element
     * @param operation (optional) — What operation to perform
     * @param data — Array of row objects
     * @impure has side effects / drives control flow
     */
    function updateTable({ elementRef: Struct, operation?: string, data: Struct }): void;

    /**
     * Push CSV or Table data directly to a table element
     * @node a2ui_write_csv_to_table @alias a2uiWriteCsvToTable
     * @param elementRef — Reference to the table element
     * @param csv — CSV text with headers
     * @param table — Table data from DataFusion query
     * @param delimiter (optional) — CSV delimiter (default: comma)
     * @impure has side effects / drives control flow
     */
    function writeCsvToTable({ elementRef: Struct, csv: string, table: Struct, delimiter?: string }): void;

    // === UI/Navigation ===

    /**
     * Closes an open dialog. If no dialog ID is specified, closes the topmost dialog.
     * @node a2ui_close_dialog @alias a2uiCloseDialog
     * @param dialogId — Optional ID of the specific dialog to close. If empty, closes the topmost dialog.
     * @impure has side effects / drives control flow
     */
    function closeDialog({ dialogId: string }): void;

    /**
     * Gets the current page route from the execution context
     * @node a2ui_get_current_route @alias a2uiGetCurrentRoute
     * @returns route — The current route path
     * @impure has side effects / drives control flow
     */
    function getCurrentRoute(): string;

    /**
     * Gets query parameters from the current URL
     * @node a2ui_get_query_params @alias a2uiGetQueryParams
     * @param paramName — The name of the query parameter to get (optional - if empty, returns all params)
     * @returns value — The parameter value (string if param_name specified, object if all params)
     * @returns exists — Whether the parameter exists
     * @impure has side effects / drives control flow
     */
    function getQueryParam({ paramName: string }): { value: any, exists: bool };

    /**
     * Gets route parameters from the current URL
     * @node a2ui_get_route_params @alias a2uiGetRouteParams
     * @param paramName — The name of the route parameter to get (optional - if empty, returns all params)
     * @returns value — The parameter value (string if param_name specified, object if all params)
     * @returns exists — Whether the parameter exists
     * @impure has side effects / drives control flow
     */
    function getRouteParam({ paramName: string }): { value: any, exists: bool };

    /**
     * Navigates to a page route
     * @node a2ui_navigate_to @alias a2uiNavigateTo
     * @param route — The route to navigate to (e.g., /dashboard, /users/123)
     * @param queryParams (optional) — Optional query parameters as key-value pairs (e.g., {"tab": "settings", "id": "123"})
     * @param replace (optional) — If true, replaces the current history entry instead of adding a new one
     * @impure has side effects / drives control flow
     */
    function navigateTo({ route: string, queryParams?: Struct, replace?: bool }): void;

    /**
     * Opens a route/page as a modal dialog overlay
     * @node a2ui_open_dialog @alias a2uiOpenDialog
     * @param route — The route path to open in the dialog (e.g., /settings, /edit/123)
     * @param title — Optional dialog title (shown in header)
     * @param queryParams — Optional JSON object of query parameters to pass to the route
     * @param dialogId — Optional unique ID for the dialog (for closing specific dialogs)
     * @impure has side effects / drives control flow
     */
    function openDialog({ route: string, title: string, queryParams: string, dialogId: string }): void;

    /**
     * Sets or updates a query parameter in the URL
     * @node a2ui_set_query_param @alias a2uiSetQueryParam
     * @param key — The query parameter key to set
     * @param value — The value to set (empty string removes the param)
     * @param replace — If true, replaces the current history entry instead of adding a new one
     * @impure has side effects / drives control flow
     */
    function setQueryParam({ key: string, value: string, replace: bool }): void;

    /**
     * Decodes a URL-encoded (percent-encoded) string
     * @node a2ui_url_decode @alias a2uiUrlDecode
     * @param input — The URL-encoded string to decode
     * @returns decoded — The decoded string
     * @returns success — Whether the decoding was successful
     */
    function urlDecode({ input: string }): { decoded: string, success: bool };

    /**
     * Encodes a string for safe use in URLs (percent-encoding)
     * @node a2ui_url_encode @alias a2uiUrlEncode
     * @param input — The string to URL-encode
     * @returns encoded — The URL-encoded string
     */
    function urlEncode({ input: string }): string;

    // === UI/State ===

    /**
     * Gets a value from global state by key
     * @node a2ui_get_global_state @alias a2uiGetGlobalState
     * @param key — The key to retrieve from global state
     * @returns value — The value stored at the key
     * @returns exists — Whether the key exists in global state
     */
    function getGlobalState({ key: string }): { value: any, exists: bool };

    /**
     * Gets a value from page-local state by key
     * @node a2ui_get_page_state @alias a2uiGetPageState
     * @param key — The key to retrieve from page state
     * @returns value — The value stored at the key
     * @returns exists — Whether the key exists in page state
     */
    function getPageState({ key: string }): { value: any, exists: bool };

    /**
     * Sets a value in global state by key
     * @node a2ui_set_global_state @alias a2uiSetGlobalState
     * @param key — The key to store the value at
     * @param value — The value to store
     * @impure has side effects / drives control flow
     */
    function setGlobalState({ key: string, value: any }): void;

    /**
     * Sets a value in page-local state by key
     * @node a2ui_set_page_state @alias a2uiSetPageState
     * @param key — The key to store the value at
     * @param value — The value to store
     * @impure has side effects / drives control flow
     */
    function setPageState({ key: string, value: any }): void;

    // === UI/Surface ===

    /**
     * Sends a surface to the frontend to begin rendering
     * @node a2ui_begin_rendering @alias a2uiBeginRendering
     * @param surface — The surface to render
     * @param components — Array of components to include
     * @param dataModel — Initial data model for bindings
     * @impure has side effects / drives control flow
     */
    function beginRendering({ surface: Struct, components: Struct[], dataModel: Struct }): void;

    /**
     * Creates a new A2UI surface with an ID and root component
     * @node a2ui_create_surface @alias a2uiCreateSurface
     * @param surfaceId (optional) — Unique identifier for the surface
     * @param rootComponentId (optional) — ID of the root component in the surface
     * @param catalogId — Optional custom component catalog
     * @returns surface — The created surface for adding components
     * @impure has side effects / drives control flow
     */
    function createSurface({ surfaceId?: string, rootComponentId?: string, catalogId: string }): Struct;

    /**
     * Removes a surface from the frontend
     * @node a2ui_delete_surface @alias a2uiDeleteSurface
     * @param surfaceId (optional) — ID of the surface to delete
     * @impure has side effects / drives control flow
     */
    function deleteSurface({ surfaceId?: string }): void;

    /**
     * Sets or clears scoped custom CSS for a custom UI surface at runtime
     * @node a2ui_set_surface_custom_css @alias a2uiSetSurfaceCustomCss
     * @param surfaceId (optional) — ID of the custom UI surface to update
     * @param customCss (optional) — CSS to apply to the surface. Leave empty to clear it.
     * @impure has side effects / drives control flow
     */
    function setSurfaceCustomCss({ surfaceId?: string, customCss?: string }): void;

    /**
     * Shows the current frontend screen while the workflow continues running
     * @node a2ui_show_screen @alias a2uiShowScreen
     * @impure has side effects / drives control flow
     */
    function showScreen(): void;

    /**
     * Updates components in an existing surface
     * @node a2ui_surface_update @alias a2uiSurfaceUpdate
     * @param surfaceId (optional) — ID of the surface to update
     * @param components — Components to add or update
     * @impure has side effects / drives control flow
     */
    function surfaceUpdate({ surfaceId?: string, components: Struct[] }): void;
}
