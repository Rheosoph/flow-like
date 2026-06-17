// automation — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Automation ===

/**
 * Starts a unified automation session for desktop, browser, and RPA automation
 * @param defaultDelayMs (optional) — Default delay between actions in milliseconds
 * @param clickDelayMs (optional) — Delay between mouse move and click to ensure registration
 * @param debugMode (optional) — Enable debug mode for verbose logging
 * @returns session — Unified automation session for all operations
 * @impure has side effects / drives control flow
 */
declare function automationStartSession({ defaultDelayMs?: int, clickDelayMs?: int, debugMode?: bool }): Struct;

/**
 * Stops an automation session and releases all resources
 * @param session — Automation session to stop
 * @impure has side effects / drives control flow
 */
declare function automationStopSession({ session: Struct }): void;


// === Automation/Browser ===

/**
 * Closes an open browser context and releases resources
 * @param session — Automation session with browser to close
 * @impure has side effects / drives control flow
 */
declare function browserClose({ session: Struct }): void;

/**
 * Closes a browser page/tab
 * @param session — Automation session with page to close
 * @impure has side effects / drives control flow
 */
declare function browserClosePage({ session: Struct }): void;

/**
 * Creates a new browser page/tab in the given context
 * @param session — Automation session with browser attached
 * @returns sessionOut — Automation session with new page set as current
 * @impure has side effects / drives control flow
 */
declare function browserNewPage({ session: Struct }): Struct;

/**
 * Connects to a WebDriver server and opens a new browser session
 * @param session — Automation session to attach browser to
 * @param webdriverUrl (optional) — URL of the WebDriver server (e.g., http://localhost:9515 for ChromeDriver)
 * @param browserType (optional) — Browser to use (Chrome, Firefox, Edge, Safari)
 * @param headless (optional) — Run browser in headless mode (no visible window)
 * @param viewportWidth (optional) — Browser viewport width in pixels
 * @param viewportHeight (optional) — Browser viewport height in pixels
 * @param userAgent (optional) — Custom user agent string (optional)
 * @param pageLoadTimeout (optional) — Timeout for page loads in seconds
 * @returns sessionOut — Automation session with browser attached
 * @impure has side effects / drives control flow
 */
declare function browserOpen({ session: Struct, webdriverUrl?: string, browserType?: string, headless?: bool, viewportWidth?: int, viewportHeight?: int, userAgent?: string, pageLoadTimeout?: int }): Struct;


// === Automation/Browser/Auth ===

/**
 * Clears all cookies from the browser session
 * @param session — Automation session
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserClearCookies({ session: Struct }): Struct;

/**
 * Loads cookies from a file into the browser session
 * @param session — Automation session
 * @param filePath — Path to cookies JSON file
 * @returns sessionOut — Automation session (pass-through)
 * @returns cookieCount — Number of cookies loaded
 * @impure has side effects / drives control flow
 */
declare function browserLoadCookies({ session: Struct, filePath: Struct }): { sessionOut: Struct, cookieCount: int };

/**
 * Saves all browser cookies to a file for later restoration
 * @param session — Automation session
 * @param filePath — Path to save cookies JSON file
 * @returns sessionOut — Automation session (pass-through)
 * @returns cookieCount — Number of cookies saved
 * @impure has side effects / drives control flow
 */
declare function browserSaveCookies({ session: Struct, filePath: Struct }): { sessionOut: Struct, cookieCount: int };

/**
 * Configures HTTP Basic Authentication credentials for requests
 * @param session — Automation session
 * @param username (optional) — HTTP Basic Auth username
 * @param password (optional) — HTTP Basic Auth password
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserSetBasicAuth({ session: Struct, username?: string, password?: string }): Struct;


// === Automation/Browser/Capture ===

/**
 * Takes a screenshot of the current page
 * @param session — Automation session
 * @param fullPage (optional) — Capture entire scrollable page
 * @returns sessionOut — Automation session (pass-through)
 * @returns screenshot — Screenshot as base64 PNG data
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function browserScreenshot({ session: Struct, fullPage?: bool }): { sessionOut: Struct, screenshot: string, image: Struct };

/**
 * Takes a screenshot of a specific element
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element to screenshot
 * @returns sessionOut — Automation session (pass-through)
 * @returns screenshot — Screenshot as base64 PNG data
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function browserScreenshotElement({ session: Struct, selector?: string }): { sessionOut: Struct, screenshot: string, image: Struct };


// === Automation/Browser/Extract ===

/**
 * Executes JavaScript code in the browser and returns the result
 * @param session — Automation session
 * @param script (optional) — JavaScript code to execute (use 'return' to return a value)
 * @returns sessionOut — Automation session (pass-through)
 * @returns result — Return value from JavaScript (as JSON)
 * @impure has side effects / drives control flow
 */
declare function browserExecuteJs({ session: Struct, script?: string }): { sessionOut: Struct, result: any };

/**
 * Gets an attribute value of an element
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element
 * @param attribute (optional) — Name of attribute to get
 * @returns sessionOut — Automation session (pass-through)
 * @returns value — Attribute value (empty if not found)
 * @impure has side effects / drives control flow
 */
declare function browserGetAttribute({ session: Struct, selector?: string, attribute?: string }): { sessionOut: Struct, value: string };

/**
 * Gets the HTML content of an element or the entire page
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element (empty for entire page)
 * @param outerHtml (optional) — Include element's own tags (vs just inner content)
 * @returns sessionOut — Automation session (pass-through)
 * @returns html — HTML content
 * @impure has side effects / drives control flow
 */
declare function browserGetHtml({ session: Struct, selector?: string, outerHtml?: bool }): { sessionOut: Struct, html: string };

/**
 * Gets the text content of an element
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element
 * @returns sessionOut — Automation session (pass-through)
 * @returns text — Text content of the element
 * @impure has side effects / drives control flow
 */
declare function browserGetText({ session: Struct, selector?: string }): { sessionOut: Struct, text: string };


// === Automation/Browser/Files ===

/**
 * Sets the default download directory for the browser (must be called before downloads)
 * @param session — Automation session
 * @param downloadPath — Absolute path to the download directory
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserSetDownloadDir({ session: Struct, downloadPath: Struct }): Struct;

/**
 * Clicks an element to trigger a download
 * @param session — Automation session
 * @param selector — CSS selector for the download link/button
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserTriggerDownload({ session: Struct, selector: string }): Struct;

/**
 * Uploads a file to an input element using its selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector for the file input element
 * @param filePath — Absolute path to the file to upload
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserUploadFile({ session: Struct, selector?: string, filePath: string }): Struct;

/**
 * Uploads multiple files to a file input that accepts multiple
 * @param session — Automation session
 * @param selector (optional) — CSS selector for the file input element
 * @param filePaths — Array of absolute paths to the files to upload
 * @returns sessionOut — Automation session (pass-through)
 * @returns uploadedCount — Number of files uploaded
 * @impure has side effects / drives control flow
 */
declare function browserUploadMultipleFiles({ session: Struct, selector?: string, filePaths: any }): { sessionOut: Struct, uploadedCount: int };

/**
 * Waits for a file to appear in the download directory
 * @param session — Automation session
 * @param downloadDir — Directory to watch for downloads
 * @param filePattern (optional) — File name pattern to match (e.g., '*.pdf', leave empty for any)
 * @param timeoutMs (optional) — Maximum time to wait for download
 * @returns sessionOut — Automation session (pass-through)
 * @returns downloadedFile — Path to the downloaded file
 * @impure has side effects / drives control flow
 */
declare function browserWaitForDownload({ session: Struct, downloadDir: Struct, filePattern?: string, timeoutMs?: int }): { sessionOut: Struct, downloadedFile: Struct };


// === Automation/Browser/Input ===

/**
 * Presses a keyboard key (Enter, Tab, Escape, etc.)
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element (optional, press on active element if empty)
 * @param key (optional) — Key to press
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserPressKey({ session: Struct, selector?: string, key?: string }): Struct;

/**
 * Selects an option in a dropdown/select element
 * @param session — Automation session
 * @param selector (optional) — CSS selector of select element
 * @param value (optional) — Option value to select
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserSelectOption({ session: Struct, selector?: string, value?: string }): Struct;

/**
 * Types text into an element matching the selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector of input element
 * @param text (optional) — Text to type into the element
 * @param clearFirst (optional) — Clear existing text before typing
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserTypeText({ session: Struct, selector?: string, text?: string, clearFirst?: bool }): Struct;


// === Automation/Browser/Interact ===

/**
 * Clicks on an element matching the selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element to click
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserClick({ session: Struct, selector?: string }): Struct;

/**
 * Double-clicks on an element matching the selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element to double-click
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserDoubleClick({ session: Struct, selector?: string }): Struct;

/**
 * Hovers over an element matching the selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element to hover
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserHover({ session: Struct, selector?: string }): Struct;

/**
 * Scrolls element into the visible area
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element to scroll into view
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserScrollIntoView({ session: Struct, selector?: string }): Struct;


// === Automation/Browser/Navigation ===

/**
 * Navigates back in browser history
 * @param session — Automation session
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserBack({ session: Struct }): Struct;

/**
 * Navigates forward in browser history
 * @param session — Automation session
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserForward({ session: Struct }): Struct;

/**
 * Navigates the page to a URL
 * @param session — Automation session
 * @param url (optional) — URL to navigate to
 * @returns sessionOut — Automation session (pass-through)
 * @returns finalUrl — The actual URL after navigation (may differ due to redirects)
 * @impure has side effects / drives control flow
 */
declare function browserGoto({ session: Struct, url?: string }): { sessionOut: Struct, finalUrl: string };

/**
 * Reloads the current page
 * @param session — Automation session
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserReload({ session: Struct }): Struct;


// === Automation/Browser/Observe ===

/**
 * Clears the captured console log buffer
 * @param session — Automation session
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserClearConsoleLogs({ session: Struct }): Struct;

/**
 * Retrieves console messages from the browser (logs, warnings, errors)
 * @param session — Automation session
 * @param levelFilter (optional) — Filter by log level (empty for all)
 * @returns sessionOut — Automation session (pass-through)
 * @returns logs — Array of console messages
 * @returns count — Number of log entries
 * @returns hasErrors — Whether there are error-level logs
 * @impure has side effects / drives control flow
 */
declare function browserGetConsoleLogs({ session: Struct, levelFilter?: string }): { sessionOut: Struct, logs: Struct[], count: int, hasErrors: bool };

/**
 * Retrieves captured network requests from the observer
 * @param session — Automation session
 * @param clearAfter (optional) — Clear the request buffer after retrieval
 * @returns sessionOut — Automation session (pass-through)
 * @returns requests — Array of captured network requests
 * @returns count — Number of captured requests
 * @impure has side effects / drives control flow
 */
declare function browserGetNetworkRequests({ session: Struct, clearAfter?: bool }): { sessionOut: Struct, requests: Struct[], count: int };

/**
 * Starts observing network requests using the Performance API
 * @param session — Automation session
 * @param urlPattern (optional) — Filter requests by URL pattern (empty for all)
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserStartNetworkObserver({ session: Struct, urlPattern?: string }): Struct;

/**
 * Waits until no network requests are in progress for a specified duration
 * @param session — Automation session
 * @param idleTimeMs (optional) — How long network must be idle before continuing
 * @param timeoutMs (optional) — Maximum time to wait for network idle
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserWaitForNetworkIdle({ session: Struct, idleTimeMs?: int, timeoutMs?: int }): Struct;


// === Automation/Browser/Snapshot ===

/**
 * Captures the accessibility tree of the current page for screen reader analysis
 * @param session — Automation session
 * @param maxDepth (optional) — Maximum tree depth (-1 for unlimited)
 * @param includeHidden (optional) — Include hidden elements in the tree
 * @returns sessionOut — Automation session (pass-through)
 * @returns tree — Accessibility tree root node
 * @returns treeJson — Accessibility tree as JSON string for LLM processing
 * @impure has side effects / drives control flow
 */
declare function browserGetAccessibilitySnapshot({ session: Struct, maxDepth?: int, includeHidden?: bool }): { sessionOut: Struct, tree: Struct, treeJson: string };

/**
 * Captures the current DOM state including HTML, title, URL, and viewport info
 * @param session — Automation session
 * @param includeStyles (optional) — Include computed styles (increases snapshot size)
 * @returns sessionOut — Automation session (pass-through)
 * @returns snapshot — DOM snapshot data
 * @returns html — Page HTML content
 * @returns title — Page title
 * @returns url — Current page URL
 * @impure has side effects / drives control flow
 */
declare function browserGetDomSnapshot({ session: Struct, includeStyles?: bool }): { sessionOut: Struct, snapshot: Struct, html: string, title: string, url: string };

/**
 * Gets detailed information about a specific element by selector
 * @param session — Automation session
 * @param selector (optional) — CSS selector of element
 * @returns sessionOut — Automation session (pass-through)
 * @returns html — Element outer HTML
 * @returns text — Element text content
 * @returns tag — Element tag name
 * @returns x — Element X position
 * @returns y — Element Y position
 * @returns width — Element width
 * @returns height — Element height
 * @returns visible — Whether element is visible
 * @impure has side effects / drives control flow
 */
declare function browserGetElementSnapshot({ session: Struct, selector?: string }): { sessionOut: Struct, html: string, text: string, tag: string, x: int, y: int, width: int, height: int, visible: bool };


// === Automation/Browser/Storage ===

/**
 * Clears localStorage and/or sessionStorage
 * @param session — Automation session
 * @param clearLocal (optional) — Clear localStorage
 * @param clearSession (optional) — Clear sessionStorage
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserClearStorage({ session: Struct, clearLocal?: bool, clearSession?: bool }): Struct;

/**
 * Gets all key-value pairs from localStorage or sessionStorage
 * @param session — Automation session
 * @param storageType (optional) — Which storage to retrieve
 * @returns sessionOut — Automation session (pass-through)
 * @returns data — All storage data as JSON object
 * @returns count — Number of items in storage
 * @impure has side effects / drives control flow
 */
declare function browserGetAllStorage({ session: Struct, storageType?: string }): { sessionOut: Struct, data: Struct, count: int };

/**
 * Gets a value from browser localStorage
 * @param session — Automation session
 * @param key (optional) — Storage key to retrieve
 * @returns sessionOut — Automation session (pass-through)
 * @returns value — Storage value (null if not found)
 * @returns exists — Whether the key exists
 * @impure has side effects / drives control flow
 */
declare function browserGetLocalStorage({ session: Struct, key?: string }): { sessionOut: Struct, value: string, exists: bool };

/**
 * Gets a value from browser sessionStorage
 * @param session — Automation session
 * @param key (optional) — Storage key to retrieve
 * @returns sessionOut — Automation session (pass-through)
 * @returns value — Storage value (null if not found)
 * @returns exists — Whether the key exists
 * @impure has side effects / drives control flow
 */
declare function browserGetSessionStorage({ session: Struct, key?: string }): { sessionOut: Struct, value: string, exists: bool };

/**
 * Sets a value in browser localStorage
 * @param session — Automation session
 * @param key (optional) — Storage key
 * @param value (optional) — Value to store
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserSetLocalStorage({ session: Struct, key?: string, value?: string }): Struct;

/**
 * Sets a value in browser sessionStorage
 * @param session — Automation session
 * @param key (optional) — Storage key
 * @param value (optional) — Value to store
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserSetSessionStorage({ session: Struct, key?: string, value?: string }): Struct;


// === Automation/Browser/Wait ===

/**
 * Waits for a specified amount of time
 * @param session — Automation session
 * @param delayMs (optional) — Time to wait in milliseconds
 * @returns sessionOut — Automation session (pass-through)
 * @impure has side effects / drives control flow
 */
declare function browserWaitDelay({ session: Struct, delayMs?: int }): Struct;

/**
 * Waits for an element matching the selector to appear in the DOM
 * @param session — Automation session
 * @param selector (optional) — CSS selector to wait for
 * @param timeoutMs (optional) — Maximum time to wait
 * @returns sessionOut — Automation session (pass-through)
 * @returns found — Whether the element was found within timeout
 * @impure has side effects / drives control flow
 */
declare function browserWaitFor({ session: Struct, selector?: string, timeoutMs?: int }): { sessionOut: Struct, found: bool };


// === Automation/Computer/Accessibility ===

/**
 * Finds an element in the accessibility tree by role, name, or other attributes
 * @param session — Computer session handle
 * @param role (optional) — Accessibility role to match
 * @param name (optional) — Element name to match (partial match)
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns element — Found accessibility element
 * @returns x — Element center X coordinate
 * @returns y — Element center Y coordinate
 * @impure has side effects / drives control flow
 */
declare function computerFindAccessibilityElement({ session: Struct, role?: string, name?: string }): { sessionOut: Struct, element: Struct, x: int, y: int };

/**
 * Retrieves the accessibility tree for a window (requires platform-specific accessibility APIs)
 * @param session — Computer session handle
 * @param windowTitle (optional) — Title of the window to inspect (leave empty for focused window)
 * @param maxDepth (optional) — Maximum tree depth to traverse (-1 for unlimited)
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns tree — Accessibility tree root node
 * @returns treeJson — Accessibility tree as JSON string for LLM processing
 * @returns error — Error message if accessibility APIs are unavailable
 * @impure has side effects / drives control flow
 */
declare function computerGetAccessibilityTree({ session: Struct, windowTitle?: string, maxDepth?: int }): { sessionOut: Struct, tree: Struct, treeJson: string, error: string };


// === Automation/Computer/Capture ===

/**
 * Takes a screenshot of the screen, window, or region
 * @param session — Computer session handle
 * @param captureType (optional) — What to capture: full screen, specific display, or region
 * @param displayIndex (optional) — Index of display to capture (when capture_type=display)
 * @param regionX (optional) — X coordinate of region (when capture_type=region)
 * @param regionY (optional) — Y coordinate of region
 * @param regionWidth (optional) — Width of region
 * @param regionHeight (optional) — Height of region
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns screenshot — Reference to the captured screenshot
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function computerScreenshot({ session: Struct, captureType?: string, displayIndex?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int }): { sessionOut: Struct, screenshot: Struct, image: Struct };


// === Automation/Computer/Clipboard ===

/**
 * Gets an image from the system clipboard if available
 * @param session — Computer session handle
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns image — Image from clipboard as NodeImage
 * @returns hasImage — Whether the clipboard contains an image
 * @impure has side effects / drives control flow
 */
declare function computerClipboardGetImage({ session: Struct }): { sessionOut: Struct, image: Struct, hasImage: bool };

/**
 * Gets the current text content from the system clipboard
 * @param session — Computer session handle
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns text — Text content from clipboard
 * @returns hasText — Whether the clipboard contains text
 * @impure has side effects / drives control flow
 */
declare function computerClipboardGetText({ session: Struct }): { sessionOut: Struct, text: string, hasText: bool };

/**
 * Sets an image to the system clipboard
 * @param session — Computer session handle
 * @param image — Image to copy to clipboard (NodeImage)
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerClipboardSetImage({ session: Struct, image: Struct }): Struct;

/**
 * Sets text content to the system clipboard
 * @param session — Computer session handle
 * @param text — Text to copy to clipboard
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerClipboardSetText({ session: Struct, text: string }): Struct;


// === Automation/Computer/Display ===

/**
 * Gets information about a specific display by index
 * @param session — Computer session handle
 * @param index (optional) — Display index (0-based)
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns display — Display information
 * @returns width — Display width in pixels
 * @returns height — Display height in pixels
 * @impure has side effects / drives control flow
 */
declare function computerGetDisplay({ session: Struct, index?: int }): { sessionOut: Struct, display: Struct, width: int, height: int };

/**
 * Gets information about the primary display
 * @param session — Computer session handle
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns display — Primary display information
 * @returns width — Display width in pixels
 * @returns height — Display height in pixels
 * @impure has side effects / drives control flow
 */
declare function computerGetPrimaryDisplay({ session: Struct }): { sessionOut: Struct, display: Struct, width: int, height: int };

/**
 * Enumerates all connected monitors/displays
 * @param session — Computer session handle
 * @returns sessionOut — Computer session handle (pass-through)
 * @returns displays — List of connected displays
 * @returns count — Number of connected displays
 * @returns primaryIndex — Index of the primary display
 * @impure has side effects / drives control flow
 */
declare function computerListDisplays({ session: Struct }): { sessionOut: Struct, displays: Struct[], count: int, primaryIndex: int };


// === Automation/Computer/Keyboard ===

/**
 * Presses a keyboard key or key combination
 * @param session — Computer session handle
 * @param key (optional) — Key to press (e.g., 'a', 'Enter', 'Tab', 'Escape')
 * @param modifiers (optional) — Modifier keys to hold (comma-separated: ctrl,shift,alt,meta)
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerKeyPress({ session: Struct, key?: string, modifiers?: string }): Struct;

/**
 * Types text using the keyboard
 * @param session — Computer session handle
 * @param text (optional) — Text to type
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerKeyType({ session: Struct, text?: string }): Struct;


// === Automation/Computer/Mouse ===

/**
 * Clicks the mouse at the specified coordinates
 * @param session — Computer session handle
 * @param x (optional) — X coordinate (horizontal position)
 * @param y (optional) — Y coordinate (vertical position)
 * @param button (optional) — Mouse button to click
 * @param useTemplateMatching (optional) — If enabled, use template matching to find the click target from a recorded screenshot
 * @param template — Template image for template matching
 * @param confidence (optional) — Minimum confidence threshold for template matching (0.0-1.0)
 * @param naturalMove (optional) — Use curved, human-like mouse movement to avoid bot detection
 * @param moveDurationMs (optional) — Duration of natural mouse movement in milliseconds
 * @param useFingerprint (optional) — If enabled, use fingerprint bounding box as fallback before raw coordinates
 * @param fingerprint — Optional element fingerprint for pre-click validation
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerMouseClick({ session: Struct, x?: int, y?: int, button?: string, useTemplateMatching?: bool, template: Struct, confidence?: float, naturalMove?: bool, moveDurationMs?: int, useFingerprint?: bool, fingerprint: Struct }): Struct;

/**
 * Double-clicks the mouse at the specified coordinates
 * @param session — Computer session handle
 * @param x (optional) — X coordinate
 * @param y (optional) — Y coordinate
 * @param useTemplateMatching (optional) — If enabled, use template matching to find the click target from a recorded screenshot
 * @param template — Template image for template matching
 * @param confidence (optional) — Minimum confidence threshold for template matching (0.0-1.0)
 * @param naturalMove (optional) — Use curved, human-like mouse movement to avoid bot detection
 * @param moveDurationMs (optional) — Duration of natural mouse movement in milliseconds
 * @param useFingerprint (optional) — If enabled, use fingerprint bounding box as fallback before raw coordinates
 * @param fingerprint — Optional element fingerprint for pre-click validation
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerMouseDoubleClick({ session: Struct, x?: int, y?: int, useTemplateMatching?: bool, template: Struct, confidence?: float, naturalMove?: bool, moveDurationMs?: int, useFingerprint?: bool, fingerprint: Struct }): Struct;

/**
 * Drags the mouse from one position to another
 * @param session — Computer session handle
 * @param fromX (optional) — Starting X coordinate
 * @param fromY (optional) — Starting Y coordinate
 * @param toX (optional) — Ending X coordinate
 * @param toY (optional) — Ending Y coordinate
 * @param button (optional) — Mouse button to use for dragging
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerMouseDrag({ session: Struct, fromX?: int, fromY?: int, toX?: int, toY?: int, button?: string }): Struct;

/**
 * Moves the mouse cursor to the specified screen coordinates
 * @param session — Computer session handle
 * @param x (optional) — X coordinate (horizontal position)
 * @param y (optional) — Y coordinate (vertical position)
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerMouseMove({ session: Struct, x?: int, y?: int }): Struct;

/**
 * Moves the mouse cursor naturally using curved paths with variable speed to avoid bot detection
 * @param session — Computer session handle
 * @param x (optional) — Target X coordinate
 * @param y (optional) — Target Y coordinate
 * @param durationMs (optional) — Approximate duration of the movement in milliseconds
 * @param curveIntensity (optional) — How curved the path is (0.0 = straight, 1.0 = very curved)
 * @param overshoot (optional) — Whether to slightly overshoot and correct (more human-like)
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerNaturalMouseMove({ session: Struct, x?: int, y?: int, durationMs?: int, curveIntensity?: float, overshoot?: bool }): Struct;

/**
 * Scrolls the mouse wheel
 * @param session — Computer session handle
 * @param dx (optional) — Horizontal scroll amount (positive = right)
 * @param dy (optional) — Vertical scroll amount (positive = down)
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerScroll({ session: Struct, dx?: int, dy?: int }): Struct;


// === Automation/Computer/Wait ===

/**
 * Waits for the specified number of milliseconds
 * @param session — Computer session handle
 * @param ms (optional) — Time to wait in milliseconds
 * @returns sessionOut — Computer session handle (pass-through)
 * @impure has side effects / drives control flow
 */
declare function computerWait({ session: Struct, ms?: int }): Struct;


// === Automation/Computer/Window ===

/**
 * Captures a screenshot of a specific window
 * @param session — Computer session handle
 * @param windowId — ID of the window to capture
 * @returns screenshot — Base64-encoded PNG image
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function computerCaptureWindow({ session: Struct, windowId: string }): { screenshot: string, image: Struct };

/**
 * Finds a window by its title (partial match supported)
 * @param session — Computer session handle
 * @param title — Window title to search for (partial match)
 * @param exactMatch (optional) — Require exact title match
 * @returns window — Found window information
 * @impure has side effects / drives control flow
 */
declare function computerFindWindowByTitle({ session: Struct, title: string, exactMatch?: bool }): Struct;

/**
 * Brings a window to the front and gives it focus
 * @param session — Computer session handle
 * @param windowTitle — Title or app name to search for (partial match on both title and app name)
 * @param exactMatch (optional) — Require exact title match
 * @param launchIfNotFound (optional) — Try to launch the application if no window is found
 * @returns window — Focused window information
 * @impure has side effects / drives control flow
 */
declare function computerFocusWindow({ session: Struct, windowTitle: string, exactMatch?: bool, launchIfNotFound?: bool }): Struct;

/**
 * Gets information about the currently focused window
 * @param session — Computer session handle
 * @returns window — Active window information
 * @returns title — Window title
 * @impure has side effects / drives control flow
 */
declare function computerGetActiveWindow({ session: Struct }): { window: Struct, title: string };

/**
 * Launches an application by path or name
 * @param session — Computer session handle
 * @param path — Application path or command
 * @param args (optional) — Command line arguments (space-separated)
 * @param waitMs (optional) — Time to wait after launching (ms)
 * @returns pid — Process ID if available
 * @impure has side effects / drives control flow
 */
declare function computerLaunchApp({ session: Struct, path: string, args?: string, waitMs?: int }): int;

/**
 * Lists all visible windows on the desktop
 * @param session — Computer session handle
 * @returns windows — List of window information
 * @returns count — Number of windows
 * @impure has side effects / drives control flow
 */
declare function computerListWindows({ session: Struct }): { windows: any, count: int };


// === Automation/Fingerprint ===

/**
 * Compares two fingerprints and calculates similarity
 * @param fingerprintA — First fingerprint
 * @param fingerprintB — Second fingerprint
 * @returns similarity — Similarity score (0.0-1.0)
 * @returns isMatch — Whether fingerprints likely match the same element
 * @impure has side effects / drives control flow
 */
declare function fingerprintCompare({ fingerprintA: Struct, fingerprintB: Struct }): { similarity: float, isMatch: bool };

/**
 * Computes a hash for fingerprint comparison
 * @param fingerprint — Fingerprint to hash
 * @returns hash — Computed hash string
 * @impure has side effects / drives control flow
 */
declare function fingerprintComputeHash({ fingerprint: Struct }): string;

/**
 * Creates a new element fingerprint for identification
 * @param id (optional) — Unique identifier for the fingerprint
 * @param selectors (optional) — Selector set for element location
 * @param role (optional) — ARIA role of the element
 * @param name (optional) — Accessible name of the element
 * @param text (optional) — Visible text content
 * @param boundingBox (optional) — Bounding box of the element (x1, y1, x2, y2)
 * @returns fingerprint — Created element fingerprint
 * @impure has side effects / drives control flow
 */
declare function fingerprintCreate({ id?: string, selectors?: Struct, role?: string, name?: string, text?: string, boundingBox?: Struct }): Struct;

/**
 * Extracts individual fields from a fingerprint
 * @param fingerprint — Fingerprint to extract from
 * @returns id — Fingerprint ID
 * @returns role — Element role
 * @returns name — Element name
 * @returns text — Element text
 * @returns tagName — HTML tag name
 * @returns selectorCount — Number of selectors
 * @returns matchCount — Times fingerprint was matched
 * @impure has side effects / drives control flow
 */
declare function fingerprintExtractData({ fingerprint: Struct }): { id: string, role: string, name: string, text: string, tagName: string, selectorCount: int, matchCount: int };

/**
 * Parses an element fingerprint from JSON
 * @param json (optional) — JSON string containing fingerprint data
 * @returns fingerprint — Parsed element fingerprint
 * @returns errorMessage — Error message if parsing failed
 * @impure has side effects / drives control flow
 */
declare function fingerprintFromJson({ json?: string }): { fingerprint: Struct, errorMessage: string };

/**
 * Attempts to find an element matching the fingerprint
 * @param session — Automation session
 * @param fingerprint — Fingerprint to match
 * @param strategy (optional) — Matching strategy
 * @param timeoutMs (optional) — Maximum time to search
 * @returns found — Whether element was found
 * @returns selectorUsed — The selector that matched
 * @returns confidence — Match confidence
 * @impure has side effects / drives control flow
 */
declare function fingerprintMatch({ session: Struct, fingerprint: Struct, strategy?: string, timeoutMs?: int }): { found: bool, selectorUsed: string, confidence: float };

/**
 * Creates fingerprint matching options
 * @param strategy (optional) — Matching strategy to use
 * @param minConfidence (optional) — Minimum confidence threshold (0.0-1.0)
 * @param maxFallbackAttempts (optional) — Maximum number of fallback attempts
 * @param timeoutMs (optional) — Maximum time to search
 * @returns options — Fingerprint match options
 * @impure has side effects / drives control flow
 */
declare function fingerprintMatchOptions({ strategy?: string, minConfidence?: float, maxFallbackAttempts?: int, timeoutMs?: int }): Struct;

/**
 * Records that a fingerprint was successfully matched
 * @param fingerprint — Fingerprint that was matched
 * @returns updatedFingerprint — Fingerprint with updated match stats
 * @returns matchCount — Total times this fingerprint has matched
 * @impure has side effects / drives control flow
 */
declare function fingerprintRecordMatch({ fingerprint: Struct }): { updatedFingerprint: Struct, matchCount: int };

/**
 * Serializes an element fingerprint to JSON
 * @param fingerprint — Fingerprint to serialize
 * @param pretty (optional) — Use pretty formatting
 * @returns json — JSON string
 * @impure has side effects / drives control flow
 */
declare function fingerprintToJson({ fingerprint: Struct, pretty?: bool }): string;

/**
 * Updates an existing fingerprint with new data
 * @param fingerprint — Fingerprint to update
 * @param selectors — New selector set (optional)
 * @param role (optional) — New role (empty to keep existing)
 * @param name (optional) — New name (empty to keep existing)
 * @param text (optional) — New text (empty to keep existing)
 * @returns updatedFingerprint — Updated element fingerprint
 * @impure has side effects / drives control flow
 */
declare function fingerprintUpdate({ fingerprint: Struct, selectors: Struct, role?: string, name?: string, text?: string }): Struct;


// === Automation/LLM/Healing ===

/**
 * Uses LLM to diagnose automation failures and suggest/apply healing actions
 * @param model — LLM model (vision-capable preferred)
 * @param screenshot (optional) — Base64-encoded screenshot at time of failure
 * @param errorMessage — The error message from the failed action
 * @param actionType — Type of action that failed (click, type, wait, find, etc.)
 * @param actionTarget — The target of the failed action (selector, coordinates, text)
 * @param context (optional) — Additional context about what the automation was trying to do
 * @param pageHtml (optional) — Current page HTML (for selector-based failures)
 * @returns result — Full healing result
 * @returns diagnosis — Failure diagnosis
 * @returns newValue — Healed value (new selector, coordinates, etc.)
 * @impure has side effects / drives control flow
 */
declare function llmDiagnoseAndHeal({ model: Struct, screenshot?: string, errorMessage: string, actionType: string, actionTarget: string, context?: string, pageHtml?: string }): { result: Struct, diagnosis: Struct, newValue: string };

/**
 * Uses LLM to fix a broken CSS/XPath selector based on page context
 * @param model — LLM model (vision-capable preferred)
 * @param screenshot (optional) — Base64-encoded screenshot (optional but recommended)
 * @param pageHtml — Current page HTML or DOM structure
 * @param brokenSelector — The selector that no longer works
 * @param elementDescription — Description of what the selector should match
 * @param selectorType (optional) — Type of selector: css, xpath, or accessibility
 * @returns result — Healed selector result
 * @returns newSelector — The healed selector string
 * @impure has side effects / drives control flow
 */
declare function llmHealSelector({ model: Struct, screenshot?: string, pageHtml: string, brokenSelector: string, elementDescription: string, selectorType?: string }): { result: Struct, newSelector: string };

/**
 * Uses vision LLM to find a visually similar element when template matching fails
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded current screenshot
 * @param template — Base64-encoded template image that failed to match
 * @param elementDescription — Description of what the template represents
 * @param lastKnownPosition (optional) — Where the element was previously found (x,y)
 * @returns result — Healed template result
 * @returns x — X coordinate of found element
 * @returns y — Y coordinate of found element
 * @impure has side effects / drives control flow
 */
declare function llmHealTemplate({ model: Struct, screenshot: string, template: string, elementDescription: string, lastKnownPosition?: string }): { result: Struct, x: int, y: int };


// === Automation/LLM/Planning ===

/**
 * Uses LLM to plan a sequence of automation actions to achieve a goal
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded current screenshot
 * @param goal — What the automation should accomplish
 * @param availableActions (optional) — JSON array of available action types and their parameters
 * @param constraints (optional) — Any constraints or preferences for the plan
 * @returns plan — Complete action plan
 * @returns actions — List of planned actions
 * @returns firstAction — The first action to execute
 * @impure has side effects / drives control flow
 */
declare function llmPlanActions({ model: Struct, screenshot: string, goal: string, availableActions?: string, constraints?: string }): { plan: Struct, actions: any, firstAction: Struct };

/**
 * Uses LLM to suggest the most appropriate next action given current screen and goal
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded current screenshot
 * @param goal — Ultimate goal we're trying to achieve
 * @param completedActions (optional) — JSON array of actions already taken
 * @param lastResult (optional) — Result/outcome of the last action
 * @returns suggestion — Next step suggestion
 * @returns actionType — Type of suggested action
 * @returns target — Target description
 * @impure has side effects / drives control flow
 */
declare function llmSuggestNextStep({ model: Struct, screenshot: string, goal: string, completedActions?: string, lastResult?: string }): { suggestion: Struct, actionType: string, target: string };


// === Automation/LLM/Vision ===

/**
 * Uses vision LLM to classify screen state and identify visible elements
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param expectedStates (optional) — Comma-separated list of possible states to classify into
 * @returns classification — Screen classification result
 * @returns screenType — Detected screen type
 * @returns state — Current screen state
 * @impure has side effects / drives control flow
 */
declare function llmClassifyScreen({ model: Struct, screenshot: string, expectedStates?: string }): { classification: Struct, screenType: string, state: string };

/**
 * Uses vision LLM to describe a specific UI element at given coordinates
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param x — X coordinate of element
 * @param y — Y coordinate of element
 * @returns element — Element description
 * @returns description — Text description
 * @impure has side effects / drives control flow
 */
declare function llmDescribeElement({ model: Struct, screenshot: string, x: int, y: int }): { element: Struct, description: string };

/**
 * Uses vision LLM to extract structured data from a screenshot
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param schema — JSON Schema describing what to extract (or example JSON)
 * @param hint (optional) — Optional extraction hint
 * @returns data — Extracted structured data
 * @impure has side effects / drives control flow
 */
declare function llmExtractFromScreen({ model: Struct, screenshot: string, schema: string, hint?: string }): any;

/**
 * Uses a vision LLM to locate UI elements based on natural language description
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot of the screen
 * @param description — Natural language description of the element to find (e.g., 'the blue submit button')
 * @param context (optional) — Optional context about the application or page
 * @returns location — Element location details
 * @impure has side effects / drives control flow
 */
declare function llmFindElement({ model: Struct, screenshot: string, description: string, context?: string }): Struct;

/**
 * Uses vision LLM to comprehensively observe and describe the current screen
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param focusArea (optional) — Specific area or aspect to focus on (optional)
 * @returns observation — Complete screen observation
 * @returns description — Text description of the screen
 * @returns elements — List of observed elements
 * @impure has side effects / drives control flow
 */
declare function llmObserveScreen({ model: Struct, screenshot: string, focusArea?: string }): { observation: Struct, description: string, elements: any };

/**
 * Uses LLM to rank multiple element candidates based on match quality
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param candidates — Array of candidate elements to rank
 * @param criteria — What the target element should match (description/intent)
 * @param context (optional) — Additional context for ranking
 * @returns result — Full ranking result
 * @returns bestMatch — ID of the best matching candidate
 * @returns ranked — Candidates sorted by rank
 * @impure has side effects / drives control flow
 */
declare function llmRankCandidates({ model: Struct, screenshot: string, candidates: any, criteria: string, context?: string }): { result: Struct, bestMatch: string, ranked: any };

/**
 * Uses LLM to disambiguate between multiple element candidates
 * @param model — Vision-capable LLM model
 * @param screenshot — Base64-encoded screenshot
 * @param candidates — Array of element candidates to choose from
 * @param intent — What the user is trying to accomplish
 * @returns result — Resolution result
 * @impure has side effects / drives control flow
 */
declare function llmResolveElement({ model: Struct, screenshot: string, candidates: any, intent: string }): Struct;


// === Automation/RPA ===

/**
 * Asserts that a specific color exists at a position
 * @param session — RPA session handle
 * @param x (optional) — X position
 * @param y (optional) — Y position
 * @param red (optional) — Expected red (0-255)
 * @param green (optional) — Expected green (0-255)
 * @param blue (optional) — Expected blue (0-255)
 * @param tolerance (optional) — Color tolerance (0-255)
 * @returns passed — Whether assertion passed
 * @impure has side effects / drives control flow
 */
declare function rpaAssertColor({ session: Struct, x?: int, y?: int, red?: int, green?: int, blue?: int, tolerance?: int }): bool;

/**
 * Asserts that a template image exists on screen
 * @param session — RPA session handle
 * @param templatePath (optional) — Path to the template image
 * @param confidence (optional) — Minimum match confidence
 * @returns passed — Whether assertion passed
 * @impure has side effects / drives control flow
 */
declare function rpaAssertTemplateExists({ session: Struct, templatePath?: string, confidence?: float }): bool;

/**
 * Calculates elapsed time from a start timestamp
 * @param startTime (optional) — Start timestamp (ms since epoch) from Start Timer node
 * @returns elapsedMs — Time elapsed in milliseconds
 * @returns elapsedSec — Time elapsed in seconds
 * @impure has side effects / drives control flow
 */
declare function rpaCalculateElapsed({ startTime?: int }): { elapsedMs: int, elapsedSec: float };

/**
 * Performs a click at a specific screen position
 * @param session — Automation session
 * @param x (optional) — X coordinate
 * @param y (optional) — Y coordinate
 * @param clickType (optional) — Type of click to perform
 * @impure has side effects / drives control flow
 */
declare function rpaClickAtPosition({ session: Struct, x?: int, y?: int, clickType?: string }): void;

/**
 * Pauses execution for a specified duration
 * @param durationMs (optional) — Delay duration in milliseconds
 * @impure has side effects / drives control flow
 */
declare function rpaDelay({ durationMs?: int }): void;

/**
 * Captures diagnostic info when an automation fails
 * @param session — RPA session handle
 * @param errorMessage (optional) — The error that occurred
 * @param screenshotPath (optional) — Path to save diagnostic screenshot
 * @returns diagnosticInfo — JSON string with diagnostic data
 * @impure has side effects / drives control flow
 */
declare function rpaDiagnoseFailure({ session: Struct, errorMessage?: string, screenshotPath?: string }): string;

/**
 * Performs a drag and drop operation
 * @param session — Automation session
 * @param fromX (optional) — Start X coordinate
 * @param fromY (optional) — Start Y coordinate
 * @param toX (optional) — End X coordinate
 * @param toY (optional) — End Y coordinate
 * @param durationSec (optional) — Duration of drag in seconds
 * @impure has side effects / drives control flow
 */
declare function rpaDragDrop({ session: Struct, fromX?: int, fromY?: int, toX?: int, toY?: int, durationSec?: float }): void;

/**
 * Defines recovery actions for specific error types
 * @param errorType (optional) — Type of error to handle
 * @param actualError (optional) — The actual error message to check
 * @impure has side effects / drives control flow
 */
declare function rpaErrorRecovery({ errorType?: string, actualError?: string }): void;

/**
 * Finds a pixel on screen matching a specific color
 * @param session — RPA session handle
 * @param red (optional) — Red component (0-255)
 * @param green (optional) — Green component (0-255)
 * @param blue (optional) — Blue component (0-255)
 * @param tolerance (optional) — Color matching tolerance (0-255)
 * @returns x — X coordinate
 * @returns y — Y coordinate
 * @impure has side effects / drives control flow
 */
declare function rpaLocateColor({ session: Struct, red?: int, green?: int, blue?: int, tolerance?: int }): { x: int, y: int };

/**
 * Finds an element on screen using template matching
 * @param session — RPA session handle
 * @param templatePath (optional) — Path to the template image
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @returns x — X coordinate
 * @returns y — Y coordinate
 * @impure has side effects / drives control flow
 */
declare function rpaLocateTemplate({ session: Struct, templatePath?: string, confidence?: float }): { x: int, y: int };

/**
 * Logs an automation action for debugging and auditing
 * @param action (optional) — Action being performed
 * @param details (optional) — Additional details about the action
 * @param level (optional) — Log level
 * @returns logEntry — Formatted log entry
 * @impure has side effects / drives control flow
 */
declare function rpaLogAction({ action?: string, details?: string, level?: string }): string;

/**
 * Parses checkpoint data from a saved JSON string
 * @param checkpointData (optional) — Checkpoint JSON string to parse
 * @returns data — Extracted data from checkpoint
 * @returns name — Checkpoint name
 * @returns timestamp — When checkpoint was saved
 * @impure has side effects / drives control flow
 */
declare function rpaParseCheckpoint({ checkpointData?: string }): { data: string, name: string, timestamp: string };

/**
 * Retries an action multiple times with configurable backoff. WARNING: This node activates exec_attempt in a loop but the current executor does not re-enter downstream nodes -- the retry semantics require executor-level loop support to work correctly.
 * @param maxRetries (optional) — Maximum number of retry attempts
 * @param initialDelayMs (optional) — Initial delay before first retry
 * @param backoffType (optional) — Type of backoff strategy
 * @param shouldRetry (optional) — Whether to retry (connect to condition check)
 * @returns attempt — Current attempt number
 * @returns totalAttempts — Total attempts made
 * @impure has side effects / drives control flow
 */
declare function rpaRetryLoop({ maxRetries?: int, initialDelayMs?: int, backoffType?: string, shouldRetry?: bool }): { attempt: int, totalAttempts: int };

/**
 * Creates checkpoint data for potential recovery
 * @param checkpointName (optional) — Name to identify this checkpoint
 * @param data (optional) — JSON string of data to save at checkpoint
 * @returns checkpointData — Complete checkpoint data as JSON
 * @returns checkpointId — Unique ID for this checkpoint
 * @impure has side effects / drives control flow
 */
declare function rpaSaveCheckpoint({ checkpointName?: string, data?: string }): { checkpointData: string, checkpointId: string };

/**
 * Performs a scroll action at the current mouse position
 * @param session — Automation session
 * @param clicks (optional) — Number of scroll clicks (positive = up, negative = down)
 * @impure has side effects / drives control flow
 */
declare function rpaScroll({ session: Struct, clicks?: int }): void;

/**
 * Returns the current timestamp for measuring action duration
 * @returns startTime — Timestamp when timer started (ms since epoch)
 * @impure has side effects / drives control flow
 */
declare function rpaStartTimer(): int;

/**
 * Captures a screen snapshot and saves to file
 * @param session — Automation session
 * @param filePath — Path to save the snapshot image
 * @param monitor (optional) — Monitor index (0 = primary)
 * @returns success — Whether the snapshot was saved
 * @impure has side effects / drives control flow
 */
declare function rpaTakeSnapshot({ session: Struct, filePath: Struct, monitor?: int }): bool;

/**
 * Catches errors from automation actions. WARNING: This node reads error_occurred as a plain boolean input -- it does not actually intercept panics or Result::Err from downstream nodes. True try/catch semantics require executor-level support.
 * @param errorOccurred (optional) — Whether an error occurred (wire from action)
 * @param errorMessage (optional) — Error message if any (wire from action)
 * @returns message — Error message
 * @impure has side effects / drives control flow
 */
declare function rpaTryCatch({ errorOccurred?: bool, errorMessage?: string }): string;

/**
 * Types text using keyboard simulation
 * @param session — Automation session
 * @param text (optional) — Text to type
 * @param intervalMs (optional) — Delay between keystrokes
 * @impure has side effects / drives control flow
 */
declare function rpaTypeText({ session: Struct, text?: string, intervalMs?: int }): void;

/**
 * Waits for a specific color to appear at a position
 * @param session — Automation session
 * @param x (optional) — X position to check
 * @param y (optional) — Y position to check
 * @param red (optional) — Expected red (0-255)
 * @param green (optional) — Expected green (0-255)
 * @param blue (optional) — Expected blue (0-255)
 * @param tolerance (optional) — Color tolerance (0-255)
 * @param timeoutMs (optional) — Maximum wait time
 * @impure has side effects / drives control flow
 */
declare function rpaWaitForColor({ session: Struct, x?: int, y?: int, red?: int, green?: int, blue?: int, tolerance?: int, timeoutMs?: int }): void;

/**
 * Waits for a template to appear on screen
 * @param session — Automation session
 * @param templatePath (optional) — Path to the template image
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param timeoutMs (optional) — Maximum wait time in milliseconds
 * @param pollIntervalMs (optional) — Check interval in milliseconds
 * @returns x — X coordinate
 * @returns y — Y coordinate
 * @impure has side effects / drives control flow
 */
declare function rpaWaitForTemplate({ session: Struct, templatePath?: string, confidence?: float, timeoutMs?: int, pollIntervalMs?: int }): { x: int, y: int };

/**
 * Executes an action with a timeout constraint
 * @param timeoutMs (optional) — Maximum time to wait for action
 * @param completed (optional) — Whether the action completed (wire from action result)
 * @returns elapsedMs — Time elapsed
 * @impure has side effects / drives control flow
 */
declare function rpaWithTimeout({ timeoutMs?: int, completed?: bool }): int;


// === Automation/Selector ===

/**
 * Adds a selector to an existing selector set
 * @param selectorSet — Existing selector set
 * @param selector — Selector to add
 * @returns updatedSet — Selector set with new selector added
 * @impure has side effects / drives control flow
 */
declare function selectorAddToSet({ selectorSet: Struct, selector: Struct }): Struct;

/**
 * Creates a selector from a value and kind
 * @param kind (optional) — Type of selector
 * @param value (optional) — Selector value (CSS selector, XPath, text, etc.)
 * @param confidence (optional) — Confidence level (0.0-1.0)
 * @param scope (optional) — Optional scope selector to narrow search
 * @returns selector — The built selector
 * @impure has side effects / drives control flow
 */
declare function selectorBuild({ kind?: string, value?: string, confidence?: float, scope?: string }): Struct;

/**
 * Creates a new empty selector set
 * @returns selectorSet — Empty selector set
 * @impure has side effects / drives control flow
 */
declare function selectorCreateSet(): Struct;

/**
 * Gets the highest-ranked selector from a ranked set
 * @param rankedSet — Ranked selector set
 * @returns selector — Best selector
 * @returns score — Selector score
 * @impure has side effects / drives control flow
 */
declare function selectorGetBest({ rankedSet: Struct }): { selector: Struct, score: float };

/**
 * Gets the primary (first) selector from a selector set
 * @param selectorSet — Selector set to get primary from
 * @returns selector — Primary selector
 * @impure has side effects / drives control flow
 */
declare function selectorGetPrimary({ selectorSet: Struct }): Struct;

/**
 * Ranks selectors in a set by their confidence and specificity
 * @param selectorSet — Selector set to rank
 * @returns rankedSet — Ranked selector set
 * @impure has side effects / drives control flow
 */
declare function selectorRank({ selectorSet: Struct }): Struct;

/**
 * Converts a ranked selector set back to a regular selector set
 * @param rankedSet — Ranked selector set to convert
 * @returns selectorSet — Regular selector set with ranked order
 * @impure has side effects / drives control flow
 */
declare function selectorRankedToSet({ rankedSet: Struct }): Struct;

/**
 * Converts a selector to its string representation
 * @param selector — Selector to convert
 * @returns kind — Selector kind
 * @returns value — Selector value
 * @returns confidence — Selector confidence
 * @impure has side effects / drives control flow
 */
declare function selectorToString({ selector: Struct }): { kind: string, value: string, confidence: float };

/**
 * Validates a selector's format and structure
 * @param selector — Selector to validate
 * @returns isValid — Whether selector is valid
 * @returns error — Validation error message
 * @impure has side effects / drives control flow
 */
declare function selectorValidate({ selector: Struct }): { isValid: bool, error: string };


// === Automation/Vision ===

/**
 * Finds a template image on screen and clicks on it
 * @param session — Automation session handle (provides template matching via rustautogui)
 * @param template — Path to the template image file (FlowPath with caching support)
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param clickType (optional) — Type of click to perform
 * @param offsetX (optional) — X offset from center of matched template
 * @param offsetY (optional) — Y offset from center of matched template
 * @param fallbackX (optional) — X coordinate to click if template not found (use -1 to disable fallback)
 * @param fallbackY (optional) — Y coordinate to click if template not found (use -1 to disable fallback)
 * @returns found — Whether the template was found and clicked
 * @returns x — X coordinate where clicked
 * @returns y — Y coordinate where clicked
 * @impure has side effects / drives control flow
 */
declare function visionClickTemplate({ session: Struct, template: Struct, confidence?: float, clickType?: string, offsetX?: int, offsetY?: int, fallbackX?: int, fallbackY?: int }): { found: bool, x: int, y: int };

/**
 * Searches the screen for all occurrences of a template image
 * @param session — Automation session handle for screen operations
 * @param template — Template image file
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param maxResults (optional) — Maximum number of matches to return
 * @returns count — Number of matches found
 * @returns results — Array of match results (as JSON)
 * @impure has side effects / drives control flow
 */
declare function visionFindAllTemplates({ session: Struct, template: Struct, confidence?: float, maxResults?: int }): { count: int, results: any };

/**
 * Searches the screen for a template image and returns its location
 * @param session — Automation session handle for screen operations
 * @param template — Template image file
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param matchMode (optional) — Algorithm for template matching
 * @returns found — Whether the template was found
 * @returns result — Match result with location and confidence
 * @returns x — X coordinate of match center
 * @returns y — Y coordinate of match center
 * @impure has side effects / drives control flow
 */
declare function visionFindTemplate({ session: Struct, template: Struct, confidence?: float, matchMode?: string }): { found: bool, result: Struct, x: int, y: int };

/**
 * Gets the color of a pixel at a screen position
 * @param session — Automation session handle
 * @param x (optional) — X position
 * @param y (optional) — Y position
 * @returns red — Red component (0-255)
 * @returns green — Green component (0-255)
 * @returns blue — Blue component (0-255)
 * @returns hex — Hex color code (#RRGGBB)
 * @impure has side effects / drives control flow
 */
declare function visionGetPixelColor({ session: Struct, x?: int, y?: int }): { red: int, green: int, blue: int, hex: string };

/**
 * Gets the dimensions of a monitor
 * @param session — Automation session handle
 * @param monitor (optional) — Monitor index (0 = primary)
 * @returns width — Screen width
 * @returns height — Screen height
 * @impure has side effects / drives control flow
 */
declare function visionGetScreenSize({ session: Struct, monitor?: int }): { width: int, height: int };

/**
 * Captures a region of the screen and saves it
 * @param session — Automation session handle
 * @param x (optional) — Left position
 * @param y (optional) — Top position
 * @param width (optional) — Region width
 * @param height (optional) — Region height
 * @param filePath — Path to save the screenshot
 * @returns success — Whether the screenshot was saved
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function visionScreenshotRegion({ session: Struct, x?: int, y?: int, width?: int, height?: int, filePath: Struct }): { success: bool, image: Struct };

/**
 * Captures a screenshot and saves it to a file
 * @param session — Automation session handle
 * @param filePath — Path to save the screenshot
 * @param monitor (optional) — Monitor index (0 = primary)
 * @returns success — Whether the screenshot was saved
 * @returns image — Screenshot as NodeImage
 * @impure has side effects / drives control flow
 */
declare function visionScreenshotToFile({ session: Struct, filePath: Struct, monitor?: int }): { success: bool, image: Struct };

/**
 * Waits for a template image to appear on screen
 * @param session — Automation session handle
 * @param template — Template image file
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param timeoutMs (optional) — Maximum time to wait
 * @param pollIntervalMs (optional) — How often to check for template
 * @returns found — Whether the template was found
 * @returns result — Match result with location
 * @impure has side effects / drives control flow
 */
declare function visionWaitTemplate({ session: Struct, template: Struct, confidence?: float, timeoutMs?: int, pollIntervalMs?: int }): { found: bool, result: Struct };

/**
 * Waits for a template image to disappear from screen
 * @param session — Automation session handle
 * @param template — Template image file
 * @param confidence (optional) — Minimum match confidence (0.0-1.0)
 * @param timeoutMs (optional) — Maximum time to wait
 * @returns disappeared — Whether the template disappeared
 * @impure has side effects / drives control flow
 */
declare function visionWaitTemplateDisappear({ session: Struct, template: Struct, confidence?: float, timeoutMs?: int }): bool;

