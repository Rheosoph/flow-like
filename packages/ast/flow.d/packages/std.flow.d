// std — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Control ===

/**
 * Branches the flow based on a condition
 * @param condition (optional) — The condition to evaluate
 * @impure has side effects / drives control flow
 */
declare function controlBranch({ condition?: bool }): void;

/**
 * Loops over an Array
 * @param array — Array to Loop
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlForEach({ array: any[] }): { value: any, index: int };

/**
 * Loops over an Array; allows breaking early from inside the loop body.
 * @param break (optional) — Trigger this to terminate the active loop early (callable from inside Loop Body)
 * @param array — Array to Loop
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlForEachWithBreak({ break?: bool, array: any[] }): { value: any, index: int };

/**
 * Parallel Execution
 * @param threadModel (optional) — Threads
 * @impure has side effects / drives control flow
 */
declare function controlParExecution({ threadModel?: string }): void;

/**
 * Loops over an Array in Parallel
 * @param array — Array to Loop
 * @param maxConcurrent (optional) — Maximum number of concurrent executions (0 = unlimited)
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlParForEach({ array: any[], maxConcurrent?: int }): { value: any, index: int };

/**
 * Sequential Execution
 * @impure has side effects / drives control flow
 */
declare function controlSequence(): void;

/**
 * Executes with a timeout, branching based on completion
 * @param timeoutMs (optional) — Timeout duration in milliseconds
 * @impure has side effects / drives control flow
 */
declare function controlTimeout({ timeoutMs?: float }): void;

/**
 * Loop downstream execution in while loop
 * @param condition (optional) — Loop while this is true
 * @param maxIter (optional) — Maximum number of iterations
 * @returns iter — Current iteration index
 * @impure has side effects / drives control flow
 */
declare function controlWhileLoop({ condition?: bool, maxIter?: int }): int;

/**
 * Delays execution for a specified amount of time
 * @param time (optional) — Delay time in milliseconds
 * @impure has side effects / drives control flow
 */
declare function delay({ time?: float }): void;

/**
 * Control Flow Node
 * @param routeIn
 * @returns routeOut
 */
declare function reroute({ routeIn: any }): any;


// === Control/Call ===

/**
 * References a specific call in the flow
 * @param fnRef — The function reference to call
 * @impure has side effects / drives control flow
 */
declare function controlCallReference({ fnRef: string }): void;


// === Control/Flow ===

/**
 * Pass execution the first N triggers, then block; fire 'Completed' on Nth.
 * @param n (optional) — Number of times to allow execution to pass (>= 0)
 * @param startIndex (optional) — Initial index before first pass (commonly 0)
 * @returns index — Current counter after this trigger
 * @returns remaining — How many passes are left until Completed fires
 * @impure has side effects / drives control flow
 */
declare function controlDoN({ n?: int, startIndex?: int }): { index: int, remaining: int };

/**
 * Let execution pass once, then block until Reset.
 * @param startClosed (optional) — If true, starts blocked until a Reset arrives
 * @returns hasFired — Whether this node has already allowed a pass (blocked if true)
 * @impure has side effects / drives control flow
 */
declare function controlDoOnce({ startClosed?: bool }): bool;

/**
 * Alternate execution between A and B on successive triggers.
 * @param startOnA (optional) — If true, first pass goes to A; otherwise to B
 * @returns isA — Side that will fire on next trigger
 * @returns tick — How many times FlipFlop has executed
 * @impure has side effects / drives control flow
 */
declare function controlFlipFlop({ startOnA?: bool }): { isA: bool, tick: int };

/**
 * Open/close a gate to conditionally pass execution.
 * @param startClosed (optional) — If true, the gate starts closed (blocked)
 * @returns isOpen — Current open/closed state after this tick
 * @impure has side effects / drives control flow
 */
declare function controlGate({ startClosed?: bool }): bool;


// === Control/Functions ===

/**
 * Calls a function defined on this board
 * @param functionLayerId — The function to call
 */
declare function controlCallFunction({ functionLayerId: string }): void;


// === Control/Parallel ===

/**
 * Gather all execution states
 * @impure has side effects / drives control flow
 */
declare function controlGather(): void;


// === Logging ===

/**
 * Logs / Prints an Error
 * @param message (optional) — Print Error Message
 * @param toast (optional) — Should the user see a toast popping up?
 * @impure has side effects / drives control flow
 */
declare function logError({ message?: any, toast?: bool }): void;

/**
 * Print Debugging Information
 * @param message (optional) — The message to log
 * @param toast (optional) — Should the user see a toast popping up?
 * @impure has side effects / drives control flow
 */
declare function logInfo({ message?: any, toast?: bool }): void;

/**
 * Shows a progress toast to the user that can be updated
 * @param id (optional) — Unique identifier for this progress. Use the same ID to update the progress.
 * @param message (optional) — The message shown to the user
 * @param progress (optional) — Progress value between 0 and 100. Leave empty to show indeterminate progress.
 * @impure has side effects / drives control flow
 */
declare function logProgress({ id?: string, message?: string, progress?: int }): void;

/**
 * Completes a progress toast with a success or error state
 * @param id (optional) — The ID of the progress toast to complete (must match the ID used in Show Progress)
 * @param message (optional) — Final message to show (e.g., 'Completed!' or 'Failed')
 * @param success (optional) — Whether the operation was successful (true shows success toast, false shows error)
 * @impure has side effects / drives control flow
 */
declare function logProgressDone({ id?: string, message?: string, success?: bool }): void;

/**
 * Logs a Warning
 * @param message (optional) — Print Warning
 * @param toast (optional) — Should the user see a toast popping up?
 * @impure has side effects / drives control flow
 */
declare function logWarning({ message?: any, toast?: bool }): void;


// === Math ===

/**
 * Evaluates a mathematical expression
 * @param expression — Mathematical expression
 * @returns result — Result of the expression
 */
declare function eval({ expression: string }): float;


// === Math/Float ===

/**
 * Calculates the absolute value of a float
 * @param float — Input Float
 * @returns absolute — The absolute value of the float
 */
declare function floatAbs({ float: float }): float;

/**
 * Adds two floats together
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns sum — The sum of the two floats
 */
declare function floatAdd({ float1: float, float2: float }): float;

/**
 * Rounds a float up to the nearest integer
 * @param float — Input Float
 * @returns ceiling — The ceiling of the float
 */
declare function floatCeil({ float: float }): int;

/**
 * Clamps a float within a given range
 * @param float — Input Float
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns clamped — The clamped float
 */
declare function floatClamp({ float: float, min: float, max: float }): float;

/**
 * Divides one float by another
 * @param dividend — The number to be divided
 * @param divisor — The number to divide by
 * @returns quotient — The result of the division
 */
declare function floatDivide({ dividend: float, divisor: float }): float;

/**
 * Rounds a float down to the nearest integer
 * @param float — Input Float
 * @returns floor — The floor of the float
 */
declare function floatFloor({ float: float }): int;

/**
 * Returns the larger of two floats
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns maximum — The larger of the two floats
 */
declare function floatMax({ float1: float, float2: float }): float;

/**
 * Returns the smaller of two floats
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns minimum — The smaller of the two floats
 */
declare function floatMin({ float1: float, float2: float }): float;

/**
 * Multiplies two floats together
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns product — The product of the two floats
 */
declare function floatMultiply({ float1: float, float2: float }): float;

/**
 * Calculates the power of a float
 * @param base — Base float
 * @param exponent — Exponent float
 * @returns power — Result of the power calculation
 */
declare function floatPower({ base: float, exponent: float }): float;

/**
 * Calculates the nth root of a float
 * @param radicand — The float to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function floatRoot({ radicand: float, degree: int }): float;

/**
 * Rounds a float to the nearest integer
 * @param float — Input Float
 * @returns rounded — The rounded float
 */
declare function floatRound({ float: float }): float;

/**
 * Subtracts one float from another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns difference — The difference between the two floats
 */
declare function floatSubtract({ float1: float, float2: float }): float;


// === Math/Float/Comparison ===

/**
 * Checks if two floats are equal (within a tolerance)
 * @param float1 — First Float
 * @param float2 — Second Float
 * @param tolerance — Comparison Tolerance
 * @returns isEqual — True if the floats are equal, false otherwise
 */
declare function floatEqual({ float1: float, float2: float, tolerance: float }): bool;

/**
 * Checks if one float is greater than another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isGreater — True if float1 is greater than float2, false otherwise
 */
declare function floatGreaterThan({ float1: float, float2: float }): bool;

/**
 * Checks if one float is greater than or equal to another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isGreaterOrEqual — True if float1 is greater than or equal to float2, false otherwise
 */
declare function floatGreaterThanOrEqual({ float1: float, float2: float }): bool;

/**
 * Checks if one float is less than another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isLess — True if float1 is less than float2, false otherwise
 */
declare function floatLessThan({ float1: float, float2: float }): bool;

/**
 * Checks if one float is less than or equal to another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isLessOrEqual — True if float1 is less than or equal to float2, false otherwise
 */
declare function floatLessThanOrEqual({ float1: float, float2: float }): bool;

/**
 * Checks if two floats are unequal (within a tolerance)
 * @param float1 — First Float
 * @param float2 — Second Float
 * @param tolerance — Comparison Tolerance
 * @returns isUnequal — True if the floats are unequal, false otherwise
 */
declare function floatUnequal({ float1: float, float2: float, tolerance: float }): bool;


// === Math/Float/Random ===

/**
 * Generates a random float within a specified range
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns randomFloat — The generated random float
 */
declare function floatRandomInRange({ min: float, max: float }): float;


// === Math/Int ===

/**
 * Returns the absolute value of an Integer
 * @param integer — Input Integer
 * @returns absolute — Absolute Value
 */
declare function intAbs({ integer: int }): int;

/**
 * Adds two Integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns sum — Sum of the two integers
 */
declare function intAdd({ integer1: int, integer2: int }): int;

/**
 * Clamps an integer within a range
 * @param integer — Input Integer
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns clamped — Clamped Value
 */
declare function intClamp({ integer: int, min: int, max: int }): int;

/**
 * Divides two Integers (handles division by zero)
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns result — Result of the division
 */
declare function intDivide({ integer1: int, integer2: int }): float;

/**
 * Checks if two integers are equal
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns equal — True if the integers are equal, false otherwise
 */
declare function intEqual({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is greater than the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns greaterThan — True if integer1 > integer2, false otherwise
 */
declare function intGreaterThan({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is greater than or equal to the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns greaterThanOrEqual — True if integer1 >= integer2, false otherwise
 */
declare function intGreaterThanOrEqual({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is less than the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns lessThan — True if integer1 < integer2, false otherwise
 */
declare function intLessThan({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is less than or equal to the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns lessThanOrEqual — True if integer1 <= integer2, false otherwise
 */
declare function intLessThanOrEqual({ integer1: int, integer2: int }): bool;

/**
 * Returns the larger of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns maximum — The larger of the two integers
 */
declare function intMax({ integer1: int, integer2: int }): int;

/**
 * Returns the smaller of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns minimum — The smaller of the two integers
 */
declare function intMin({ integer1: int, integer2: int }): int;

/**
 * Calculates the remainder of integer division
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns remainder — Remainder of the division
 */
declare function intModulo({ integer1: int, integer2: int }): int;

/**
 * Multiplies two Integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns product — Product of the two integers
 */
declare function intMultiply({ integer1: int, integer2: int }): int;

/**
 * Calculates the power of an integer
 * @param base — Base integer
 * @param exponent — Exponent integer
 * @returns power — Result of the power calculation
 */
declare function intPower({ base: int, exponent: int }): int;

/**
 * Calculates the nth root of an integer
 * @param radicand — The integer to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function intRoot({ radicand: int, degree: int }): float;

/**
 * Subtracts two Integers
 * @param integer1 — Minuend
 * @param integer2 — Subtrahend
 * @returns difference — Difference of the two integers
 */
declare function intSubtract({ integer1: int, integer2: int }): int;

/**
 * Checks if two integers are unequal
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns unequal — True if the integers are unequal, false otherwise
 */
declare function intUnequal({ integer1: int, integer2: int }): bool;


// === Math/Int/Random ===

/**
 * Generates a random integer within a specified range
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns randomInteger — The generated random integer
 */
declare function intRandomInRange({ min: int, max: int }): int;


// === Notifications ===

/**
 * Send a notification to a specific user in this project
 * @param flowUserSub (optional) — Project user to notify
 * @param title (optional) — Notification title
 * @param description (optional) — Notification description (optional)
 * @param icon — FlowPath to a notification icon image (optional)
 * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
 * @returns success — Whether the notification was sent successfully
 * @impure has side effects / drives control flow
 */
declare function notifyProjectUser({ flowUserSub?: string, title?: string, description?: string, icon: Struct, link?: string }): bool;

/**
 * Send a notification to the user who executed this workflow
 * @param title (optional) — Notification title
 * @param description (optional) — Notification description (optional)
 * @param icon — FlowPath to a notification icon image (optional)
 * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
 * @param showDesktop (optional) — Show desktop notification if available
 * @returns success — Whether the notification was sent successfully
 * @impure has side effects / drives control flow
 */
declare function notifyUser({ title?: string, description?: string, icon: Struct, link?: string, showDesktop?: bool }): bool;


// === Structs ===

/**
 * Breaks a struct into its individual fields based on the schema
 * @param structIn — The struct to break apart
 */
declare function structBreak({ structIn: Struct }): void;

/**
 * Creates a new struct
 * @returns struct — Struct Output
 */
declare function structMake(): Struct;

/**
 * Creates a struct from individual fields based on a connected schema
 * @returns structOut — The constructed struct
 */
declare function structMakeFromSchema(): Struct;


// === Structs/Fields ===

/**
 * Fetches a field from a struct (supports dot notation and array access)
 * @param struct — Struct Output
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @returns value — Value of the Struct
 * @returns found — Indicates if the value was found
 */
declare function structGet({ struct: Struct, field: string }): { value: any, found: bool };

/**
 * Fetches fields from a struct
 * @param struct — Struct Output
 * @returns fieldNames — Fields
 * @returns fields — Fields
 */
declare function structGetFields({ struct: Struct }): { fieldNames: string[], fields: any[] };

/**
 * Checks if a field exists in a struct (supports dot notation and array access)
 * @param struct — Struct Output
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @returns found — Indicates if the value was found
 */
declare function structHas({ struct: Struct, field: string }): bool;

/**
 * Removes a field from a struct (supports dot notation and array access)
 * @param structIn — Struct In
 * @param field — Field path to remove (e.g., 'message.content' or 'items[0]')
 * @returns structOut — Struct Out
 * @returns removedValue — The value that was removed (null if field didn't exist)
 * @impure has side effects / drives control flow
 */
declare function structRemove({ structIn: Struct, field: string }): { structOut: Struct, removedValue: any };

/**
 * Sets a field in a struct (supports dot notation and array access)
 * @param structIn — Struct In
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @param value — Value to set
 * @returns structOut — Struct Out
 * @impure has side effects / drives control flow
 */
declare function structSet({ structIn: Struct, field: string, value: any }): Struct;


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

/**
 * Resolves an element inside a widget instance (from Instantiate Widget). The output plugs into any element node (Set Element Value, Update GeoMap, Push CSV To Chart, …).
 * @param elementRef — Widget instance reference (from Instantiate Widget)
 * @param elementId — ID of the element inside the widget (e.g. 'chart-1')
 * @returns element — The element reference (connect to element nodes)
 * @returns exists — Whether the element exists in the widget
 */
declare function a2uiWidgetGetElement({ elementRef: Struct, elementId: string }): { element: Struct, exists: bool };

/**
 * Sets the text of an element inside a widget instance (from Instantiate Widget) before it is pushed to the frontend
 * @param elementRef — Widget instance reference (from Instantiate Widget)
 * @param elementId — ID of the element inside the widget (e.g. 'title-text')
 * @param text (optional) — The text to set
 * @returns elementRefOut — The updated widget instance reference (connect to Push Widget / Push To Container)
 * @impure has side effects / drives control flow
 */
declare function a2uiWidgetSetText({ elementRef: Struct, elementId: string, text?: string }): Struct;


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


// === Utils ===

/**
 * Generates a Collision Resistant Unique Identifier
 * @returns cuid — Generated CUID
 * @impure has side effects / drives control flow
 */
declare function cuid(): string;


// === Utils/Array ===

/**
 * Removes all elements from an array
 * @param arrayIn — Your Array
 * @returns arrayOut — Empty Array
 * @impure has side effects / drives control flow
 */
declare function arrayClear({ arrayIn: any[] }): any[];

/**
 * Append an Array to another Array
 * @param arrayIn — Your Array
 * @param values — Value to push
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayExtend({ arrayIn: any[], values: any[] }): any[];

/**
 * Removes a specific field from every struct in an array. Elements without the field are kept unchanged. Returns the filtered array and count of removed fields.
 * @param arrayIn — Array of structs to filter
 * @param field — Field name to remove from each struct
 * @returns arrayOut — Array with the field removed from each struct
 * @returns removedCount — Number of fields that were removed
 * @impure has side effects / drives control flow
 */
declare function arrayFilterField({ arrayIn: Struct[], field: string }): { arrayOut: Struct[], removedCount: int };

/**
 * Removes multiple fields from every struct in an array. Elements without the fields are kept unchanged. Returns the filtered array and count of removed fields.
 * @param arrayIn — Array of structs to filter
 * @param fields — Array of field names to remove from each struct
 * @returns arrayOut — Array with the fields removed from each struct
 * @returns removedCount — Total number of fields that were removed
 * @impure has side effects / drives control flow
 */
declare function arrayFilterFields({ arrayIn: Struct[], fields: string[] }): { arrayOut: Struct[], removedCount: int };

/**
 * Finds the index of an item in an array
 * @param arrayIn — Your Array
 * @param item — Item to find
 * @returns index — Index of the item (-1 if not found)
 * @returns found — Was the item found?
 * @impure has side effects / drives control flow
 */
declare function arrayFindItem({ arrayIn: any[], item: any }): { index: int, found: bool };

/**
 * Gets an element from an array by index
 * @param arrayIn — Your Array
 * @param index — Index of the element to get
 * @returns element — Element at the specified index
 * @returns success — Was the get successful?
 */
declare function arrayGet({ arrayIn: any[], index: int }): { element: any, success: bool };

/**
 * Checks if an array includes a certain value
 * @param arrayIn — Your Array
 * @param value — Value to search for
 * @returns includes — Does the array include the value?
 */
declare function arrayIncludes({ arrayIn: any[], value: any }): bool;

/**
 * Gets the length of an array
 * @param array — Input Array
 * @returns length — Length of the array
 */
declare function arrayLength({ array: any[] }): int;

/**
 * Removes and returns the last element of an array
 * @param arrayIn — Your Array
 * @returns arrayOut — Adjusted Array
 * @returns value — Popped Value
 * @impure has side effects / drives control flow
 */
declare function arrayPop({ arrayIn: any[] }): { arrayOut: any[], value: any };

/**
 * Push an item into your Array
 * @param arrayIn — Your Array
 * @param value — Value to push
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayPush({ arrayIn: any[], value: any }): any[];

/**
 * Removes an element from an array at a specific index
 * @param arrayIn — Your Array
 * @param index — Index to remove
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayRemoveIndex({ arrayIn: any[], index: int }): any[];

/**
 * Sets an element at a specific index in an array
 * @param arrayIn — Your Array
 * @param index — Index to set
 * @param value — Value to set
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arraySetIndex({ arrayIn: any[], index: int, value: any }): any[];

/**
 * Shuffle Array Items
 * @param arrayIn — Your Array
 * @returns arrayOut — Adjusted Array
 */
declare function arrayShuffle({ arrayIn: any[] }): any[];

/**
 * Creates an array from individual elements. Add more input pins by connecting to the 'element' pins.
 * @param element — Element to include in the array
 * @param element — Element to include in the array
 * @returns arrayOut — The constructed array
 */
declare function constructArray({ element: any, element: any }): any[];

/**
 * Creates an empty array
 * @returns arrayOut — The created array
 */
declare function makeArray(): any[];


// === Utils/Array/Batch ===

/**
 * Push multiple items into an array in one operation. More efficient than multiple single pushes.
 * @param arrayIn — Your Array
 * @param items — Array of items to push
 * @returns arrayOut — Array with all items pushed
 * @impure has side effects / drives control flow
 */
declare function arrayBatchPush({ arrayIn: any[], items: any[] }): any[];

/**
 * Remove multiple elements at specific indices in one operation. More efficient than multiple single removes. Indices are processed in descending order to maintain correctness.
 * @param arrayIn — Your Array
 * @param indices — Array of indices to remove
 * @returns arrayOut — Array with elements removed
 * @returns removed — Array of removed values
 * @impure has side effects / drives control flow
 */
declare function arrayBatchRemove({ arrayIn: any[], indices: int[] }): { arrayOut: any[], removed: any[] };

/**
 * Set multiple elements at specific indices in one operation. More efficient than multiple single sets.
 * @param arrayIn — Your Array
 * @param indices — Array of indices to set
 * @param values — Array of values to set (must match indices length)
 * @returns arrayOut — Array with all values set
 * @impure has side effects / drives control flow
 */
declare function arrayBatchSet({ arrayIn: any[], indices: int[], values: any[] }): any[];


// === Utils/Array/By Reference ===

/**
 * Clear all elements directly from a variable array without copying.
 * @param varRef — Reference to the array variable to clear
 * @impure has side effects / drives control flow
 */
declare function arrayClearRef({ varRef: string }): void;

/**
 * Append multiple items directly to a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param values — Array of values to append
 * @impure has side effects / drives control flow
 */
declare function arrayExtendRef({ varRef: string, values: any[] }): void;

/**
 * Remove and return the last element directly from a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @returns value — The popped value
 * @impure has side effects / drives control flow
 */
declare function arrayPopRef({ varRef: string }): any;

/**
 * Push an item directly into a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param value — Value to push into the array
 * @impure has side effects / drives control flow
 */
declare function arrayPushRef({ varRef: string, value: any }): void;

/**
 * Remove an element at a specific index directly from a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param index — Index to remove
 * @returns value — The removed value
 * @impure has side effects / drives control flow
 */
declare function arrayRemoveIndexRef({ varRef: string, index: int }): any;

/**
 * Set an element at a specific index directly in a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param index — Index to set
 * @param value — Value to set at the index
 * @impure has side effects / drives control flow
 */
declare function arraySetIndexRef({ varRef: string, index: int, value: any }): void;


// === Utils/Bool ===

/**
 * Boolean And operation
 * @param boolean (optional) — Input Pin for AND Operation
 * @param boolean (optional) — Input Pin for AND Operation
 * @returns result — AND operation between all boolean inputs
 */
declare function boolAnd({ boolean?: bool, boolean?: bool }): bool;

/**
 * Boolean Equal
 * @param boolean (optional) — Input Pin for OR Operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @returns result — == operation between all boolean inputs
 */
declare function boolEqual({ boolean?: bool, boolean?: bool }): bool;

/**
 * Boolean NOT
 * @param boolean (optional) — Input Boolean
 * @returns result — NOT operation on the input
 */
declare function boolNot({ boolean?: bool }): bool;

/**
 * Boolean Or operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @returns result — OR operation between all boolean inputs
 */
declare function boolOr({ boolean?: bool, boolean?: bool }): bool;

/**
 * Boolean XOR
 * @param boolean (optional) — Input Boolean
 * @param boolean (optional) — Input Boolean
 * @returns result — XOR operation between all boolean inputs
 */
declare function boolXor({ boolean?: bool, boolean?: bool }): bool;

/**
 * Generates a random boolean value
 * @param probability (optional) — The probability of the boolean being true
 * @returns value — The random boolean value
 */
declare function randomBool({ probability?: float }): bool;


// === Utils/CSV ===

/**
 * Stream Read a CSV File
 * @param csv — CSV Path
 * @param chunkSize (optional) — Chunk Size for Buffered Read
 * @param delimiter (optional) — Delimiter for CSV
 * @returns chunk — Chunk
 * @impure has side effects / drives control flow
 */
declare function csvBufferedReader({ csv: Struct, chunkSize?: int, delimiter?: string }): Struct[];


// === Utils/Conversions ===

/**
 * Convert String to Bytes
 * @param bytes — Bytes to convert
 * @returns value — Parsed Value
 */
declare function valFromBytes({ bytes: bytes[] }): any;

/**
 * Convert String to Struct
 * @param string — String to convert
 * @returns valueRef — Value of the Generic
 */
declare function valFromString({ string: string }): any;

/**
 * Convert Struct to Bytes
 * @param value — Input Value
 * @param pretty (optional) — Should the struct be pretty printed?
 * @returns bytes — Output Bytes
 */
declare function valToBytes({ value: any, pretty?: bool }): bytes[];

/**
 * Convert any object to String
 * @param value — Input Value
 * @param pretty (optional) — Should the struct be pretty printed?
 * @returns string — Output String
 */
declare function valToString({ value: any, pretty?: bool }): string;


// === Utils/Crypto ===

/**
 * Decrypts an AES-256-GCM encrypted payload and verifies its authentication tag.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns plaintext — Decrypted bytes
 * @impure has side effects / drives control flow
 */
declare function cryptoAesDecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

/**
 * Decrypts an AES-256-GCM payload and parses the plaintext as a struct.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns value — Decrypted struct
 * @impure has side effects / drives control flow
 */
declare function cryptoAesDecryptValue({ key: bytes[], encrypted: Struct }): Struct;

/**
 * Encrypts bytes with AES-256-GCM. A fresh nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param plaintext — Bytes to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoAesEncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

/**
 * Serializes and encrypts a struct with AES-256-GCM. A fresh nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param value — Struct to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoAesEncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;

/**
 * Generates a 256-bit symmetric key for AES-256-GCM and XChaCha20-Poly1305.
 * @returns key — Random 32-byte symmetric key
 * @impure has side effects / drives control flow
 */
declare function cryptoGenerateKey(): bytes[];

/**
 * Decrypts an XChaCha20-Poly1305 encrypted payload and verifies its authentication tag.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns plaintext — Decrypted bytes
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20DecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

/**
 * Decrypts an XChaCha20-Poly1305 payload and parses the plaintext as a struct.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns value — Decrypted struct
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20DecryptValue({ key: bytes[], encrypted: Struct }): Struct;

/**
 * Encrypts bytes with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param plaintext — Bytes to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20EncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

/**
 * Serializes and encrypts a struct with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param value — Struct to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20EncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;


// === Utils/DateTime ===

/**
 * Calculates the duration between two dates
 * @param start — Start date
 * @param end — End date
 * @returns totalSeconds — Total duration in seconds
 * @returns days — Number of days
 * @returns hours — Remaining hours
 * @returns minutes — Remaining minutes
 * @returns seconds — Remaining seconds
 * @returns humanReadable — Human readable duration string
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function utilsDatetimeDiff({ start: Date, end: Date }): { totalSeconds: int, days: int, hours: int, minutes: int, seconds: int, humanReadable: string, errorMessage: string };

/**
 * Adds or subtracts a duration from a date
 * @param date — Base date
 * @param days (optional) — Days to add (negative to subtract)
 * @param hours (optional) — Hours to add
 * @param minutes (optional) — Minutes to add
 * @param seconds (optional) — Seconds to add
 * @returns result — Resulting date
 */
declare function utilsDatetimeDuration({ date: Date, days?: int, hours?: int, minutes?: int, seconds?: int }): Date;

/**
 * Converts a DateTime to a formatted string
 * @param date — Date to format
 * @param format (optional) — Format string (e.g., '%Y-%m-%d %H:%M:%S', '%Y-%m-%d', 'rfc3339', 'rfc2822')
 * @returns formatted — Formatted string
 */
declare function utilsDatetimeFormat({ date: Date, format?: string }): string;

/**
 * Returns the current date and time in UTC
 * @returns date — Current UTC date and time
 * @impure has side effects / drives control flow
 */
declare function utilsDatetimeNow(): Date;

/**
 * Parses a string into a DateTime. Auto-detects common formats or uses custom format string.
 * @param input — String to parse
 * @param format (optional) — Optional format string (e.g., '%Y-%m-%d %H:%M:%S'). Leave empty for auto-detection.
 * @returns date — Parsed date
 */
declare function utilsDatetimeParse({ input: string, format?: string }): Date;

/**
 * Extracts date components from a DateTime
 * @param date — DateTime to extract from
 * @returns year — Year
 * @returns month — Month (1-12)
 * @returns day — Day of month (1-31)
 * @returns weekday — Day of week (0=Monday, 6=Sunday)
 * @returns dayOfYear — Day of year (1-366)
 */
declare function utilsDatetimeToDate({ date: Date }): { year: int, month: int, day: int, weekday: int, dayOfYear: int };

/**
 * Extracts time components from a DateTime
 * @param date — DateTime to extract from
 * @returns hour — Hour (0-23)
 * @returns minute — Minute (0-59)
 * @returns second — Second (0-59)
 * @returns nanosecond — Nanosecond (0-999999999)
 */
declare function utilsDatetimeToTime({ date: Date }): { hour: int, minute: int, second: int, nanosecond: int };


// === Utils/Encoding ===

/**
 * Decodes a Base64 string back to a UTF-8 string
 * @param input — Base64 encoded string
 * @returns output — Decoded UTF-8 string
 */
declare function utilsEncodingBase64Decode({ input: string }): string;

/**
 * Decodes a Base64 string to raw bytes
 * @param input — Base64 encoded string
 * @returns output — Decoded raw bytes
 */
declare function utilsEncodingBase64DecodeBytes({ input: string }): bytes[];

/**
 * Encodes a string to Base64
 * @param input — String to encode
 * @returns output — Base64 encoded string
 */
declare function utilsEncodingBase64Encode({ input: string }): string;

/**
 * Encodes raw bytes to a Base64 string
 * @param input — Raw bytes to encode
 * @returns output — Base64 encoded string
 */
declare function utilsEncodingBase64EncodeBytes({ input: bytes[] }): string;

/**
 * Decodes a hexadecimal string back to a UTF-8 string
 * @param input — Hex-encoded string
 * @returns output — Decoded UTF-8 string
 */
declare function utilsEncodingHexDecode({ input: string }): string;

/**
 * Decodes a hexadecimal string to raw bytes
 * @param input — Hex-encoded string
 * @returns output — Decoded raw bytes
 */
declare function utilsEncodingHexDecodeBytes({ input: string }): bytes[];

/**
 * Encodes a string's bytes to a hexadecimal string
 * @param input — String to encode
 * @returns output — Hex-encoded string
 */
declare function utilsEncodingHexEncode({ input: string }): string;

/**
 * Encodes raw bytes to a hexadecimal string
 * @param input — Raw bytes to encode
 * @returns output — Hex-encoded string
 */
declare function utilsEncodingHexEncodeBytes({ input: bytes[] }): string;

/**
 * Decodes HTML entities back to their original characters
 * @param input — HTML-encoded string
 * @returns output — Decoded string
 */
declare function utilsEncodingHtmlDecode({ input: string }): string;

/**
 * Encodes special characters as HTML entities (&amp; &lt; &gt; &quot; &#39;)
 * @param input — String to encode
 * @returns output — HTML-encoded string
 */
declare function utilsEncodingHtmlEncode({ input: string }): string;

/**
 * Decodes a percent-encoded URL string back to plain text
 * @param input — URL-encoded string
 * @returns output — Decoded string
 */
declare function utilsEncodingUrlDecode({ input: string }): string;

/**
 * Percent-encodes a string for safe use in URLs (RFC 3986)
 * @param input — String to encode
 * @returns output — URL-encoded string
 */
declare function utilsEncodingUrlEncode({ input: string }): string;


// === Utils/Execution ===

/**
 * Returns the current app identifier.
 * @returns appId — Current app identifier
 */
declare function utilsExecutionGetAppId(): string;

/**
 * Returns where and how the current run is executing.
 * @returns environment — The execution environment: local, desktop, mobile, browser_sandbox, or server
 * @returns executionMode — The execution mode: sync, async, event, or scheduled
 * @returns isDesktop — True when the run is executing locally in the desktop app
 * @returns isServer — True when the run is executing on the server
 * @returns isMobile — True when the run is executing on a mobile runtime
 * @returns isBrowserSandbox — True when the run is executing in a browser sandbox runtime
 * @returns isLocal — True when the run has local/offline execution context
 * @returns isRemote — True when the run does not have local/offline execution context
 * @returns runId — Current run identifier
 * @returns appId — Current app identifier, if available
 * @returns userId — Current user identifier, if available
 * @returns details — Structured execution environment details
 */
declare function utilsExecutionGetEnvironment(): { environment: string, executionMode: string, isDesktop: bool, isServer: bool, isMobile: bool, isBrowserSandbox: bool, isLocal: bool, isRemote: bool, runId: string, appId: string, userId: string, details: Struct };

/**
 * Returns the current execution mode.
 * @returns mode — The execution mode: sync, async, event, or scheduled
 */
declare function utilsExecutionGetMode(): string;

/**
 * Returns the current execution run identifier.
 * @returns runId — Current run identifier
 */
declare function utilsExecutionGetRunId(): string;

/**
 * Returns the current user identifier, when available.
 * @returns userId — Current user identifier, or empty when unavailable
 */
declare function utilsExecutionGetUserId(): string;

/**
 * Returns true when the current run is executing on a local/client runtime.
 * @returns isLocal — True for local, desktop, mobile, and browser sandbox execution
 */
declare function utilsExecutionIsLocalEnvironment(): bool;

/**
 * Returns true when the current run is executing on a mobile runtime.
 * @returns isMobile — True for mobile execution
 */
declare function utilsExecutionIsMobileEnvironment(): bool;

/**
 * Returns true when the current run is executing on the server.
 * @returns isServer — True for server-side execution
 */
declare function utilsExecutionIsServerEnvironment(): bool;


// === Utils/Faker/Address ===

/**
 * Generates a random city name for mocking data
 * @returns city — Generated city name
 * @impure has side effects / drives control flow
 */
declare function fakerCityName(): string;

/**
 * Generates a random country code (e.g., US, DE, FR) for mocking data
 * @returns code — Generated country code
 * @impure has side effects / drives control flow
 */
declare function fakerCountryCode(): string;

/**
 * Generates a random country name for mocking data
 * @returns country — Generated country name
 * @impure has side effects / drives control flow
 */
declare function fakerCountryName(): string;

/**
 * Generates a random latitude coordinate for mocking data
 * @returns latitude — Generated latitude
 * @impure has side effects / drives control flow
 */
declare function fakerLatitude(): float;

/**
 * Generates a random longitude coordinate for mocking data
 * @returns longitude — Generated longitude
 * @impure has side effects / drives control flow
 */
declare function fakerLongitude(): float;

/**
 * Generates a random postal/zip code for mocking data
 * @returns code — Generated postal code
 * @impure has side effects / drives control flow
 */
declare function fakerPostCode(): string;

/**
 * Generates a random state/province name for mocking data
 * @returns state — Generated state name
 * @impure has side effects / drives control flow
 */
declare function fakerStateName(): string;

/**
 * Generates a random full street address for mocking data
 * @returns address — Generated street address
 * @impure has side effects / drives control flow
 */
declare function fakerStreetAddress(): string;

/**
 * Generates a random street name for mocking data
 * @returns street — Generated street name
 * @impure has side effects / drives control flow
 */
declare function fakerStreetName(): string;


// === Utils/Faker/Company ===

/**
 * Generates a random business buzzword for mocking data
 * @returns buzzword — Generated buzzword
 * @impure has side effects / drives control flow
 */
declare function fakerBuzzword(): string;

/**
 * Generates a random business catch phrase for mocking data
 * @returns phrase — Generated catch phrase
 * @impure has side effects / drives control flow
 */
declare function fakerCatchPhrase(): string;

/**
 * Generates a random company name for mocking data
 * @returns company — Generated company name
 * @impure has side effects / drives control flow
 */
declare function fakerCompanyName(): string;

/**
 * Generates a random industry name for mocking data
 * @returns industry — Generated industry name
 * @impure has side effects / drives control flow
 */
declare function fakerIndustry(): string;

/**
 * Generates a random profession/job title for mocking data
 * @returns profession — Generated profession
 * @impure has side effects / drives control flow
 */
declare function fakerProfession(): string;


// === Utils/Faker/Internet ===

/**
 * Generates a random domain suffix (com, org, net, etc.)
 * @returns suffix — Generated domain suffix
 * @impure has side effects / drives control flow
 */
declare function fakerDomainSuffix(): string;

/**
 * Generates a random email address for mocking data
 * @returns email — Generated email address
 * @impure has side effects / drives control flow
 */
declare function fakerEmail(): string;

/**
 * Generates a random IPv4 address for mocking data
 * @returns ip — Generated IPv4 address
 * @impure has side effects / drives control flow
 */
declare function fakerIpv4(): string;

/**
 * Generates a random IPv6 address for mocking data
 * @returns ip — Generated IPv6 address
 * @impure has side effects / drives control flow
 */
declare function fakerIpv6(): string;

/**
 * Generates a random password for mocking data
 * @param minLength (optional) — Minimum password length
 * @param maxLength (optional) — Maximum password length
 * @returns password — Generated password
 * @impure has side effects / drives control flow
 */
declare function fakerPassword({ minLength?: int, maxLength?: int }): string;

/**
 * Generates a random user agent string for mocking data
 * @returns userAgent — Generated user agent
 * @impure has side effects / drives control flow
 */
declare function fakerUserAgent(): string;

/**
 * Generates a random username for mocking data
 * @returns username — Generated username
 * @impure has side effects / drives control flow
 */
declare function fakerUsername(): string;


// === Utils/Faker/Lorem ===

/**
 * Generates a random lorem ipsum paragraph for mocking data
 * @param minSentences (optional) — Minimum sentences in paragraph
 * @param maxSentences (optional) — Maximum sentences in paragraph
 * @returns paragraph — Generated paragraph
 * @impure has side effects / drives control flow
 */
declare function fakerParagraph({ minSentences?: int, maxSentences?: int }): string;

/**
 * Generates random lorem ipsum paragraphs for mocking data
 * @param minCount (optional) — Minimum number of paragraphs
 * @param maxCount (optional) — Maximum number of paragraphs
 * @returns paragraphs — Generated paragraphs as array
 * @impure has side effects / drives control flow
 */
declare function fakerParagraphs({ minCount?: int, maxCount?: int }): any;

/**
 * Generates a random lorem ipsum sentence for mocking data
 * @param minWords (optional) — Minimum words in sentence
 * @param maxWords (optional) — Maximum words in sentence
 * @returns sentence — Generated sentence
 * @impure has side effects / drives control flow
 */
declare function fakerSentence({ minWords?: int, maxWords?: int }): string;

/**
 * Generates random lorem ipsum sentences for mocking data
 * @param minCount (optional) — Minimum number of sentences
 * @param maxCount (optional) — Maximum number of sentences
 * @returns sentences — Generated sentences as array
 * @impure has side effects / drives control flow
 */
declare function fakerSentences({ minCount?: int, maxCount?: int }): any;

/**
 * Generates a random lorem ipsum word for mocking data
 * @returns word — Generated word
 * @impure has side effects / drives control flow
 */
declare function fakerWord(): string;

/**
 * Generates random lorem ipsum words for mocking data
 * @param minCount (optional) — Minimum number of words
 * @param maxCount (optional) — Maximum number of words
 * @returns words — Generated words as array
 * @impure has side effects / drives control flow
 */
declare function fakerWords({ minCount?: int, maxCount?: int }): any;


// === Utils/Faker/Name ===

/**
 * Generates a random first name for mocking data
 * @returns name — Generated first name
 * @impure has side effects / drives control flow
 */
declare function fakerFirstName(): string;

/**
 * Generates a random full name for mocking data
 * @returns name — Generated full name
 * @impure has side effects / drives control flow
 */
declare function fakerFullName(): string;

/**
 * Generates a random last name for mocking data
 * @returns name — Generated last name
 * @impure has side effects / drives control flow
 */
declare function fakerLastName(): string;

/**
 * Generates a random name title (Mr., Mrs., Dr., etc.)
 * @returns title — Generated title
 * @impure has side effects / drives control flow
 */
declare function fakerTitle(): string;


// === Utils/Faker/Number ===

/**
 * Generates a random boolean for mocking data
 * @param probability (optional) — Probability of true (0.0 to 1.0)
 * @returns value — Generated boolean
 * @impure has side effects / drives control flow
 */
declare function fakerBoolean({ probability?: float }): bool;

/**
 * Generates a random digit (0-9) for mocking data
 * @returns digit — Generated digit
 * @impure has side effects / drives control flow
 */
declare function fakerDigit(): int;

/**
 * Generates a random float in a specified range for mocking data
 * @param min (optional) — Minimum value (inclusive)
 * @param max (optional) — Maximum value (exclusive)
 * @returns number — Generated float
 * @impure has side effects / drives control flow
 */
declare function fakerFloat({ min?: float, max?: float }): float;

/**
 * Generates a random integer in a specified range for mocking data
 * @param min (optional) — Minimum value (inclusive)
 * @param max (optional) — Maximum value (exclusive)
 * @returns number — Generated integer
 * @impure has side effects / drives control flow
 */
declare function fakerInteger({ min?: int, max?: int }): int;


// === Utils/Faker/Phone ===

/**
 * Generates a random cell/mobile phone number for mocking data
 * @returns phone — Generated cell number
 * @impure has side effects / drives control flow
 */
declare function fakerCellNumber(): string;

/**
 * Generates a random phone number for mocking data
 * @returns phone — Generated phone number
 * @impure has side effects / drives control flow
 */
declare function fakerPhoneNumber(): string;


// === Utils/Hash ===

/**
 * Computes the AHash of the input
 * @param input — Input data to hash
 * @param consistent (optional) — Use consistent hashing
 * @param seed (optional) — Seed value for consistent hashing
 * @returns hash — AHash of the input
 * @impure has side effects / drives control flow
 */
declare function utilsHashAhash({ input: any, consistent?: bool, seed?: int }): int;

/**
 * Computes the Blake3 hash of the input
 * @param input — Input data to hash
 * @returns hash — Blake3 hash of the input
 * @impure has side effects / drives control flow
 */
declare function utilsHashBlake3({ input: any }): string;

/**
 * Computes the MD5 hash of the input string. Note: MD5 is not collision-resistant — use SHA-256 or Blake3 for security-sensitive hashing.
 * @param input — String to hash
 * @returns hash — MD5 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashMd5({ input: string }): string;

/**
 * Computes the SHA-256 hash of the input string
 * @param input — String to hash
 * @returns hash — SHA-256 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashSha256({ input: string }): string;

/**
 * Computes the SHA-512 hash of the input string
 * @param input — String to hash
 * @returns hash — SHA-512 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashSha512({ input: string }): string;


// === Utils/JSON ===

/**
 * Parse JSON input Data With JSON/OpenAI Schema and Return Value
 * @param schema — JSON Schema or OpenAI Function Definition
 * @param data — JSON Input Data to be parsed
 * @returns parsed — Parsed and Validated JSON
 * @impure has side effects / drives control flow
 */
declare function parseWithSchema({ schema: string, data: string }): Struct;

/**
 * Attempts to repair and parse potentially malformed JSON
 * @param jsonString — String containing potentially malformed JSON
 * @returns result — The parsed JSON structure
 * @impure has side effects / drives control flow
 */
declare function repairParse({ jsonString: string }): Struct;

/**
 * Generate Tool Definitions for Tool Calls
 * @param exampleJson — Example JSON to infer schema from
 * @returns schema — Generated JSON Schema / Tool Definition
 * @impure has side effects / drives control flow
 */
declare function utilsJsonMakeSchema({ exampleJson: string }): Struct;


// === Utils/Map ===

/**
 * Creates an empty map (string keys)
 * @returns mapOut — The created map
 */
declare function makeMap(): Map<string, any>;

/**
 * Removes all entries from a map
 * @param mapIn — Your Map
 * @returns mapOut — Empty Map
 * @impure has side effects / drives control flow
 */
declare function mapClear({ mapIn: Map<string, any> }): Map<string, any>;

/**
 * Gets a value from a map by key
 * @param mapIn — Your Map
 * @param key — Key to get
 * @returns value — Value at the specified key
 * @returns found — Was the key found in the map?
 */
declare function mapGet({ mapIn: Map<string, any>, key: string }): { value: any, found: bool };

/**
 * Checks if a key exists in the map
 * @param mapIn — Your Map
 * @param key — Key to check
 * @returns hasKey — Does the map contain the key?
 */
declare function mapHasKey({ mapIn: Map<string, any>, key: string }): bool;

/**
 * Gets all keys from the map as an array
 * @param mapIn — Your Map
 * @returns keys — Array of all keys
 */
declare function mapKeys({ mapIn: Map<string, any> }): any[];

/**
 * Removes a key from the map
 * @param mapIn — Your Map
 * @param key — Key to remove
 * @returns mapOut — Adjusted Map
 * @returns value — The removed value (null if key not found)
 * @returns wasPresent — Was the key in the map?
 * @impure has side effects / drives control flow
 */
declare function mapRemove({ mapIn: Map<string, any>, key: string }): { mapOut: Map<string, any>, value: any, wasPresent: bool };

/**
 * Sets a value in a map at the given key
 * @param mapIn — Your Map
 * @param key — Key to set
 * @param value — Value to set
 * @returns mapOut — Adjusted Map
 * @returns replaced — Was an existing value replaced?
 * @impure has side effects / drives control flow
 */
declare function mapSet({ mapIn: Map<string, any>, key: string, value: any }): { mapOut: Map<string, any>, replaced: bool };

/**
 * Gets the number of entries in the map
 * @param mapIn — Your Map
 * @returns size — Number of entries in the map
 */
declare function mapSize({ mapIn: Map<string, any> }): int;

/**
 * Gets all values from the map as an array
 * @param mapIn — Your Map
 * @returns values — Array of all values
 */
declare function mapValues({ mapIn: Map<string, any> }): any[];


// === Utils/Map/By Reference ===

/**
 * Clear all entries directly from a variable map without copying.
 * @param varRef — Reference to the map variable to clear
 * @impure has side effects / drives control flow
 */
declare function mapClearRef({ varRef: string }): void;

/**
 * Remove a key directly from a variable map without copying. Much faster for large maps.
 * @param varRef — Reference to the map variable to modify
 * @param key — Key to remove
 * @returns value — The removed value (null if key not found)
 * @returns wasPresent — Was the key in the map?
 * @impure has side effects / drives control flow
 */
declare function mapRemoveRef({ varRef: string, key: string }): { value: any, wasPresent: bool };

/**
 * Set a value directly in a variable map without copying. Much faster for large maps.
 * @param varRef — Reference to the map variable to modify
 * @param key — Key to set
 * @param value — Value to set at the key
 * @impure has side effects / drives control flow
 */
declare function mapSetRef({ varRef: string, key: string, value: any }): void;


// === Utils/Markdown ===

/**
 * Attempts to convert HTML to Markdown, removing unwanted tags
 * @param html — Html to Parse
 * @param skippedTags (optional) — Tags to skip
 * @returns markdown — The parsed Markdown
 * @impure has side effects / drives control flow
 */
declare function utilsMdHtmlToMd({ html: string, skippedTags?: string[] }): string;


// === Utils/Math/Vector ===

/**
 * Adds two float vectors together element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Sum of the two vectors
 */
declare function floatVectorAddition({ vector1: float[], vector2: float[] }): float[];

/**
 * Calculates the cosine similarity of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns similarity — Cosine similarity of the two vectors
 */
declare function floatVectorCosineSimilarity({ vector1: float[], vector2: float[] }): float;

/**
 * Calculates the cross product of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Cross product of the two vectors
 */
declare function floatVectorCrossProduct({ vector1: float[], vector2: float[] }): float[];

/**
 * Calculates the dot product of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns result — Dot product of the two vectors
 */
declare function floatVectorDotProduct({ vector1: float[], vector2: float[] }): float;

/**
 * Multiplies two float vectors element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Element-wise product of the two vectors
 */
declare function floatVectorMultiplication({ vector1: float[], vector2: float[] }): float[];

/**
 * Normalizes a float vector
 * @param vector — Float vector to normalize
 * @returns normalizedVector — Normalized float vector
 */
declare function floatVectorNormalize({ vector: float[] }): float[];

/**
 * Subtracts one float vector from another element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Element-wise difference of the two vectors
 */
declare function floatVectorSubtraction({ vector1: float[], vector2: float[] }): float[];


// === Utils/Set ===

/**
 * Converts an array to a set
 * @param arrayIn
 * @returns setOut
 */
declare function arrayToSet({ arrayIn: any[] }): Set<any>;

/**
 * Creates a set from the difference of 2 sets
 * @param setIn1 — Your First Set
 * @param setIn2 — Your Second Set
 * @returns setOut — The difference set
 * @impure has side effects / drives control flow
 */
declare function difference({ setIn1: Set<any>, setIn2: Set<any> }): Set<any>;

/**
 * Inserts an element to the set
 * @param setIn — Your Set
 * @param value — Value to push
 * @returns setOut — Adjusted Set
 * @returns existedBefore — Was the element there before?
 * @impure has side effects / drives control flow
 */
declare function insert({ setIn: Set<any>, value: any }): { setOut: Set<any>, existedBefore: bool };

/**
 * Checks if one of the hash sets has at least one mutual element
 * @param setIn1
 * @param setIn2
 * @returns isMutual — Does it include a mutual element that both sets share or not?
 * @impure has side effects / drives control flow
 */
declare function isMutual({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Creates an empty set
 * @returns setOut — The created set
 */
declare function makeSet(): Set<any>;

/**
 * Removes / Clears all elements from a set
 * @param setIn — Your Set
 * @returns setOut — Empty Set
 * @impure has side effects / drives control flow
 */
declare function setClear({ setIn: Set<any> }): Set<any>;

/**
 * Discards an element of a set
 * @param setIn — Your Set
 * @param value — Value to remove
 * @returns setOut — Adjusted Set
 * @returns hasRemoved — If the element was removed
 * @impure has side effects / drives control flow
 */
declare function setDiscard({ setIn: Set<any>, value: any }): { setOut: Set<any>, hasRemoved: bool };

/**
 * Gets the size of the hash set (how many elements)
 * @param setIn — Your Set
 * @returns size — How many elements does it have
 */
declare function setGetSize({ setIn: Set<any> }): int;

/**
 * Checks if an element is present in the set
 * @param setIn — Your Set
 * @param value — Value to search for
 * @returns contains — Does the set include the value?
 */
declare function setHas({ setIn: Set<any>, value: any }): bool;

/**
 * Checks if a hash set is empty or not
 * @param setIn — Your Set
 * @returns isEmpty — Does it have any values or not?
 */
declare function setIsEmpty({ setIn: Set<any> }): bool;

/**
 * Checks if a hash set is a subset from a supposed bigger one
 * @param setIn1 — Your Smaller Set
 * @param setIn2 — Your Bigger Set
 * @returns isSubset — Is the first set a subset of the second?
 */
declare function setIsSubset({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Checks if a hash set is a superset from a supposed smaller one
 * @param setIn1 — Your Bigger Set
 * @param setIn2 — Your Smaller Set
 * @returns isSuperset — Is the first set a superset of the second?
 */
declare function setIsSuperset({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Pops a random element of a set
 * @param setIn — Your Set
 * @returns setOut — Adjusted Set
 * @impure has side effects / drives control flow
 */
declare function setPop({ setIn: Set<any> }): Set<any>;

/**
 * Converts a set to an array
 * @param setIn
 * @returns arrayOut
 */
declare function setToArray({ setIn: Set<any> }): any[];

/**
 * Combines 2 sets into one unified hash set
 * @param setIn1 — Your First Set
 * @param setIn2 — Your Second Set
 * @returns setOut — Combined Set
 * @impure has side effects / drives control flow
 */
declare function union({ setIn1: Set<any>, setIn2: Set<any> }): Set<any>;


// === Utils/Set/By Reference ===

/**
 * Clear all elements directly from a variable set without copying.
 * @param varRef — Reference to the set variable to clear
 * @impure has side effects / drives control flow
 */
declare function setClearRef({ varRef: string }): void;

/**
 * Remove an element directly from a variable set without copying. Much faster for large sets.
 * @param varRef — Reference to the set variable to modify
 * @param value — Value to remove from the set
 * @returns wasPresent — True if the element was in the set and removed
 * @impure has side effects / drives control flow
 */
declare function setDiscardRef({ varRef: string, value: any }): bool;

/**
 * Insert an element directly into a variable set without copying. Much faster for large sets.
 * @param varRef — Reference to the set variable to modify
 * @param value — Value to insert into the set
 * @returns wasNew — True if the element was not already in the set
 * @impure has side effects / drives control flow
 */
declare function setInsertRef({ varRef: string, value: any }): bool;


// === Utils/String ===

/**
 * Compares two Strings
 * @param string — Input
 * @param string — Input
 * @returns equal — Are the strings equal?
 */
declare function equalString({ string: string, string: string }): bool;

/**
 * Compares two Strings
 * @param string — Input
 * @param string — Input
 * @returns unequal — Are the strings equal?
 */
declare function notEqualString({ string: string, string: string }): bool;

/**
 * Checks if a string contains a substring
 * @param string — Input String
 * @param substring — Substring to search for
 * @returns contains — Does the string contain the substring?
 */
declare function stringContains({ string: string, substring: string }): bool;

/**
 * Checks if a string ends with a specific string
 * @param string — Input String
 * @param suffix — String to check against
 * @returns endsWith — Does the string end with the suffix?
 */
declare function stringEndsWith({ string: string, suffix: string }): bool;

/**
 * Escapes special characters in a string (newlines, tabs, carriage returns, backslashes, quotes).
 * @param string — Input String
 * @returns escaped — String with special characters escaped
 */
declare function stringEscape({ string: string }): string;

/**
 * Formats a string with placeholders
 * @param formatString — String with placeholders
 * @returns formattedString — Formatted string
 */
declare function stringFormat({ formatString: string }): string;

/**
 * Joins multiple strings together
 * @param strings — Strings to join
 * @param separator — String to separate by
 * @returns joinedString — Concatenated string
 */
declare function stringJoin({ strings: string[], separator: string }): string;

/**
 * Calculates the length of a string
 * @param string — Input String
 * @returns length — Length of the string
 */
declare function stringLength({ string: string }): int;

/**
 * Template Engine based on Jinja Templates
 * @param template — Jinja Template String
 * @returns rendered — Rendered String
 */
declare function stringRenderTemplate({ template: string }): string;

/**
 * Replaces occurrences of a substring or regex pattern within a string.
 * @param string — Input String
 * @param pattern — Substring or regex pattern to replace
 * @param replacement — Replacement string (supports $1, $2, ... for regex capture groups)
 * @param isRegex (optional) — Treat the pattern as a regular expression
 * @returns newString — String with replacements
 */
declare function stringReplace({ string: string, pattern: string, replacement: string, isRegex?: bool }): string;

/**
 * Splits a string into substrings
 * @param string — Input String
 * @param separator — String to split by
 * @returns substrings — Array of substrings
 */
declare function stringSplit({ string: string, separator: string }): string[];

/**
 * Checks if a string starts with a specific string
 * @param string — Input String
 * @param prefix — String to check against
 * @returns startsWith — Does the string start with the prefix?
 */
declare function stringStartsWith({ string: string, prefix: string }): bool;

/**
 * Converts a string to lowercase
 * @param string — Input String
 * @returns lowercaseString — String in lowercase
 */
declare function stringToLower({ string: string }): string;

/**
 * Converts a string to uppercase
 * @param string — Input String
 * @returns uppercaseString — String in uppercase
 */
declare function stringToUpper({ string: string }): string;

/**
 * Removes leading and trailing whitespace from a string
 * @param string — Input String
 * @returns trimmedString — String without leading/trailing whitespace
 */
declare function stringTrim({ string: string }): string;

/**
 * Unescapes special character sequences in a string (\n, \t, \r, \\, \").
 * @param string — Input String
 * @returns unescaped — String with escape sequences resolved to actual characters
 */
declare function stringUnescape({ string: string }): string;

/**
 * Converts a byte array to a string using the UTF-8 lossy strategy
 * @param bytes
 * @returns string — Input String
 */
declare function utf8Lossy({ bytes: bytes[] }): string;


// === Utils/String/Similarity ===

/**
 * Calculates the Damerau-Levenshtein distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @param normalize (optional) — Normalize the Distance
 * @returns distance — Damerau-Levenshtein Distance
 */
declare function damerauLevenshteinDistance({ string1: string, string2: string, normalize?: bool }): float;

/**
 * Calculates the Hamming distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Hamming Distance
 */
declare function hammingDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Jaro distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Jaro Distance
 */
declare function jaroDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Jaro-Winkler distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Jaro-Winkler Distance
 */
declare function jaroWinklerDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Levenshtein distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @param normalize (optional) — Normalize the Distance
 * @returns distance — Levenshtein Distance
 */
declare function levenshteinDistance({ string1: string, string2: string, normalize?: bool }): float;

/**
 * Calculates the Optimal String Alignment distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Optimal String Alignment Distance
 */
declare function optimalStringAlignmentDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Sørensen-Dice coefficient between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns coefficient — Sørensen-Dice Coefficient
 */
declare function sorensenDiceCoefficient({ string1: string, string2: string }): float;


// === Utils/Types ===

/**
 * Returns the input value if valid, otherwise returns the fallback default. Useful for handling optional values or error recovery.
 * @param value — The primary value to use if available and valid
 * @param default — Fallback value used when the primary value is null, missing, or invalid
 * @returns result — The resolved value (primary if valid, otherwise default)
 * @returns usedFallback — True if the fallback value was used
 */
declare function utilsTypesFallback({ value: any, default: any }): { result: any, usedFallback: bool };

/**
 * Selects between two values based on a boolean condition. Returns A if true, B if false.
 * @param a — Value returned when condition is true
 * @param b — Value returned when condition is false
 * @param condition (optional) — If true, returns A. If false, returns B.
 * @returns result — The selected value (A if true, B if false)
 */
declare function utilsTypesSelect({ a: any, b: any, condition?: bool }): any;

/**
 * Tries to transform cast types.
 * @param typeIn — Type to transform
 * @returns typeOut — If the type was successfully transformed, transformed type
 * @returns success — Determines of tje transformation was successful
 */
declare function utilsTypesTryTransform({ typeIn: any }): { typeOut: any, success: bool };


// === Utils/User ===

/**
 * Checks whether a project user has the specified role ID or exact role name.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @param role (optional) — Role ID or exact role name.
 * @returns hasRole — True when the user has the requested role.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserCheckUserHasRole({ appId?: string, userId?: string, role?: string }): { hasRole: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks whether a project user effectively has a permission. Owner and Admin imply all permissions.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @param permission (optional) — Permission name or bit value to check.
 * @returns hasPermission — True when the user effectively has the requested permission.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserCheckUserPermission({ appId?: string, userId?: string, permission?: string }): { hasPermission: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the current runtime user and, when available, their project membership, role, effective permissions, and attributes.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @returns currentUser — Current runtime user with project membership details when available.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetCurrentUser({ appId?: string }): { currentUser: Struct, success: bool, statusCode: int, error: string };

/**
 * Fetches the current user's persisted user information from the configured FlowLike hub's /api/v1/user/info endpoint when an execution token is available.
 * @returns userInfo — The user record returned by /api/v1/user/info
 * @returns success — True when user info was fetched successfully
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made
 * @returns error — Error message when user info could not be fetched
 */
declare function utilsUserGetCurrentUserInfo(): { userInfo: Struct, success: bool, statusCode: int, error: string };

/**
 * Gets a project user's effective permission bitfield and expanded permission names. Owner and Admin imply all permissions.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userPermissions — Effective permissions for the project user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetEffectiveUserPermissions({ appId?: string, userId?: string }): { userPermissions: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the user context of the current execution. Returns a typed struct containing sub (user ID), role, permissions, attributes, and technical user info. Use 'Break Struct' to access individual fields.
 * @returns userContext — The complete user execution context. Use 'Break Struct' to access: sub, role (with id, name, permissions, attributes), is_technical_user, key_id
 * @returns hasUser — True if user context is available
 */
declare function utilsUserGetExecutingUser(): { userContext: Struct, hasUser: bool };

/**
 * Gets a project user membership by user ID/sub.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetProjectUser({ appId?: string, userId?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks for one custom role attribute on a project user.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @param attribute (optional) — Role attribute to read.
 * @returns hasAttribute — True when the user has the requested attribute.
 * @returns attributeValue — The matching attribute when present.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetUserAttribute({ appId?: string, userId?: string, attribute?: string }): { hasAttribute: bool, attributeValue: string, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets custom role attributes assigned to a project user.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userAttributes — Role attributes for the project user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetUserAttributes({ appId?: string, userId?: string }): { userAttributes: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the project role assigned to a user.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userRoles — Role assignment for the project user. Current projects have one role per user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetUserRoles({ appId?: string, userId?: string }): { userRoles: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks if the executing user's role has a specific attribute (tag). Attributes are custom string tags assigned to roles for flexible authorization. Returns false if no user context is available or the user has no role.
 * @param attribute (optional) — The attribute (tag) to check for
 * @returns hasAttribute — True if the user's role has the specified attribute
 */
declare function utilsUserHasAttribute({ attribute?: string }): bool;

/**
 * Checks if the executing user has a specific permission. Admin and Owner roles automatically have all permissions. Returns false if no user context is available.
 * @param permission (optional) — The permission to check for
 * @returns hasPermission — True if the user has the specified permission (or is Admin/Owner)
 */
declare function utilsUserHasPermission({ permission?: string }): bool;

/**
 * Checks if the current execution is triggered by a technical user (API key) rather than a human user. Technical users don't have a human identity (sub) but do have a key_id.
 * @returns isTechnical — True if the execution is by a technical user (API key), false otherwise
 * @returns keyId — The API key identifier for technical users, empty string for human users
 */
declare function utilsUserIsTechnicalUser(): { isTechnical: bool, keyId: string };

/**
 * Lists project users with pagination.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param offset (optional) — Number of matching users to skip.
 * @param limit (optional) — Maximum number of users to return, capped at 100.
 * @returns users — Matching project users.
 * @returns count — Number of users returned.
 * @returns nextOffset — Offset to use for the next page.
 * @returns hasMore — True when another page may contain more matching users.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserListProjectUsers({ appId?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Lists project users whose assigned role contains a custom attribute.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param attribute (optional) — Role attribute to match.
 * @param offset (optional) — Number of matching users to skip.
 * @param limit (optional) — Maximum number of users to return, capped at 100.
 * @returns users — Matching project users.
 * @returns count — Number of users returned.
 * @returns nextOffset — Offset to use for the next page.
 * @returns hasMore — True when another page may contain more matching users.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserListUsersWithAttribute({ appId?: string, attribute?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Lists project users assigned to a role ID or exact role name.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param role (optional) — Role ID or exact role name. Leave empty to return all project users.
 * @param offset (optional) — Number of matching users to skip.
 * @param limit (optional) — Maximum number of users to return, capped at 100.
 * @returns users — Matching project users.
 * @returns count — Number of users returned.
 * @returns nextOffset — Offset to use for the next page.
 * @returns hasMore — True when another page may contain more matching users.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserListUsersWithRole({ appId?: string, role?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Resolves a project user by user ID/sub or by email when email is exposed by platform lookup settings. Email matching is constrained to project members.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param identifier (optional) — Email, sub, or user ID to resolve within the project.
 * @param identifierType (optional) — How to interpret the identifier.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserResolveUser({ appId?: string, identifier?: string, identifierType?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Searches project users by exposed profile fields. Email is only searchable when the platform returns email in user lookup results.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param query (optional) — Search text matched against project user ID, username, preferred username, name, visible email, or role name.
 * @param offset (optional) — Number of matching users to skip.
 * @param limit (optional) — Maximum number of users to return, capped at 100.
 * @returns users — Matching project users.
 * @returns count — Number of users returned.
 * @returns nextOffset — Offset to use for the next page.
 * @returns hasMore — True when another page may contain more matching users.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserSearchUsers({ appId?: string, query?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };


// === Variable ===

/**
 * Get Variable Value
 * @param varRef — The reference to the variable
 * @returns valueRef — The value of the variable
 */
declare function variableGet({ varRef: string }): any;

/**
 * Set Variable Value
 * @param varRef — The reference to the variable
 * @param valueIn — The value of the variable
 * @returns valueRef — The newly set value
 * @impure has side effects / drives control flow
 */
declare function variableSet({ varRef: string, valueIn: any }): any;

