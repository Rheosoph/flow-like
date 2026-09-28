// Automation — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace automation {
    // === Automation ===

    /**
     * Checks whether a native automation capability is granted, unavailable, or unsupported
     * @node automation_check_capability @alias automationCheckCapability
     * @param capability (optional) — Native capability to inspect
     * @returns status — Capability state and recovery information
     * @returns available — Whether access is currently available
     * @impure has side effects / drives control flow
     */
    function checkCapability({ capability?: string }): { status: Struct, available: bool };

    /**
     * Requests operating-system access and returns the verified capability state
     * @node automation_request_capability @alias automationRequestCapability
     * @param capability (optional) — Native capability to inspect
     * @returns status — Capability state and recovery information
     * @returns available — Whether access is currently available
     * @impure has side effects / drives control flow
     */
    function requestCapability({ capability?: string }): { status: Struct, available: bool };

    /**
     * Starts a unified automation session for desktop, browser, and RPA automation
     * @node automation_start_session @alias automationStartSession
     * @param defaultDelayMs (optional) — Default delay between actions in milliseconds
     * @param clickDelayMs (optional) — Delay between mouse move and click to ensure registration
     * @param debugMode (optional) — Enable debug mode for verbose logging
     * @returns session — Unified automation session for all operations
     * @impure has side effects / drives control flow
     */
    function startSession({ defaultDelayMs?: int, clickDelayMs?: int, debugMode?: bool }): Struct;

    /**
     * Stops an automation session and releases all resources
     * @node automation_stop_session @alias automationStopSession
     * @param session — Automation session to stop
     * @impure has side effects / drives control flow
     */
    function stopSession({ session: Struct }): void;

    namespace fingerprint {
        // === Automation/Fingerprint ===

        /**
         * Compares two fingerprints and calculates similarity
         * @node fingerprint_compare @receiver fingerprint_a @alias fingerprintCompare
         * @param fingerprintA — First fingerprint (receiver: `this` in `x.compare(...)`)
         * @param fingerprintB — Second fingerprint
         * @returns similarity — Similarity score (0.0-1.0)
         * @returns isMatch — Whether fingerprints likely match the same element
         * @impure has side effects / drives control flow
         */
        function compare(this: ElementFingerprint, { fingerprintA: Struct, fingerprintB: Struct }): { similarity: float, isMatch: bool };

        /**
         * Computes a hash for fingerprint comparison
         * @node fingerprint_compute_hash @receiver fingerprint @alias fingerprintComputeHash
         * @param fingerprint — Fingerprint to hash (receiver: `this` in `x.computeHash(...)`)
         * @returns hash — Computed hash string
         * @impure has side effects / drives control flow
         */
        function computeHash(this: ElementFingerprint, { fingerprint: Struct }): string;

        /**
         * Creates a new element fingerprint for identification
         * @node fingerprint_create @alias fingerprintCreate
         * @param id (optional) — Unique identifier for the fingerprint
         * @param selectors (optional) — Selector set for element location
         * @param role (optional) — ARIA role of the element
         * @param name (optional) — Accessible name of the element
         * @param text (optional) — Visible text content
         * @param boundingBox (optional) — Bounding box of the element (x1, y1, x2, y2)
         * @returns fingerprint — Created element fingerprint
         * @impure has side effects / drives control flow
         */
        function create({ id?: string, selectors?: Struct, role?: string, name?: string, text?: string, boundingBox?: Struct }): Struct;

        /**
         * Extracts individual fields from a fingerprint
         * @node fingerprint_extract_data @receiver fingerprint @alias fingerprintExtractData
         * @param fingerprint — Fingerprint to extract from (receiver: `this` in `x.extractData(...)`)
         * @returns id — Fingerprint ID
         * @returns role — Element role
         * @returns name — Element name
         * @returns text — Element text
         * @returns tagName — HTML tag name
         * @returns selectorCount — Number of selectors
         * @returns matchCount — Times fingerprint was matched
         * @impure has side effects / drives control flow
         */
        function extractData(this: ElementFingerprint, { fingerprint: Struct }): { id: string, role: string, name: string, text: string, tagName: string, selectorCount: int, matchCount: int };

        /**
         * Parses an element fingerprint from JSON
         * @node fingerprint_from_json @alias fingerprintFromJson
         * @param json (optional) — JSON string containing fingerprint data
         * @returns fingerprint — Parsed element fingerprint
         * @returns errorMessage — Error message if parsing failed
         * @impure has side effects / drives control flow
         */
        function fromJson({ json?: string }): { fingerprint: Struct, errorMessage: string };

        /**
         * Attempts to find an element matching the fingerprint
         * @node fingerprint_match @alias fingerprintMatch
         * @param session — Automation session
         * @param fingerprint — Fingerprint to match
         * @param strategy (optional) — Matching strategy
         * @param timeoutMs (optional) — Maximum time to search
         * @returns found — Whether element was found
         * @returns selectorUsed — The selector that matched
         * @returns confidence — Match confidence
         * @returns matchedSelector — Typed selector that matched
         * @impure has side effects / drives control flow
         */
        function match({ session: Struct, fingerprint: Struct, strategy?: string, timeoutMs?: int }): { found: bool, selectorUsed: string, confidence: float, matchedSelector: Struct };

        /**
         * Creates fingerprint matching options
         * @node fingerprint_match_options @alias fingerprintMatchOptions
         * @param strategy (optional) — Matching strategy to use
         * @param minConfidence (optional) — Minimum confidence threshold (0.0-1.0)
         * @param maxFallbackAttempts (optional) — Maximum number of fallback attempts
         * @param timeoutMs (optional) — Maximum time to search
         * @returns options — Fingerprint match options
         * @impure has side effects / drives control flow
         */
        function matchOptions({ strategy?: string, minConfidence?: float, maxFallbackAttempts?: int, timeoutMs?: int }): Struct;

        /**
         * Records that a fingerprint was successfully matched
         * @node fingerprint_record_match @receiver fingerprint @alias fingerprintRecordMatch
         * @param fingerprint — Fingerprint that was matched (receiver: `this` in `x.recordMatch(...)`)
         * @returns updatedFingerprint — Fingerprint with updated match stats
         * @returns matchCount — Total times this fingerprint has matched
         * @impure has side effects / drives control flow
         */
        function recordMatch(this: ElementFingerprint, { fingerprint: Struct }): { updatedFingerprint: Struct, matchCount: int };

        /**
         * Serializes an element fingerprint to JSON
         * @node fingerprint_to_json @receiver fingerprint @alias fingerprintToJson
         * @param fingerprint — Fingerprint to serialize (receiver: `this` in `x.toJson(...)`)
         * @param pretty (optional) — Use pretty formatting
         * @returns json — JSON string
         * @impure has side effects / drives control flow
         */
        function toJson(this: ElementFingerprint, { fingerprint: Struct, pretty?: bool }): string;

        /**
         * Updates an existing fingerprint with new data
         * @node fingerprint_update @receiver fingerprint @alias fingerprintUpdate
         * @param fingerprint — Fingerprint to update (receiver: `this` in `x.update(...)`)
         * @param selectors — New selector set (optional)
         * @param role (optional) — New role (empty to keep existing)
         * @param name (optional) — New name (empty to keep existing)
         * @param text (optional) — New text (empty to keep existing)
         * @returns updatedFingerprint — Updated element fingerprint
         * @impure has side effects / drives control flow
         */
        function update(this: ElementFingerprint, { fingerprint: Struct, selectors: Struct, role?: string, name?: string, text?: string }): Struct;
    }

    namespace llm {
        // === Automation/LLM/Healing ===

        /**
         * Uses LLM to diagnose automation failures and suggest/apply healing actions
         * @node llm_diagnose_and_heal @alias llmDiagnoseAndHeal
         * @param model — LLM model (vision-capable preferred)
         * @param screenshot (optional) — Optional screenshot at the time of failure as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Optional screenshot at the time of failure as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param errorMessage — The error message from the failed action
         * @param actionType — Type of action that failed (click, type, wait, find, etc.)
         * @param actionTarget — The target of the failed action (selector, text, or an x,y point in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot)
         * @param context (optional) — Additional context about what the automation was trying to do
         * @param pageHtml (optional) — Current page HTML (for selector-based failures); only the first 30,000 bytes are sent
         * @returns result — Full healing result
         * @returns diagnosis — Failure diagnosis
         * @returns newValue — Healed value (new selector, text, or an x,y point in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot)
         * @impure has side effects / drives control flow
         */
        function diagnoseAndHeal({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, errorMessage: string, actionType: string, actionTarget: string, context?: string, pageHtml?: string }): { result: Struct, diagnosis: Struct, newValue: string };

        /**
         * Uses LLM to fix a broken CSS/XPath selector based on page context
         * @node llm_heal_selector @alias llmHealSelector
         * @param model — LLM model (vision-capable preferred)
         * @param screenshot (optional) — Optional (recommended) screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Optional (recommended) screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param pageHtml — Current page HTML or DOM structure; only the first 50,000 bytes are sent
         * @param brokenSelector — The selector that no longer works
         * @param elementDescription — Description of what the selector should match
         * @param selectorType (optional) — Type of selector: css, xpath, or accessibility
         * @returns result — Healed selector result
         * @returns newSelector — The healed selector string
         * @impure has side effects / drives control flow
         */
        function healSelector({ model: Struct, screenshot?: string, image?: Struct, pageHtml: string, brokenSelector: string, elementDescription: string, selectorType?: string }): { result: Struct, newSelector: string };

        /**
         * Uses vision LLM to find a visually similar element when template matching fails
         * @node llm_heal_template @alias llmHealTemplate
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Current screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Current screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param template — Base64-encoded template image (PNG, JPEG, WebP or GIF) that failed to match
         * @param elementDescription — Description of what the template represents
         * @param lastKnownPosition (optional) — Where the element was previously found (x,y in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot)
         * @returns result — Healed template result; points and regions are in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @returns x — X coordinate of the found element's center, in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @returns y — Y coordinate of the found element's center, in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @impure has side effects / drives control flow
         */
        function healTemplate({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, template: string, elementDescription: string, lastKnownPosition?: string }): { result: Struct, x: int, y: int };

        // === Automation/LLM/Planning ===

        /**
         * Uses LLM to plan a sequence of automation actions to achieve a goal
         * @node llm_plan_actions @alias llmPlanActions
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Current screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Current screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param executionTarget (optional) — General proposes actions for any surface. Browser creates a plan for Execute Browser Action Plan.
         * @param pageContext (optional) — DOM or accessibility snapshot containing selectors for browser actions
         * @param goal — What the automation should accomplish
         * @param availableActions (optional) — JSON array of available action types and their parameters
         * @param constraints (optional) — Any constraints or preferences for the plan
         * @returns plan — Complete action plan
         * @returns actions — List of planned actions. In General plans parameters.x/parameters.y are a screen position in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @returns firstAction — The first action to execute
         * @impure has side effects / drives control flow
         */
        function planActions({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, executionTarget?: string, pageContext?: string, goal: string, availableActions?: string, constraints?: string }): { plan: Struct, actions: Struct[], firstAction: Struct };

        /**
         * Uses LLM to suggest the most appropriate next action given current screen and goal
         * @node llm_suggest_next_step @alias llmSuggestNextStep
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Current screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Current screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param goal — Ultimate goal we're trying to achieve
         * @param completedActions (optional) — JSON array of actions already taken
         * @param lastResult (optional) — Result/outcome of the last action
         * @returns suggestion — Next step suggestion; target_coordinates are in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @returns actionType — Type of suggested action
         * @returns target — Target description
         * @impure has side effects / drives control flow
         */
        function suggestNextStep({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, goal: string, completedActions?: string, lastResult?: string }): { suggestion: Struct, actionType: string, target: string };

        // === Automation/LLM/Vision ===

        /**
         * Uses vision LLM to classify screen state and identify visible elements
         * @node llm_classify_screen @alias llmClassifyScreen
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param expectedStates (optional) — Comma-separated list of possible states to classify into
         * @returns classification — Screen classification result
         * @returns screenType — Detected screen type
         * @returns state — Current screen state
         * @impure has side effects / drives control flow
         */
        function classifyScreen({ model: Struct, screenshot?: string, image?: Struct, expectedStates?: string }): { classification: Struct, screenType: string, state: string };

        /**
         * Uses vision LLM to describe a specific UI element at given coordinates
         * @node llm_describe_element @alias llmDescribeElement
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param x — X coordinate of the element, in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param y — Y coordinate of the element, in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @returns element — Element description
         * @returns description — Text description
         * @impure has side effects / drives control flow
         */
        function describeElement({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, x: int, y: int }): { element: Struct, description: string };

        /**
         * Uses vision LLM to extract structured data from a screenshot
         * @node llm_extract_from_screen @alias llmExtractFromScreen
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param schema — JSON Schema describing what to extract (or example JSON)
         * @param hint (optional) — Optional extraction hint
         * @returns data — Extracted structured data, validated against the schema
         * @impure has side effects / drives control flow
         */
        function extractFromScreen({ model: Struct, screenshot?: string, image?: Struct, schema: string, hint?: string }): any;

        /**
         * Uses a vision LLM to locate UI elements based on natural language description
         * @node llm_find_element @alias llmFindElement
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot of the screen as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot of the screen as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param description — Natural language description of the element to find (e.g., 'the blue submit button')
         * @param context (optional) — Optional context about the application or page
         * @returns location — Element location. x/y is the element's center and width/height its size, in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @impure has side effects / drives control flow
         */
        function findElement({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, description: string, context?: string }): Struct;

        /**
         * Uses vision LLM to comprehensively observe and describe the current screen
         * @node llm_observe_screen @alias llmObserveScreen
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param focusArea (optional) — Specific area or aspect to focus on (optional)
         * @returns observation — Complete screen observation
         * @returns description — Text description of the screen
         * @returns elements — Observed elements. Optional x/y is the element's center in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @impure has side effects / drives control flow
         */
        function observeScreen({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, focusArea?: string }): { observation: Struct, description: string, elements: Struct[] };

        /**
         * Uses LLM to rank multiple element candidates based on match quality
         * @node llm_rank_candidates @alias llmRankCandidates
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param candidates — Candidate elements to rank, each with a unique id. Optional x/y are in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param criteria — What the target element should match (description/intent)
         * @param context (optional) — Additional context for ranking
         * @returns result — Full ranking result; only given candidate ids appear
         * @returns bestMatch — ID of the best matching candidate
         * @returns ranked — Candidates sorted by rank
         * @impure has side effects / drives control flow
         */
        function rankCandidates({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, candidates: Struct[], criteria: string, context?: string }): { result: Struct, bestMatch: string, ranked: Struct[] };

        /**
         * Uses LLM to disambiguate between multiple element candidates
         * @node llm_resolve_element @alias llmResolveElement
         * @param model — Vision-capable LLM model
         * @param screenshot (optional) — Screenshot as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected
         * @param image (optional) — Screenshot as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot
         * @param frame (optional) — Screen frame of the screenshot, from the capture node. Coordinates are desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param candidates — Element candidates to choose from, each with a unique index. x/y are in desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot
         * @param intent — What the user is trying to accomplish
         * @returns result — Resolution result; x/y are copied from the chosen candidate
         * @impure has side effects / drives control flow
         */
        function resolveElement({ model: Struct, screenshot?: string, image?: Struct, frame?: Struct, candidates: Struct[], intent: string }): Struct;
    }

    namespace selector {
        // === Automation/Selector ===

        /**
         * Adds a selector to an existing selector set
         * @node selector_add_to_set @receiver selector_set @alias selectorAddToSet
         * @param selectorSet — Existing selector set (receiver: `this` in `x.addToSet(...)`)
         * @param selector — Selector to add
         * @returns updatedSet — Selector set with new selector added
         * @impure has side effects / drives control flow
         */
        function addToSet(this: SelectorSet, { selectorSet: Struct, selector: Struct }): Struct;

        /**
         * Creates a selector from a value and kind
         * @node selector_build @alias selectorBuild
         * @param kind (optional) — Type of selector
         * @param value (optional) — Selector value (CSS selector, XPath, text, etc.)
         * @param confidence (optional) — Confidence level (0.0-1.0)
         * @param scope (optional) — Optional scope selector to narrow search
         * @returns selector — The built selector
         * @impure has side effects / drives control flow
         */
        function build({ kind?: string, value?: string, confidence?: float, scope?: string }): Struct;

        /**
         * Creates a new empty selector set
         * @node selector_create_set @alias selectorCreateSet
         * @returns selectorSet — Empty selector set
         * @impure has side effects / drives control flow
         */
        function createSet(): Struct;

        /**
         * Gets the highest-ranked selector from a ranked set
         * @node selector_get_best @receiver ranked_set @alias selectorGetBest
         * @param rankedSet — Ranked selector set (receiver: `this` in `x.getBest(...)`)
         * @returns selector — Best selector
         * @returns score — Selector score
         * @impure has side effects / drives control flow
         */
        function getBest(this: RankedSelectorSet, { rankedSet: Struct }): { selector: Struct, score: float };

        /**
         * Gets the primary (first) selector from a selector set
         * @node selector_get_primary @receiver selector_set @alias selectorGetPrimary
         * @param selectorSet — Selector set to get primary from (receiver: `this` in `x.getPrimary(...)`)
         * @returns selector — Primary selector
         * @impure has side effects / drives control flow
         */
        function getPrimary(this: SelectorSet, { selectorSet: Struct }): Struct;

        /**
         * Ranks selectors in a set by their confidence and specificity
         * @node selector_rank @receiver selector_set @alias selectorRank
         * @param selectorSet — Selector set to rank (receiver: `this` in `x.rank(...)`)
         * @returns rankedSet — Ranked selector set
         * @impure has side effects / drives control flow
         */
        function rank(this: SelectorSet, { selectorSet: Struct }): Struct;

        /**
         * Converts a ranked selector set back to a regular selector set
         * @node selector_ranked_to_set @receiver ranked_set @alias selectorRankedToSet
         * @param rankedSet — Ranked selector set to convert (receiver: `this` in `x.rankedToSet(...)`)
         * @returns selectorSet — Regular selector set with ranked order
         * @impure has side effects / drives control flow
         */
        function rankedToSet(this: RankedSelectorSet, { rankedSet: Struct }): Struct;

        /**
         * Converts a selector to its string representation
         * @node selector_to_string @receiver selector @alias selectorToString
         * @param selector — Selector to convert (receiver: `this` in `x.toString(...)`)
         * @returns kind — Selector kind
         * @returns value — Selector value
         * @returns confidence — Selector confidence
         * @impure has side effects / drives control flow
         */
        function toString(this: Selector, { selector: Struct }): { kind: string, value: string, confidence: float };

        /**
         * Validates a selector's format and structure
         * @node selector_validate @receiver selector @alias selectorValidate
         * @param selector — Selector to validate (receiver: `this` in `x.validate(...)`)
         * @returns isValid — Whether selector is valid
         * @returns error — Validation error message
         * @impure has side effects / drives control flow
         */
        function validate(this: Selector, { selector: Struct }): { isValid: bool, error: string };
    }

    namespace vision {
        // === Automation/Vision ===

        /**
         * Finds a template image on screen and clicks on it. Fails instead of guessing when two matches score within 0.01 of each other; use Find All Templates to choose one
         * @node vision_click_template @alias visionClickTemplate
         * @param session — Automation session handle
         * @param template — Path to the template image file (FlowPath with caching support)
         * @param monitor (optional) — Display index, -1 for primary, or -2 for all displays
         * @param confidence (optional) — Minimum match confidence (0.0-1.0)
         * @param clickType (optional) — Type of click to perform
         * @param offsetX (optional) — X offset from center of matched template
         * @param offsetY (optional) — Y offset from center of matched template
         * @param fallbackX (optional) — Desktop X to click when the template is not found. The fallback is disabled only when both Fallback X and Fallback Y are -1, so negative coordinates on displays left of the primary work
         * @param fallbackY (optional) — Desktop Y to click when the template is not found (both -1 disables the fallback)
         * @returns found — Whether the template was found and clicked
         * @returns x — X coordinate where clicked
         * @returns y — Y coordinate where clicked
         * @impure has side effects / drives control flow
         */
        function clickTemplate({ session: Struct, template: Struct, monitor?: int, confidence?: float, clickType?: string, offsetX?: int, offsetY?: int, fallbackX?: int, fallbackY?: int }): { found: bool, x: int, y: int };

        /**
         * Searches the screen for all occurrences of a template image
         * @node vision_find_all_templates @alias visionFindAllTemplates
         * @param session — Automation session handle for screen operations
         * @param template — Template image file
         * @param monitor (optional) — Display index, -1 for primary, or -2 for all displays
         * @param confidence (optional) — Minimum match confidence (0.0-1.0)
         * @param maxResults (optional) — Maximum number of matches to return
         * @returns count — Number of matches found
         * @returns results — Array of match results (as JSON)
         * @impure has side effects / drives control flow
         */
        function findAllTemplates({ session: Struct, template: Struct, monitor?: int, confidence?: float, maxResults?: int }): { count: int, results: any };

        /**
         * Searches the screen for a template image and returns its location
         * @node vision_find_template @alias visionFindTemplate
         * @param session — Automation session handle for screen operations
         * @param template — Template image file
         * @param monitor (optional) — Display index, -1 for primary, or -2 for all displays
         * @param confidence (optional) — Minimum match confidence (0.0-1.0)
         * @param matchMode (optional) — Automatic template matching for this platform
         * @returns found — Whether the template was found
         * @returns result — Match result with location and confidence
         * @returns x — X coordinate of match center
         * @returns y — Y coordinate of match center
         * @impure has side effects / drives control flow
         */
        function findTemplate({ session: Struct, template: Struct, monitor?: int, confidence?: float, matchMode?: string }): { found: bool, result: Struct, x: int, y: int };

        /**
         * Gets the color at a screen position. By default X/Y are desktop coordinates, the same ones the mouse nodes and Assert Color use; set Coordinate Space to pixels for the version 1 behaviour (screenshot pixels of Monitor)
         * @node vision_get_pixel_color @alias visionGetPixelColor
         * @param session — Automation session handle
         * @param x (optional) — X position
         * @param y (optional) — Y position
         * @param coordinateSpace (optional) — desktop: X/Y are desktop input coordinates on any display (Monitor is ignored). pixels: X/Y are screenshot pixels of Monitor
         * @param monitor (optional) — Monitor index from List Displays (-1 = primary); only used when Coordinate Space is pixels
         * @returns red — Red component (0-255)
         * @returns green — Green component (0-255)
         * @returns blue — Blue component (0-255)
         * @returns hex — Hex color code (#RRGGBB)
         * @impure has side effects / drives control flow
         */
        function getPixelColor({ session: Struct, x?: int, y?: int, coordinateSpace?: string, monitor?: int }): { red: int, green: int, blue: int, hex: string };

        /**
         * Gets the size of a monitor in desktop coordinates (as Get Display reports it) and in screenshot pixels
         * @node vision_get_screen_size @alias visionGetScreenSize
         * @param session — Automation session handle
         * @param monitor (optional) — Monitor index from List Displays (-1 = primary)
         * @returns width — Width in desktop input coordinates
         * @returns height — Height in desktop input coordinates
         * @returns pixelWidth — Width of a screenshot of this monitor in pixels
         * @returns pixelHeight — Height of a screenshot of this monitor in pixels
         * @returns frame — Desktop rectangle and screenshot pixel size of the monitor
         * @impure has side effects / drives control flow
         */
        function getScreenSize({ session: Struct, monitor?: int }): { width: int, height: int, pixelWidth: int, pixelHeight: int, frame: Struct };

        /**
         * Captures a region of a display, given in that display's screenshot pixels, and optionally saves it. Frame converts pixels of the result to mouse coordinates
         * @node vision_screenshot_region @alias visionScreenshotRegion
         * @param session — Automation session handle
         * @param x (optional) — Left edge in screenshot pixels of the display
         * @param y (optional) — Top edge in screenshot pixels of the display
         * @param width (optional) — Region width in screenshot pixels
         * @param height (optional) — Region height in screenshot pixels
         * @param filePath — Path to save the screenshot
         * @param monitor (optional) — Monitor index from List Displays (-1 = primary); coordinates are screenshot pixels
         * @returns success — Whether the screenshot was saved
         * @returns image — Screenshot as NodeImage
         * @returns frame — Desktop rectangle and pixel size of the captured region
         * @impure has side effects / drives control flow
         */
        function screenshotRegion({ session: Struct, x?: int, y?: int, width?: int, height?: int, filePath: Struct, monitor?: int }): { success: bool, image: Struct, frame: Struct };

        /**
         * Captures a display and optionally saves it as PNG
         * @node vision_screenshot_to_file @alias visionScreenshotToFile
         * @param session — Automation session handle
         * @param filePath — Path to save the screenshot
         * @param monitor (optional) — Monitor index from List Displays (-1 = primary)
         * @returns success — Whether the screenshot was saved
         * @returns image — Screenshot as NodeImage
         * @returns frame — Desktop rectangle and pixel size of the display; converts image pixels to mouse coordinates
         * @impure has side effects / drives control flow
         */
        function screenshotToFile({ session: Struct, filePath: Struct, monitor?: int }): { success: bool, image: Struct, frame: Struct };

        /**
         * Waits for a template image to appear on screen
         * @node vision_wait_template @alias visionWaitTemplate
         * @param session — Automation session handle
         * @param template — Template image file
         * @param monitor (optional) — Display index, -1 for primary, or -2 for all displays
         * @param confidence (optional) — Minimum match confidence (0.0-1.0)
         * @param timeoutMs (optional) — Maximum time to wait
         * @param pollIntervalMs (optional) — How often to check for template
         * @returns found — Whether the template was found
         * @returns result — Match result with location
         * @impure has side effects / drives control flow
         */
        function waitTemplate({ session: Struct, template: Struct, monitor?: int, confidence?: float, timeoutMs?: int, pollIntervalMs?: int }): { found: bool, result: Struct };

        /**
         * Waits for a template image to disappear from screen
         * @node vision_wait_template_disappear @alias visionWaitTemplateDisappear
         * @param session — Automation session handle
         * @param template — Template image file
         * @param monitor (optional) — Display index, -1 for primary, or -2 for all displays
         * @param confidence (optional) — Minimum match confidence (0.0-1.0)
         * @param timeoutMs (optional) — Maximum time to wait
         * @returns disappeared — Whether the template disappeared
         * @impure has side effects / drives control flow
         */
        function waitTemplateDisappear({ session: Struct, template: Struct, monitor?: int, confidence?: float, timeoutMs?: int }): bool;
    }
}

declare namespace browser {
    // === Automation/Browser ===

    /**
     * Attaches ChromeDriver or EdgeDriver to an existing debugging-enabled browser.
     * @node browser_attach @alias browserAttach
     * @param session — Automation session
     * @param webdriverUrl (optional) — Running ChromeDriver or EdgeDriver URL
     * @param debuggerAddress (optional) — Existing browser debugging host:port
     * @param browserType (optional) — Chrome or Edge
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function attach({ session: Struct, webdriverUrl?: string, debuggerAddress?: string, browserType?: string }): Struct;

    /**
     * Closes a browser started by this session, or disconnects from an existing browser
     * @node browser_close @alias browserClose
     * @param session — Automation session with browser to close
     * @returns sessionOut — Session with browser detached
     * @impure has side effects / drives control flow
     */
    function close({ session: Struct }): Struct;

    /**
     * Closes a browser page/tab
     * @node browser_close_page @alias browserClosePage
     * @param session — Automation session with page to close
     * @returns sessionOut — Session selecting a remaining tab when available
     * @impure has side effects / drives control flow
     */
    function closePage({ session: Struct }): Struct;

    /**
     * Drags an element to another element.
     * @node browser_drag @alias browserDrag
     * @param session — Automation session
     * @param source — Element to drag
     * @param target — Drop target
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function drag({ session: Struct, source: Struct, target: Struct }): Struct;

    /**
     * Selects an iframe for subsequent actions on this session wire.
     * @node browser_enter_frame @alias browserEnterFrame
     * @param session — Automation session
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param selector (optional) — Frame CSS selector
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function enterFrame({ session: Struct, locator?: Struct, selector?: string }): Struct;

    /**
     * Executes a validated LLM browser plan in order, stopping on the first failed action. Navigate actions follow the session navigation policy (HTTP and HTTPS only when none is set), including the final URL after redirects.
     * @node browser_execute_plan @alias browserExecutePlan
     * @param session — Automation session
     * @param plan — Plan from LLM Plan Actions with CSS or typed selectors
     * @param maxActions (optional) — Maximum actions accepted in one plan
     * @param actionTimeoutMs (optional) — Maximum time per action in milliseconds
     * @returns sessionOut — Updated automation session
     * @returns executedCount — Number of completed actions
     * @impure has side effects / drives control flow
     */
    function executePlan({ session: Struct, plan: Struct, maxActions?: int, actionTimeoutMs?: int }): { sessionOut: Struct, executedCount: int };

    /**
     * Reads, accepts, or dismisses a JavaScript alert, confirm, or prompt.
     * @node browser_handle_dialog @alias browserHandleDialog
     * @param session — Automation session
     * @param action (optional) — read, accept, or dismiss
     * @param text (optional) — Text for a prompt before accepting
     * @returns sessionOut — Updated automation session
     * @returns dialogText — Text shown in the dialog
     * @impure has side effects / drives control flow
     */
    function handleDialog({ session: Struct, action?: string, text?: string }): { sessionOut: Struct, dialogText: string };

    /**
     * Presses a key while holding browser modifier keys.
     * @node browser_key_chord @alias browserKeyChord
     * @param session — Automation session
     * @param key — Character or key name such as Enter
     * @param modifiers (optional) — Control, Shift, Alt, or Meta
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function keyChord({ session: Struct, key: string, modifiers?: string[] }): Struct;

    /**
     * Returns to the parent frame or the top-level document.
     * @node browser_leave_frame @alias browserLeaveFrame
     * @param session — Automation session
     * @param topLevel (optional) — Return to the top-level document
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function leaveFrame({ session: Struct, topLevel?: bool }): Struct;

    /**
     * Lists the browser tabs and their URLs.
     * @node browser_list_tabs @alias browserListTabs
     * @param session — Automation session
     * @returns sessionOut — Updated automation session
     * @returns tabs — Open browser tabs
     * @impure has side effects / drives control flow
     */
    function listTabs({ session: Struct }): { sessionOut: Struct, tabs: Struct[] };

    /**
     * Creates a new browser page/tab in the given context
     * @node browser_new_page @alias browserNewPage
     * @param session — Automation session with browser attached
     * @returns sessionOut — Automation session with new page set as current
     * @impure has side effects / drives control flow
     */
    function newPage({ session: Struct }): Struct;

    /**
     * Connects to a WebDriver server and opens a new browser session, optionally with a persistent profile, proxy, locale and relaxed certificate checks
     * @node browser_open @alias browserOpen
     * @param session — Automation session to attach browser to
     * @param webdriverUrl (optional) — URL of the WebDriver server (e.g., http://localhost:9515 for ChromeDriver)
     * @param browserType (optional) — Browser to use (Chrome, Firefox, Edge, Safari)
     * @param headless (optional) — Run browser in headless mode (no visible window)
     * @param viewportWidth (optional) — Page viewport width in CSS pixels; the window grows by the browser frame so the page gets this size (the screen may cap it)
     * @param viewportHeight (optional) — Page viewport height in CSS pixels; the window grows by the browser frame so the page gets this size (the screen may cap it)
     * @param userAgent (optional) — Custom user agent string (optional)
     * @param pageLoadTimeout (optional) — Timeout for page loads in seconds (at least 1)
     * @param userDataDir (optional) — Local directory for a persistent browser profile (cookies, storage, logins survive between runs). Chrome and Edge allow one browser per profile at a time. Requires WebDriver on this machine.
     * @param userDataPath (optional) — Absolute profile directory on the WebDriver host; used when Profile Directory is not connected
     * @param proxyServer (optional) — Proxy such as http://host:8080 or socks5://host:1080 (proxy credentials are not supported)
     * @param proxyBypass (optional) — Comma-separated hosts that skip the proxy, such as localhost,*.internal
     * @param locale (optional) — Browser language and Accept-Language such as de-DE (empty keeps the default)
     * @param ignoreHttpsErrors (optional) — Accept invalid or self-signed TLS certificates
     * @returns debuggerAddress — Chrome or Edge debugger endpoint, when available
     * @returns sessionOut — Automation session with browser attached
     * @impure has side effects / drives control flow
     */
    function open({ session: Struct, webdriverUrl?: string, browserType?: string, headless?: bool, viewportWidth?: int, viewportHeight?: int, userAgent?: string, pageLoadTimeout?: int, userDataDir?: Struct, userDataPath?: string, proxyServer?: string, proxyBypass?: string, locale?: string, ignoreHttpsErrors?: bool }): { debuggerAddress: string, sessionOut: Struct };

    /**
     * Opens the context menu for an element.
     * @node browser_right_click @alias browserRightClick
     * @param session — Automation session
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param selector (optional) — Legacy CSS selector
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function rightClick({ session: Struct, locator?: Struct, selector?: string }): Struct;

    /**
     * Selects a tab by its explicit browser handle.
     * @node browser_select_tab @alias browserSelectTab
     * @param session — Automation session
     * @param targetId (optional) — Exact CDP target ID of an attached Chrome or Edge tab
     * @param handle (optional) — Handle returned by List Tabs
     * @param url (optional) — Exact URL when no handle is supplied
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function selectTab({ session: Struct, targetId?: string, handle?: string, url?: string }): Struct;

    /**
     * Captures browser console messages before navigation or actions.
     * @node browser_start_console_observer @alias browserStartConsoleObserver
     * @param session — Automation session
     * @param debuggerAddress (optional) — Chrome or Edge debugging address (host:port)
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function startConsoleObserver({ session: Struct, debuggerAddress?: string }): Struct;

    /**
     * Starts an installed WebDriver executable and waits until it is ready.
     * @node browser_start_driver @alias browserStartDriver
     * @param session — Automation session
     * @param executable (optional) — Path to chromedriver, geckodriver, or msedgedriver
     * @param port (optional) — Local WebDriver port
     * @returns sessionOut — Updated automation session
     * @returns webdriverUrl — Ready local WebDriver endpoint
     * @impure has side effects / drives control flow
     */
    function startDriver({ session: Struct, executable?: string, port?: int }): { sessionOut: Struct, webdriverUrl: string };

    /**
     * Stops the WebDriver process started for this automation session.
     * @node browser_stop_driver @alias browserStopDriver
     * @param session — Automation session
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function stopDriver({ session: Struct }): Struct;

    /**
     * Waits for the current tab to reach an expected URL without navigating again.
     * @node browser_wait_for_url @alias browserWaitForUrl
     * @param session — Automation session
     * @param expectedUrl — Exact URL after navigation
     * @param timeoutMs (optional) — Maximum wait in milliseconds
     * @returns sessionOut — Updated automation session
     * @returns found — Expected URL was reached
     * @impure has side effects / drives control flow
     */
    function waitForUrl({ session: Struct, expectedUrl: string, timeoutMs?: int }): { sessionOut: Struct, found: bool };

    // === Automation/Browser/Auth ===

    /**
     * Clears cookies: every domain on Chrome and Edge, the current document's cookies on other browsers
     * @node browser_clear_cookies @alias browserClearCookies
     * @param session — Automation session
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function clearCookies({ session: Struct }): Struct;

    /**
     * Loads cookies from a file into the browser session. Chrome and Edge accept cookies for every domain with their HttpOnly, Secure and SameSite flags; other browsers only accept cookies for the current page's domain.
     * @node browser_load_cookies @alias browserLoadCookies
     * @param session — Automation session
     * @param filePath — Path to cookies JSON file
     * @returns sessionOut — Automation session (pass-through)
     * @returns cookieCount — Number of cookies the browser accepted
     * @returns failedCount — Number of cookies that were expired or rejected
     * @impure has side effects / drives control flow
     */
    function loadCookies({ session: Struct, filePath: Struct }): { sessionOut: Struct, cookieCount: int, failedCount: int };

    /**
     * Restores cookies and web storage from a Playwright-compatible storageState JSON file. Cookies for every domain are restored on Chrome and Edge (only the current domain elsewhere). Storage is applied only to the origin the page is on, without navigating; other origins are reported as skipped.
     * @node browser_load_storage_state @alias browserLoadStorageState
     * @param session — Automation session
     * @param filePath — Storage state JSON written by Save Storage State or Playwright
     * @returns sessionOut — Updated automation session
     * @returns cookiesApplied — Cookies the browser accepted
     * @returns cookiesFailed — Cookies that were expired or rejected
     * @returns originsApplied — Origins whose storage was restored
     * @returns skippedOrigins — Origins not restored because the page is on a different origin
     * @impure has side effects / drives control flow
     */
    function loadStorageState({ session: Struct, filePath: Struct }): { sessionOut: Struct, cookiesApplied: int, cookiesFailed: int, originsApplied: int, skippedOrigins: string[] };

    /**
     * Saves browser cookies to a file for later restoration: every domain including HttpOnly cookies on Chrome and Edge, the current document's cookies on other browsers
     * @node browser_save_cookies @alias browserSaveCookies
     * @param session — Automation session
     * @param filePath — Path to save cookies JSON file
     * @returns sessionOut — Automation session (pass-through)
     * @returns cookieCount — Number of cookies saved
     * @impure has side effects / drives control flow
     */
    function saveCookies({ session: Struct, filePath: Struct }): { sessionOut: Struct, cookieCount: int };

    /**
     * Saves all cookies (every domain, including HttpOnly, on Chrome and Edge) and the current origin's localStorage to a Playwright-compatible storageState JSON file. The file holds login sessions; store it like a password.
     * @node browser_save_storage_state @alias browserSaveStorageState
     * @param session — Automation session
     * @param filePath — Where to write the storage state JSON
     * @param includeSessionStorage (optional) — Also save the current origin's sessionStorage
     * @returns sessionOut — Updated automation session
     * @returns cookieCount — Number of cookies saved
     * @returns originCount — Number of origins whose storage was saved
     * @impure has side effects / drives control flow
     */
    function saveStorageState({ session: Struct, filePath: Struct, includeSessionStorage?: bool }): { sessionOut: Struct, cookieCount: int, originCount: int };

    /**
     * Configures HTTP Basic Authentication credentials for requests
     * @node browser_set_basic_auth @alias browserSetBasicAuth
     * @param session — Automation session
     * @param username (optional) — HTTP Basic Auth username
     * @param password (optional) — HTTP Basic Auth password
     * @param origin — HTTP(S) origin allowed to receive credentials
     * @param debuggerAddress (optional) — Optional Chrome or Edge debugger address; defaults to the attached browser
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function setBasicAuth({ session: Struct, username?: string, password?: string, origin: string, debuggerAddress?: string }): Struct;

    /**
     * Generates the current RFC 6238 one-time password (authenticator app code) from a base32 secret
     * @node totp_code @alias totpCode
     * @param secret (optional) — Base32 shared secret from the authenticator setup (spaces and dashes are ignored)
     * @param digits (optional) — Code length (6 to 8)
     * @param period (optional) — Seconds each code is valid
     * @param algorithm (optional) — HMAC algorithm of the secret
     * @param unixTime (optional) — Time in Unix seconds to generate the code for; negative uses the current time
     * @returns code — One-time password
     * @returns secondsRemaining — Seconds until the code changes
     */
    function totpCode({ secret?: string, digits?: int, period?: int, algorithm?: string, unixTime?: int }): { code: string, secondsRemaining: int };

    // === Automation/Browser/Capture ===

    /**
     * Prints the current page to a PDF file, the way the browser's print dialog would. Chrome and Edge print through DevTools (headless Chrome is the most reliable); other browsers use WebDriver printing.
     * @node browser_print_pdf @alias browserPrintPdf
     * @param session — Automation session
     * @param filePath — Where to write the PDF; an existing file is replaced
     * @param paperFormat (optional) — Paper size of each PDF page
     * @param landscape (optional) — Print in landscape orientation
     * @param printBackground (optional) — Include background colors and images
     * @param scale (optional) — Content scale between 0.1 and 2
     * @returns sessionOut — Updated automation session
     * @returns pdfPath — The written PDF file
     * @impure has side effects / drives control flow
     */
    function printPdf({ session: Struct, filePath: Struct, paperFormat?: string, landscape?: bool, printBackground?: bool, scale?: float }): { sessionOut: Struct, pdfPath: Struct };

    /**
     * Takes a screenshot of the current page
     * @node browser_screenshot @alias browserScreenshot
     * @param session — Automation session
     * @param fullPage (optional) — Capture the entire scrollable page instead of the viewport (Chrome and Edge only)
     * @returns sessionOut — Automation session (pass-through)
     * @returns screenshot — Screenshot as base64 PNG data
     * @returns image — Screenshot as NodeImage
     * @impure has side effects / drives control flow
     */
    function screenshot({ session: Struct, fullPage?: bool }): { sessionOut: Struct, screenshot: string, image: Struct };

    /**
     * Takes a screenshot of a specific element
     * @node browser_screenshot_element @alias browserScreenshotElement
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element to screenshot
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns screenshot — Screenshot as base64 PNG data
     * @returns image — Screenshot as NodeImage
     * @impure has side effects / drives control flow
     */
    function screenshotElement({ session: Struct, selector?: string, locator?: Struct }): { sessionOut: Struct, screenshot: string, image: Struct };

    /**
     * Captures a region of the visible page at a higher device scale, so small text and icons stay legible for vision models. Coordinates are viewport CSS pixels, the same space as element bounding boxes and Click At Point. Chrome and Edge only.
     * @node browser_zoom_screenshot @alias browserZoomScreenshot
     * @param session — Automation session
     * @param x (optional) — Left edge of the region in viewport CSS pixels
     * @param y (optional) — Top edge of the region in viewport CSS pixels
     * @param width (optional) — Region width in CSS pixels
     * @param height (optional) — Region height in CSS pixels
     * @param scale (optional) — Render scale: 2 doubles the region's pixel size (on top of the display's pixel ratio); up to 10
     * @returns sessionOut — Updated automation session
     * @returns screenshot — Region as base64 PNG data
     * @returns image — Region as NodeImage
     * @impure has side effects / drives control flow
     */
    function zoomScreenshot({ session: Struct, x?: float, y?: float, width?: float, height?: float, scale?: float }): { sessionOut: Struct, screenshot: string, image: Struct };

    // === Automation/Browser/Emulation ===

    /**
     * Stops the current tab from loading matching URLs, such as ads, trackers or heavy media. Patterns use * as a wildcard. Replaces the previous list; an empty list unblocks everything. Chrome and Edge only.
     * @node browser_block_urls @alias browserBlockUrls
     * @param session — Automation session
     * @param patterns (optional) — URL patterns such as *://*.doubleclick.net/* or *.mp4
     * @returns sessionOut — Updated automation session
     * @returns blockedCount — Number of patterns now blocked
     * @impure has side effects / drives control flow
     */
    function blockUrls({ session: Struct, patterns?: string[] }): { sessionOut: Struct, blockedCount: int };

    /**
     * Makes the current tab behave like another device or place: locale, time zone, geolocation, screen size, color scheme, user agent and offline mode. Empty or zero inputs leave that setting unchanged; Offline is always applied. Overrides last until the tab closes. Chrome and Edge only.
     * @node browser_set_emulation @alias browserSetEmulation
     * @param session — Automation session
     * @param locale (optional) — BCP 47 locale such as de-DE, used for Intl formatting, navigator.language and the Accept-Language header
     * @param timezoneId (optional) — IANA time zone such as Europe/Berlin
     * @param geolocation (optional) — Position reported by the Geolocation API; also grants the geolocation permission
     * @param viewportWidth (optional) — Emulated screen width in CSS pixels; set together with Viewport Height
     * @param viewportHeight (optional) — Emulated screen height in CSS pixels; set together with Viewport Width
     * @param deviceScaleFactor (optional) — Device pixel ratio for the emulated screen; 0 keeps the display's own ratio
     * @param mobile (optional) — Emulate a mobile device (meta viewport, overlay scrollbars) with the viewport size
     * @param colorScheme (optional) — Value reported to prefers-color-scheme media queries
     * @param userAgent (optional) — User-Agent header and navigator.userAgent for this tab
     * @param offline (optional) — Simulate a lost network connection; false restores connectivity
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function setEmulation({ session: Struct, locale?: string, timezoneId?: string, geolocation?: Struct, viewportWidth?: int, viewportHeight?: int, deviceScaleFactor?: float, mobile?: bool, colorScheme?: string, userAgent?: string, offline?: bool }): Struct;

    /**
     * Adds HTTP headers to every request the current tab makes, including requests to third-party origins, so avoid credentials on pages that load foreign content. Replaces headers set earlier; an empty object removes them. Chrome and Edge only.
     * @node browser_set_extra_headers @alias browserSetExtraHeaders
     * @param session — Automation session
     * @param headers (optional) — Object mapping header names to string values, e.g. {"X-Tenant": "acme"}
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function setExtraHeaders({ session: Struct, headers?: Struct }): Struct;

    // === Automation/Browser/Extract ===

    /**
     * Counts the elements matching a selector (0 when none match).
     * @node browser_count_elements @alias browserCountElements
     * @param session — Automation session
     * @param selector (optional) — CSS selector, or a snapshot ref such as e12
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Updated automation session
     * @returns count — Number of matching elements
     * @impure has side effects / drives control flow
     */
    function countElements({ session: Struct, selector?: string, locator?: Struct }): { sessionOut: Struct, count: int };

    /**
     * Executes JavaScript code in the browser and returns the result
     * @node browser_execute_js @alias browserExecuteJs
     * @param session — Automation session
     * @param script (optional) — JavaScript code to execute (use 'return' to return a value)
     * @returns sessionOut — Automation session (pass-through)
     * @returns result — Return value from JavaScript (as JSON)
     * @impure has side effects / drives control flow
     */
    function executeJs({ session: Struct, script?: string }): { sessionOut: Struct, result: any };

    /**
     * Gets an attribute value of an element
     * @node browser_get_attribute @alias browserGetAttribute
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element
     * @param attribute (optional) — Name of attribute to get
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns value — Attribute value (empty if not found)
     * @impure has side effects / drives control flow
     */
    function getAttribute({ session: Struct, selector?: string, attribute?: string, locator?: Struct }): { sessionOut: Struct, value: string };

    /**
     * Reads whether an element is visible, enabled, checked, editable, focused and in the viewport, plus its box, tag and text. Fails when no element matches.
     * @node browser_get_element_state @alias browserGetElementState
     * @param session — Automation session
     * @param selector (optional) — CSS selector, or a snapshot ref such as e12
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Updated automation session
     * @returns visible — Rendered with a non-empty box
     * @returns enabled — Not disabled
     * @returns checked — Checked, selected or pressed
     * @returns editable — Accepts typed input
     * @returns focused — Has keyboard focus
     * @returns inViewport — Visible and intersecting the viewport
     * @returns bounds — Box in CSS pixels relative to the viewport
     * @returns tag — Lowercase tag name
     * @returns text — Visible text, or the field value (empty for passwords)
     * @returns state — All state values
     * @impure has side effects / drives control flow
     */
    function getElementState({ session: Struct, selector?: string, locator?: Struct }): { sessionOut: Struct, visible: bool, enabled: bool, checked: bool, editable: bool, focused: bool, inViewport: bool, bounds: Struct, tag: string, text: string, state: Struct };

    /**
     * Gets the HTML content of an element or the entire page
     * @node browser_get_html @alias browserGetHtml
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element (empty for entire page)
     * @param outerHtml (optional) — Include element's own tags (vs just inner content)
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns html — HTML content
     * @impure has side effects / drives control flow
     */
    function getHtml({ session: Struct, selector?: string, outerHtml?: bool, locator?: Struct }): { sessionOut: Struct, html: string };

    /**
     * Reads the rendered text of the page, or of one scoped element, and a Markdown version of the same content for prompts and summaries. Hidden elements are excluded from the text; scripts, styles and embedded media are dropped from the Markdown.
     * @node browser_get_page_text @alias browserGetPageText
     * @param session — Automation session
     * @param scope (optional) — CSS selector of the element to read; empty reads the whole page body
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param maxChars (optional) — Maximum characters returned in Text and in Markdown; 0 returns everything
     * @returns sessionOut — Updated automation session
     * @returns text — Rendered text (innerText) with the page's line breaks
     * @returns markdown — Markdown converted from the same content, keeping headings, links, lists and tables
     * @returns truncated — True when Text or Markdown was cut to Max Characters
     * @impure has side effects / drives control flow
     */
    function getPageText({ session: Struct, scope?: string, locator?: Struct, maxChars?: int }): { sessionOut: Struct, text: string, markdown: string, truncated: bool };

    /**
     * Gets the text content of an element
     * @node browser_get_text @alias browserGetText
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns text — Text content of the element
     * @impure has side effects / drives control flow
     */
    function getText({ session: Struct, selector?: string, locator?: Struct }): { sessionOut: Struct, text: string };

    /**
     * Lists elements matching a selector with their text, chosen attributes, box and visibility. When a browser snapshot exists for the page, each main-frame element also gets a ref usable as a Ref selector.
     * @node browser_list_elements @alias browserListElements
     * @param session — Automation session
     * @param selector (optional) — CSS selector, or a snapshot ref such as e12
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param attributes (optional) — Attribute names to include for each element
     * @param limit (optional) — Maximum elements returned
     * @returns sessionOut — Updated automation session
     * @returns elements — Matching elements in document order
     * @returns count — Number of matches before the limit
     * @impure has side effects / drives control flow
     */
    function listElements({ session: Struct, selector?: string, locator?: Struct, attributes?: string[], limit?: int }): { sessionOut: Struct, elements: Struct[], count: int };

    // === Automation/Browser/Files ===

    /**
     * Sets the default download directory for the browser (must be called before downloads)
     * @node browser_set_download_dir @alias browserSetDownloadDir
     * @param session — Automation session
     * @param downloadPath — Absolute path to the download directory
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function setDownloadDir({ session: Struct, downloadPath: Struct }): Struct;

    /**
     * Clicks an element to trigger a download
     * @node browser_trigger_download @alias browserTriggerDownload
     * @param session — Automation session
     * @param selector — CSS selector for the download link/button
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function triggerDownload({ session: Struct, selector: string, locator?: Struct }): Struct;

    /**
     * Uploads a file to an input element using its selector
     * @node browser_upload_file @alias browserUploadFile
     * @param session — Automation session
     * @param selector (optional) — CSS selector for the file input element
     * @param filePath — Absolute path to the file to upload
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function uploadFile({ session: Struct, selector?: string, filePath: string, locator?: Struct }): Struct;

    /**
     * Uploads multiple files to a file input that accepts multiple
     * @node browser_upload_multiple_files @alias browserUploadMultipleFiles
     * @param session — Automation session
     * @param selector (optional) — CSS selector for the file input element
     * @param filePaths — Array of absolute paths to the files to upload
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns uploadedCount — Number of files uploaded
     * @impure has side effects / drives control flow
     */
    function uploadMultipleFiles({ session: Struct, selector?: string, filePaths: string[], locator?: Struct }): { sessionOut: Struct, uploadedCount: int };

    /**
     * Waits for a file to appear in the download directory
     * @node browser_wait_for_download @alias browserWaitForDownload
     * @param session — Automation session
     * @param downloadDir — Directory to watch for downloads
     * @param filePattern (optional) — File name pattern to match (e.g., '*.pdf', leave empty for any)
     * @param timeoutMs (optional) — Maximum time to wait for download
     * @returns sessionOut — Automation session (pass-through)
     * @returns downloadedFile — Path to the downloaded file
     * @impure has side effects / drives control flow
     */
    function waitForDownload({ session: Struct, downloadDir: Struct, filePattern?: string, timeoutMs?: int }): { sessionOut: Struct, downloadedFile: Struct };

    // === Automation/Browser/Input ===

    /**
     * Fills several form fields in order: text inputs are cleared and typed, selects pick an option by value or label, checkboxes and radios are set. Each field is scrolled into view and must become visible and enabled within the timeout. Stops at the first failing field.
     * @node browser_fill_form @alias browserFillForm
     * @param session — Automation session
     * @param fields (optional) — Fields to fill: target (selector or ref such as e12), value and kind (text, select, checkbox, radio)
     * @param timeoutMs (optional) — Maximum wait per field for it to become visible and enabled
     * @returns sessionOut — Updated automation session
     * @returns filledCount — Number of fields filled
     * @impure has side effects / drives control flow
     */
    function fillForm({ session: Struct, fields?: Struct[], timeoutMs?: int }): { sessionOut: Struct, filledCount: int };

    /**
     * Presses a keyboard key (Enter, Tab, Escape, etc.)
     * @node browser_press_key @alias browserPressKey
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element (optional, press on active element if empty)
     * @param key (optional) — Key to press
     * @param modifiers (optional) — Control, Shift, Alt, or Meta
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function pressKey({ session: Struct, selector?: string, key?: string, modifiers?: string[], locator?: Struct }): Struct;

    /**
     * Selects an option in a dropdown/select element
     * @node browser_select_option @alias browserSelectOption
     * @param session — Automation session
     * @param selector (optional) — CSS selector of select element
     * @param value (optional) — Option value to select
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function selectOption({ session: Struct, selector?: string, value?: string, locator?: Struct }): Struct;

    /**
     * Types a password or other secret into a field without logging it. The field must become visible and enabled within the timeout.
     * @node browser_type_secret @alias browserTypeSecret
     * @param session — Automation session
     * @param selector (optional) — CSS selector of the field, or a snapshot ref such as e12
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param secret (optional) — Value to type; never written to logs
     * @param clear (optional) — Clear the field before typing
     * @param submit (optional) — Press Enter after typing
     * @param timeoutMs (optional) — Maximum wait for the field to become visible and enabled
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function typeSecret({ session: Struct, selector?: string, locator?: Struct, secret?: string, clear?: bool, submit?: bool, timeoutMs?: int }): Struct;

    /**
     * Types text into an element matching the selector
     * @node browser_type_text @alias browserTypeText
     * @param session — Automation session
     * @param selector (optional) — CSS selector of input element
     * @param text (optional) — Text to type into the element
     * @param clearFirst (optional) — Clear existing text before typing
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function typeText({ session: Struct, selector?: string, text?: string, clearFirst?: bool, locator?: Struct }): Struct;

    // === Automation/Browser/Interact ===

    /**
     * Clicks on an element matching the selector
     * @node browser_click @alias browserClick
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element to click
     * @param button (optional) — left, middle, or right
     * @param modifiers (optional) — Control, Shift, Alt, or Meta
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function click({ session: Struct, selector?: string, button?: string, modifiers?: string[], locator?: Struct }): Struct;

    /**
     * Clicks at viewport CSS coordinates instead of an element, for canvas apps and coordinates read off a screenshot by a vision model. Works in every WebDriver browser.
     * @node browser_click_at_point @alias browserClickAtPoint
     * @param session — Automation session
     * @param x (optional) — Horizontal position in viewport CSS pixels
     * @param y (optional) — Vertical position in viewport CSS pixels
     * @param button (optional) — Mouse button to press
     * @param clickCount (optional) — 1 for a click, 2 for a double click, 3 for a triple click
     * @param modifiers (optional) — Keys held during the click: Control, Shift, Alt, or Meta
     * @returns sessionOut — Updated automation session
     * @impure has side effects / drives control flow
     */
    function clickAtPoint({ session: Struct, x?: float, y?: float, button?: string, clickCount?: int, modifiers?: string[] }): Struct;

    /**
     * Double-clicks on an element matching the selector
     * @node browser_double_click @alias browserDoubleClick
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element to double-click
     * @param button (optional) — left, middle, or right
     * @param modifiers (optional) — Control, Shift, Alt, or Meta
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function doubleClick({ session: Struct, selector?: string, button?: string, modifiers?: string[], locator?: Struct }): Struct;

    /**
     * Hovers over an element matching the selector
     * @node browser_hover @alias browserHover
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element to hover
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function hover({ session: Struct, selector?: string, locator?: Struct }): Struct;

    /**
     * Scrolls element into the visible area
     * @node browser_scroll_into_view @alias browserScrollIntoView
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element to scroll into view
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function scrollIntoView({ session: Struct, selector?: string, locator?: Struct }): Struct;

    /**
     * Scrolls the page, or a scrollable container, by a distance or to its top or bottom, and reports where it ended up. Use At Bottom to stop infinite-scroll loops.
     * @node browser_scroll_page @alias browserScrollPage
     * @param session — Automation session
     * @param mode (optional) — by scrolls by Delta X/Y; top and bottom jump to the vertical ends
     * @param deltaX (optional) — Horizontal distance in CSS pixels for mode 'by'; negative scrolls left
     * @param deltaY (optional) — Vertical distance in CSS pixels for mode 'by'; negative scrolls up
     * @param scope (optional) — CSS selector of a scrollable container; empty scrolls the page
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Updated automation session
     * @returns scrollX — Horizontal scroll offset after scrolling, in CSS pixels
     * @returns scrollY — Vertical scroll offset after scrolling, in CSS pixels
     * @returns atBottom — True when the page or container cannot scroll further down
     * @impure has side effects / drives control flow
     */
    function scrollPage({ session: Struct, mode?: string, deltaX?: float, deltaY?: float, scope?: string, locator?: Struct }): { sessionOut: Struct, scrollX: int, scrollY: int, atBottom: bool };

    // === Automation/Browser/Navigation ===

    /**
     * Navigates back in browser history
     * @node browser_back @alias browserBack
     * @param session — Automation session
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function back({ session: Struct }): Struct;

    /**
     * Navigates forward in browser history
     * @node browser_forward @alias browserForward
     * @param session — Automation session
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function forward({ session: Struct }): Struct;

    /**
     * Navigates the page to a URL. file:, javascript:, chrome:, edge:, devtools: and view-source: URLs are blocked unless Set Navigation Policy allows them; a session navigation policy is enforced before loading and again on the final URL after redirects.
     * @node browser_goto @alias browserGoto
     * @param session — Automation session
     * @param url (optional) — URL to navigate to
     * @returns sessionOut — Automation session (pass-through)
     * @returns finalUrl — The actual URL after navigation (may differ due to redirects)
     * @impure has side effects / drives control flow
     */
    function goto({ session: Struct, url?: string }): { sessionOut: Struct, finalUrl: string };

    /**
     * Reloads the current page
     * @node browser_reload @alias browserReload
     * @param session — Automation session
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function reload({ session: Struct }): Struct;

    /**
     * Restricts where Go To and Execute Browser Action Plan may navigate in this session: URL schemes, domain allow and block lists, and private network addresses. The final URL after redirects is checked too; a blocked landing page is left for about:blank and the node fails. Requests the page makes on its own are not filtered.
     * @node browser_set_navigation_policy @alias browserSetNavigationPolicy
     * @param session — Automation session
     * @param enabled (optional) — Disable to remove the policy and restore the default rules (privileged schemes such as file: and javascript: stay blocked)
     * @param allowedSchemes (optional) — URL schemes that may be opened; empty allows all except file, javascript, chrome, edge, devtools and view-source
     * @param allowedDomains (optional) — Host globs such as example.com or *.example.com; empty allows any host
     * @param blockedDomains (optional) — Host globs that are always blocked
     * @param blockPrivateNetworks (optional) — Block loopback, private, link-local and cloud metadata addresses (hosts are resolved; unresolvable hosts are blocked)
     * @returns sessionOut — Updated automation session
     * @returns policy — The policy now in effect
     * @impure has side effects / drives control flow
     */
    function setNavigationPolicy({ session: Struct, enabled?: bool, allowedSchemes?: string[], allowedDomains?: string[], blockedDomains?: string[], blockPrivateNetworks?: bool }): { sessionOut: Struct, policy: Struct };

    // === Automation/Browser/Observe ===

    /**
     * Clears the captured console log buffer
     * @node browser_clear_console_logs @alias browserClearConsoleLogs
     * @param session — Automation session
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function clearConsoleLogs({ session: Struct }): Struct;

    /**
     * Retrieves captured console messages, uncaught page exceptions, and browser log entries such as failed resource loads. Requires a running console or network observer.
     * @node browser_get_console_logs @alias browserGetConsoleLogs
     * @param session — Automation session
     * @param levelFilter (optional) — Filter by log level (empty for all)
     * @returns sessionOut — Automation session (pass-through)
     * @returns logs — Array of console messages
     * @returns count — Number of log entries
     * @returns hasErrors — Whether any returned entry is an error, including uncaught exceptions and failed resource loads
     * @impure has side effects / drives control flow
     */
    function getConsoleLogs({ session: Struct, levelFilter?: string }): { sessionOut: Struct, logs: Struct[], count: int, hasErrors: bool };

    /**
     * Retrieves captured network requests from the observer
     * @node browser_get_network_requests @alias browserGetNetworkRequests
     * @param session — Automation session
     * @param clearAfter (optional) — Clear the request buffer after retrieval
     * @returns sessionOut — Automation session (pass-through)
     * @returns requests — Array of captured network requests
     * @returns count — Number of captured requests
     * @impure has side effects / drives control flow
     */
    function getNetworkRequests({ session: Struct, clearAfter?: bool }): { sessionOut: Struct, requests: Struct[], count: int };

    /**
     * Starts observing network requests and console output (including uncaught exceptions) through the Chrome DevTools Protocol. Chrome and Edge only.
     * @node browser_start_network_observer @alias browserStartNetworkObserver
     * @param session — Automation session
     * @param urlPattern (optional) — Filter requests by URL pattern (empty for all)
     * @param debuggerAddress (optional) — Optional Chrome or Edge debugger address; defaults to the attached browser
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function startNetworkObserver({ session: Struct, urlPattern?: string, debuggerAddress?: string }): Struct;

    /**
     * Waits until no network requests are in progress for a specified duration
     * @node browser_wait_for_network_idle @alias browserWaitForNetworkIdle
     * @param session — Automation session
     * @param idleTimeMs (optional) — How long network must be idle before continuing
     * @param timeoutMs (optional) — Maximum time to wait for network idle
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function waitForNetworkIdle({ session: Struct, idleTimeMs?: int, timeoutMs?: int }): Struct;

    // === Automation/Browser/Snapshot ===

    /**
     * Searches the latest browser snapshot by role, accessible name and text and returns matching refs. Takes a fresh full snapshot when none exists, the page navigated, or Refresh is set. Requires Chrome or Edge.
     * @node browser_find_elements @alias browserFindElements
     * @param session — Automation session
     * @param role (optional) — ARIA role such as button, link or textbox (empty matches any role)
     * @param name (optional) — Accessible name to match (empty matches any name)
     * @param nameMatch (optional) — How Name is compared: case-insensitive contains, exact, or regex
     * @param text (optional) — Case-insensitive text contained in the name, value, text or description
     * @param refresh (optional) — Always take a fresh full snapshot before searching
     * @param limit (optional) — Maximum matches returned
     * @returns sessionOut — Updated automation session
     * @returns refs — Matching refs
     * @returns lines — Snapshot line of each match
     * @returns elements — Matching snapshot elements
     * @returns count — Number of matches before the limit
     * @impure has side effects / drives control flow
     */
    function findElements({ session: Struct, role?: string, name?: string, nameMatch?: string, text?: string, refresh?: bool, limit?: int }): { sessionOut: Struct, refs: string[], lines: string[], elements: Struct[], count: int };

    /**
     * Captures the accessibility tree of the current page for screen reader analysis
     * @node browser_get_accessibility_snapshot @alias browserGetAccessibilitySnapshot
     * @param session — Automation session
     * @param maxDepth (optional) — Maximum tree depth (-1 for unlimited)
     * @param includeHidden (optional) — Include hidden elements in the tree
     * @returns sessionOut — Automation session (pass-through)
     * @returns tree — Accessibility tree root node
     * @returns treeJson — Accessibility tree as JSON string for LLM processing
     * @impure has side effects / drives control flow
     */
    function getAccessibilitySnapshot({ session: Struct, maxDepth?: int, includeHidden?: bool }): { sessionOut: Struct, tree: Struct, treeJson: string };

    /**
     * Captures the current DOM state including HTML, title, URL, and viewport info
     * @node browser_get_dom_snapshot @alias browserGetDomSnapshot
     * @param session — Automation session
     * @param includeStyles (optional) — Include computed styles (increases snapshot size)
     * @returns sessionOut — Automation session (pass-through)
     * @returns snapshot — DOM snapshot data
     * @returns html — Page HTML content
     * @returns title — Page title
     * @returns url — Current page URL
     * @impure has side effects / drives control flow
     */
    function getDomSnapshot({ session: Struct, includeStyles?: bool }): { sessionOut: Struct, snapshot: Struct, html: string, title: string, url: string };

    /**
     * Gets detailed information about a specific element by selector
     * @node browser_get_element_snapshot @alias browserGetElementSnapshot
     * @param session — Automation session
     * @param selector (optional) — CSS selector of element
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
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
    function getElementSnapshot({ session: Struct, selector?: string, locator?: Struct }): { sessionOut: Struct, html: string, text: string, tag: string, x: int, y: int, width: int, height: int, visible: bool };

    /**
     * Captures the page accessibility tree as compact text with element refs (e1, e2, …) that any selector pin accepts as a Ref selector. Refs stay valid until the page navigates or the element is removed; a stale ref fails with a request to take a new snapshot. Covers the current tab including same-process iframes; out-of-process (cross-site) iframes are omitted. Requires Chrome or Edge.
     * @node browser_snapshot @alias browserSnapshot
     * @param session — Automation session
     * @param interactiveOnly (optional) — List only interactive elements (buttons, links, inputs, clickable elements) as a flat list; disable for the full page structure with text
     * @param maxDepth (optional) — Maximum nesting depth of listed elements (-1 for unlimited)
     * @param scopeRef (optional) — Optional ref from an earlier snapshot of this page; only its subtree is captured
     * @param maxChars (optional) — Maximum length of the snapshot text; longer snapshots are cut and flagged as truncated
     * @param detectClickable (optional) — Also list elements without an interactive role that have a pointer cursor, onclick or tabindex
     * @returns sessionOut — Updated automation session
     * @returns snapshot — Indented text such as `- button "Sign in" [ref=e12]`
     * @returns elements — Listed elements in document order
     * @returns url — Page URL
     * @returns title — Page title
     * @returns generation — Snapshot counter for this session
     * @returns truncated — Whether the snapshot exceeded Max Characters
     * @impure has side effects / drives control flow
     */
    function snapshot({ session: Struct, interactiveOnly?: bool, maxDepth?: int, scopeRef?: string, maxChars?: int, detectClickable?: bool }): { sessionOut: Struct, snapshot: string, elements: Struct[], url: string, title: string, generation: int, truncated: bool };

    // === Automation/Browser/Storage ===

    /**
     * Clears localStorage and/or sessionStorage
     * @node browser_clear_storage @alias browserClearStorage
     * @param session — Automation session
     * @param clearLocal (optional) — Clear localStorage
     * @param clearSession (optional) — Clear sessionStorage
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function clearStorage({ session: Struct, clearLocal?: bool, clearSession?: bool }): Struct;

    /**
     * Gets all key-value pairs from localStorage or sessionStorage
     * @node browser_get_all_storage @alias browserGetAllStorage
     * @param session — Automation session
     * @param storageType (optional) — Which storage to retrieve
     * @returns sessionOut — Automation session (pass-through)
     * @returns data — All storage data as JSON object
     * @returns count — Number of items in storage
     * @impure has side effects / drives control flow
     */
    function getAllStorage({ session: Struct, storageType?: string }): { sessionOut: Struct, data: Struct, count: int };

    /**
     * Gets a value from browser localStorage
     * @node browser_get_local_storage @alias browserGetLocalStorage
     * @param session — Automation session
     * @param key (optional) — Storage key to retrieve
     * @returns sessionOut — Automation session (pass-through)
     * @returns value — Storage value (null if not found)
     * @returns exists — Whether the key exists
     * @impure has side effects / drives control flow
     */
    function getLocalStorage({ session: Struct, key?: string }): { sessionOut: Struct, value: string, exists: bool };

    /**
     * Gets a value from browser sessionStorage
     * @node browser_get_session_storage @alias browserGetSessionStorage
     * @param session — Automation session
     * @param key (optional) — Storage key to retrieve
     * @returns sessionOut — Automation session (pass-through)
     * @returns value — Storage value (null if not found)
     * @returns exists — Whether the key exists
     * @impure has side effects / drives control flow
     */
    function getSessionStorage({ session: Struct, key?: string }): { sessionOut: Struct, value: string, exists: bool };

    /**
     * Sets a value in browser localStorage
     * @node browser_set_local_storage @alias browserSetLocalStorage
     * @param session — Automation session
     * @param key (optional) — Storage key
     * @param value (optional) — Value to store
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function setLocalStorage({ session: Struct, key?: string, value?: string }): Struct;

    /**
     * Sets a value in browser sessionStorage
     * @node browser_set_session_storage @alias browserSetSessionStorage
     * @param session — Automation session
     * @param key (optional) — Storage key
     * @param value (optional) — Value to store
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function setSessionStorage({ session: Struct, key?: string, value?: string }): Struct;

    // === Automation/Browser/Wait ===

    /**
     * Waits for a specified amount of time
     * @node browser_wait_delay @alias browserWaitDelay
     * @param session — Automation session
     * @param delayMs (optional) — Time to wait in milliseconds
     * @returns sessionOut — Automation session (pass-through)
     * @impure has side effects / drives control flow
     */
    function waitDelay({ session: Struct, delayMs?: int }): Struct;

    /**
     * Waits for an element matching the selector to appear in the DOM
     * @node browser_wait_for @alias browserWaitFor
     * @param session — Automation session
     * @param selector (optional) — CSS selector to wait for
     * @param timeoutMs (optional) — Maximum time to wait
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @returns sessionOut — Automation session (pass-through)
     * @returns found — Whether the element was found within timeout
     * @impure has side effects / drives control flow
     */
    function waitFor({ session: Struct, selector?: string, timeoutMs?: int, locator?: Struct }): { sessionOut: Struct, found: bool };

    /**
     * Polls the current page until a condition holds, then continues on Met, or on Timeout when the deadline passes. Conditions: text_visible / text_gone (rendered page text contains Value), url_matches (glob with * and ?, or re:<regex>), title_contains, load_state (domcontentloaded, load, or networkidle — networkidle needs a running Network Observer), js_truthy (Value is a JavaScript function body whose return value is tested, e.g. return window.appReady === true), element_visible / element_hidden (CSS selector in Value, or a connected Locator).
     * @node browser_wait_for_condition @alias browserWaitForCondition
     * @param session — Automation session
     * @param condition (optional) — What to wait for
     * @param value (optional) — Text, URL pattern, title part, load state, JavaScript function body, or CSS selector, depending on Condition
     * @param locator (optional) — Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.
     * @param timeoutMs (optional) — Maximum time to wait
     * @param pollMs (optional) — Pause between checks, at least 10
     * @returns sessionOut — Updated automation session
     * @returns met — Whether the condition held before the deadline
     * @returns elapsedMs — Time spent waiting
     * @impure has side effects / drives control flow
     */
    function waitForCondition({ session: Struct, condition?: string, value?: string, locator?: Struct, timeoutMs?: int, pollMs?: int }): { sessionOut: Struct, met: bool, elapsedMs: int };
}

declare namespace computer {
    // === Automation/Computer/Accessibility ===

    /**
     * Invokes, focuses, selects, expands, collapses, or edits a native accessible element
     * @node computer_accessibility_action @alias computerAccessibilityAction
     * @param session — Active automation session
     * @param element — Native element returned by Find Accessibility Element or the elements list of Get Accessibility Tree
     * @param action (optional) — Native action
     * @param value (optional) — Value to write for set_value
     * @returns sessionOut — Session
     * @impure has side effects / drives control flow
     */
    function accessibilityAction({ session: Struct, element: Struct, action?: string, value?: string }): Struct;

    /**
     * Finds an element in the accessibility tree by role, name, or other attributes
     * @node computer_find_accessibility_element @alias computerFindAccessibilityElement
     * @param session — Computer session handle
     * @param windowTitle (optional) — Window title, or empty for the active window
     * @param role (optional) — Accessibility role to match
     * @param name (optional) — Element name to match (partial match)
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns element — Found accessibility element
     * @returns x — Element center X coordinate
     * @returns y — Element center Y coordinate
     * @impure has side effects / drives control flow
     */
    function findAccessibilityElement({ session: Struct, windowTitle?: string, role?: string, name?: string }): { sessionOut: Struct, element: Struct, x: int, y: int };

    /**
     * Retrieves the accessibility tree for a window (requires platform-specific accessibility APIs)
     * @node computer_get_accessibility_tree @alias computerGetAccessibilityTree
     * @param session — Computer session handle
     * @param windowTitle (optional) — Window title; empty targets the focused window outside Flow-Like (on Linux the desktop accessibility root when no window resolves)
     * @param maxDepth (optional) — Maximum tree depth (1–32); results are limited to 2000 elements
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns tree — Accessibility tree root node
     * @returns treeJson — Accessibility tree as JSON string for LLM processing
     * @returns elements — Every element of the tree as a flat list in document order (children omitted); each can be passed to Act on Accessibility Element
     * @returns error — Error message if accessibility APIs are unavailable
     * @impure has side effects / drives control flow
     */
    function getAccessibilityTree({ session: Struct, windowTitle?: string, maxDepth?: int }): { sessionOut: Struct, tree: Struct, treeJson: string, elements: Struct[], error: string };

    // === Automation/Computer/Agent ===

    /**
     * Lets a vision model operate the desktop until a goal is reached: it looks at a screenshot (optionally with numbered accessibility marks), calls mouse and keyboard tools, waits for the screen to settle, checks the result and repeats. Works with any vision model that supports tool calling. Ends when the model reports done or asks the user, or when it is stuck or out of steps or time
     * @node computer_use_agent @alias computerUseAgent
     * @param session — Computer session handle
     * @param model — Vision model with tool calling that operates the desktop
     * @param goal (optional) — The task in plain language, e.g. 'Rename report.txt on the Desktop to final.txt'
     * @param displayIndex (optional) — Display the agent sees and acts on; -1 is the primary display
     * @param targetWindow (optional) — Optional window title (or part of it): the agent sees the display showing this window and its elements are the numbered ones. Empty uses the focused window outside Flow-Like
     * @param perception (optional) — screenshot: the plain screenshot; screenshot_marks: numbered boxes on accessibility elements plus an element list; screenshot_marks_ocr: also number recognized text
     * @param maxSteps (optional) — Most model turns before the agent stops with max_steps (1–500)
     * @param maxDurationS (optional) — Wall-clock budget in seconds before the agent stops with timeout (1–86400)
     * @param maxActionsPerStep (optional) — Most tool calls executed per model turn (1–20)
     * @param settleMs (optional) — Longest wait after actions for the screen to stop changing before the next screenshot (0–10000)
     * @param forbiddenText (optional) — Regular expressions the agent may never type; a matching type call is refused
     * @param extraInstructions (optional) — Optional guidance added to the task, e.g. which app to use or what to avoid
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns status — done, failed, stuck, max_steps, timeout or needs_input
     * @returns answer — The model's result or summary, the question for the user, or why the agent stopped
     * @returns steps — Trajectory: per turn the screenshot seen, the model's text, each action with its arguments, desktop coordinates and result, and the duration
     * @returns finalImage — The last screenshot, without marks
     * @returns finalFrame — Desktop rectangle and pixel size of the final screenshot
     * @impure has side effects / drives control flow
     */
    function useAgent({ session: Struct, model: Struct, goal?: string, displayIndex?: int, targetWindow?: string, perception?: string, maxSteps?: int, maxDurationS?: int, maxActionsPerStep?: int, settleMs?: int, forbiddenText?: string[], extraInstructions?: string }): { sessionOut: Struct, status: string, answer: string, steps: Struct[], finalImage: Struct, finalFrame: Struct };

    // === Automation/Computer/Capture ===

    /**
     * Observes a window or display for an agent: a screenshot, the same screenshot with numbered boxes on every actionable element, and a compact element list (accessibility tree plus optional OCR text) whose ids Click Screen Element accepts
     * @node computer_capture_state @alias computerCaptureState
     * @param session — Computer session handle
     * @param target (optional) — focused_window: the focused window outside Flow-Like; window: the window titled below; display: a whole display
     * @param windowTitle (optional) — Title (or part of it) of the window for the window target
     * @param displayIndex (optional) — Display for the display target
     * @param includeAx (optional) — List elements from the accessibility tree
     * @param includeOcr (optional) — Add recognized text that no accessibility element already covers
     * @param interactiveOnly (optional) — Only list controls that can be clicked, typed into or toggled
     * @param maxElements (optional) — Upper bound on listed elements (1–500)
     * @param languages (optional) — Comma-separated OCR languages such as en-US, de; empty lets the OS engine choose
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns image — Screenshot of the target
     * @returns annotated — Screenshot with a numbered box on every listed element
     * @returns frame — Desktop rectangle and pixel size of both images
     * @returns elements — Listed elements with role, name, value, states and desktop bounds
     * @returns summary — One line per element for a language model, e.g. [3] button "Save" @ (812,433)
     * @returns generation — Identifies this capture; Click Screen Element rejects ids from older captures
     * @impure has side effects / drives control flow
     */
    function captureState({ session: Struct, target?: string, windowTitle?: string, displayIndex?: int, includeAx?: bool, includeOcr?: bool, interactiveOnly?: bool, maxElements?: int, languages?: string }): { sessionOut: Struct, image: Struct, annotated: Struct, frame: Struct, elements: Struct[], summary: string, generation: int };

    /**
     * Takes a screenshot of the primary display, a chosen display, or a region of one display. Frame maps image pixels back to the desktop coordinates the mouse nodes use
     * @node computer_screenshot @alias computerScreenshot
     * @param session — Computer session handle
     * @param captureType (optional) — full: the primary display only. display: the display at Display Index. region: a rectangle of one display
     * @param displayIndex (optional) — Display for display capture and pixel-space region capture: index from List Displays, -1 = primary
     * @param regionSpace (optional) — pixels: region is in screenshot pixels of the display at Display Index (Retina displays have twice as many pixels as desktop units). desktop: region is in desktop input coordinates, like the mouse nodes, and may be on any display
     * @param regionX (optional) — Left edge of the region (capture_type=region), in Region Space units
     * @param regionY (optional) — Top edge of the region, in Region Space units
     * @param regionWidth (optional) — Width of the region, in Region Space units
     * @param regionHeight (optional) — Height of the region, in Region Space units
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns screenshot — Reference to the captured screenshot
     * @returns image — Screenshot as NodeImage
     * @returns frame — Desktop rectangle and pixel size of the screenshot; converts image pixels to mouse coordinates
     * @impure has side effects / drives control flow
     */
    function screenshot({ session: Struct, captureType?: string, displayIndex?: int, regionSpace?: string, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int }): { sessionOut: Struct, screenshot: Struct, image: Struct, frame: Struct };

    /**
     * Captures a desktop rectangle at the display's full pixel density, optionally enlarged, to read small text or inspect details. Coordinates found in the image map back to the desktop through the frame
     * @node computer_zoom @alias computerZoom
     * @param session — Computer session handle
     * @param x (optional) — Left edge in desktop input coordinates
     * @param y (optional) — Top edge in desktop input coordinates
     * @param width (optional) — Width in desktop input coordinates
     * @param height (optional) — Height in desktop input coordinates
     * @param upscale (optional) — Enlarge the capture by this factor (1–4) for tiny text
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns image — The captured region
     * @returns frame — Desktop rectangle and pixel size of the image; map image points back with it
     * @impure has side effects / drives control flow
     */
    function zoom({ session: Struct, x?: int, y?: int, width?: int, height?: int, upscale?: int }): { sessionOut: Struct, image: Struct, frame: Struct };

    // === Automation/Computer/Clipboard ===

    /**
     * Gets an image from the system clipboard if available
     * @node computer_clipboard_get_image @alias computerClipboardGetImage
     * @param session — Computer session handle
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns image — Image from clipboard as NodeImage
     * @returns hasImage — Whether the clipboard contains an image
     * @impure has side effects / drives control flow
     */
    function clipboardGetImage({ session: Struct }): { sessionOut: Struct, image: Struct, hasImage: bool };

    /**
     * Gets the current text content from the system clipboard
     * @node computer_clipboard_get_text @alias computerClipboardGetText
     * @param session — Computer session handle
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns text — Text content from clipboard
     * @returns hasText — Whether the clipboard contains text
     * @impure has side effects / drives control flow
     */
    function clipboardGetText({ session: Struct }): { sessionOut: Struct, text: string, hasText: bool };

    /**
     * Writes to the local device clipboard, or to the calling frontend when this Event runs remotely
     * @node computer_clipboard_set_image @alias computerClipboardSetImage
     * @param session (optional) — Optional automation session, passed through for existing flows
     * @param image — Image to copy
     * @param localOnly (optional) — Prevent cross-device clipboard sharing on supported platforms
     * @param expiresInSeconds (optional) — Seconds before content expires; zero leaves it on the clipboard. Requires platform support
     * @param timeoutSeconds (optional) — Maximum time to wait for the invoking client
     * @returns sessionOut — Optional automation session
     * @returns error — Structured error code and message
     * @impure has side effects / drives control flow
     */
    function clipboardSetImage({ session?: Struct, image: Struct, localOnly?: bool, expiresInSeconds?: int, timeoutSeconds?: int }): { sessionOut: Struct, error: Struct };

    /**
     * Writes to the local device clipboard, or to the calling frontend when this Event runs remotely
     * @node computer_clipboard_set_text @alias computerClipboardSetText
     * @param session (optional) — Optional automation session, passed through for existing flows
     * @param text — Text to copy
     * @param html (optional) — Optional rich text, with Text as the plain-text fallback
     * @param localOnly (optional) — Prevent cross-device clipboard sharing on supported platforms
     * @param expiresInSeconds (optional) — Seconds before content expires; zero leaves it on the clipboard. Requires platform support
     * @param timeoutSeconds (optional) — Maximum time to wait for the invoking client
     * @returns sessionOut — Optional automation session
     * @returns error — Structured error code and message
     * @impure has side effects / drives control flow
     */
    function clipboardSetText({ session?: Struct, text: string, html?: string, localOnly?: bool, expiresInSeconds?: int, timeoutSeconds?: int }): { sessionOut: Struct, error: Struct };

    // === Automation/Computer/Display ===

    /**
     * Gets information about a specific display by index
     * @node computer_get_display @alias computerGetDisplay
     * @param session — Computer session handle
     * @param index (optional) — Display index (0-based)
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns display — Display information
     * @returns width — Display width in desktop input coordinates
     * @returns height — Display height in desktop input coordinates
     * @impure has side effects / drives control flow
     */
    function getDisplay({ session: Struct, index?: int }): { sessionOut: Struct, display: Struct, width: int, height: int };

    /**
     * Gets information about the primary display
     * @node computer_get_primary_display @alias computerGetPrimaryDisplay
     * @param session — Computer session handle
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns display — Primary display information
     * @returns width — Display width in desktop input coordinates
     * @returns height — Display height in desktop input coordinates
     * @impure has side effects / drives control flow
     */
    function getPrimaryDisplay({ session: Struct }): { sessionOut: Struct, display: Struct, width: int, height: int };

    /**
     * Enumerates all connected monitors/displays
     * @node computer_list_displays @alias computerListDisplays
     * @param session — Computer session handle
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns displays — List of connected displays
     * @returns count — Number of connected displays
     * @returns primaryIndex — Index of the primary display, or -1 if none is designated
     * @impure has side effects / drives control flow
     */
    function listDisplays({ session: Struct }): { sessionOut: Struct, displays: Struct[], count: int, primaryIndex: int };

    // === Automation/Computer/Keyboard ===

    /**
     * Holds a key down for a duration and then releases it. The key is released even when the run fails or is cancelled
     * @node computer_hold_key @alias computerHoldKey
     * @param session — Computer session handle
     * @param key (optional) — Key to hold (case-insensitive): a single character or a named key: Enter, Tab, Escape, Backspace, Delete, Space, arrows (Up/Down/Left/Right), Home, End, PageUp, PageDown, Insert, CapsLock, NumLock, ScrollLock, PrintScreen, Pause, Help, F1–F24, Shift, Ctrl, Alt, Cmd/Meta/Super/Win, Numpad0–Numpad9, NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal, VolumeUp, VolumeDown, VolumeMute, MediaPlayPause, MediaNext, MediaPrev. Keys the operating system cannot send fail with an error
     * @param durationMs (optional) — How long to hold the key (0-60000 ms)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function holdKey({ session: Struct, key?: string, durationMs?: int }): Struct;

    /**
     * Presses a keyboard shortcut written as text, such as ctrl+shift+s, cmd+space or alt+F4. Modifiers are released in reverse order even when a key fails
     * @node computer_key_chord @alias computerKeyChord
     * @param session — Computer session handle
     * @param chord (optional) — Modifiers joined with '+' followed by one key, e.g. ctrl+shift+s, cmd+space, alt+F4, ctrl++. Modifiers: ctrl, shift, alt/option, cmd/meta/super/win, primary (cmd on macOS, ctrl elsewhere). Key: a single character or a named key: Enter, Tab, Escape, Backspace, Delete, Space, arrows (Up/Down/Left/Right), Home, End, PageUp, PageDown, Insert, CapsLock, NumLock, ScrollLock, PrintScreen, Pause, Help, F1–F24, Shift, Ctrl, Alt, Cmd/Meta/Super/Win, Numpad0–Numpad9, NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal, VolumeUp, VolumeDown, VolumeMute, MediaPlayPause, MediaNext, MediaPrev. Keys the operating system cannot send fail with an error
     * @param repeat (optional) — How many times to press the chord (1-100)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function keyChord({ session: Struct, chord?: string, repeat?: int }): Struct;

    /**
     * Presses a keyboard key or key combination
     * @node computer_key_press @alias computerKeyPress
     * @param session — Computer session handle
     * @param key (optional) — Key to press (case-insensitive): a single character or a named key: Enter, Tab, Escape, Backspace, Delete, Space, arrows (Up/Down/Left/Right), Home, End, PageUp, PageDown, Insert, CapsLock, NumLock, ScrollLock, PrintScreen, Pause, Help, F1–F24, Shift, Ctrl, Alt, Cmd/Meta/Super/Win, Numpad0–Numpad9, NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal, VolumeUp, VolumeDown, VolumeMute, MediaPlayPause, MediaNext, MediaPrev. Keys the operating system cannot send fail with an error
     * @param modifiers (optional) — Modifier keys to hold (comma-separated: ctrl,shift,alt,meta)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function keyPress({ session: Struct, key?: string, modifiers?: string }): Struct;

    /**
     * Enters a password or other secret into the focused field without logging it. Type mode sends keystrokes; paste mode puts the secret on the clipboard (excluded from clipboard history where supported), presses the paste shortcut and restores the previous text or image clipboard afterwards
     * @node computer_type_secret @alias computerTypeSecret
     * @param session — Computer session handle
     * @param secret — Secret to enter; connect it from a secret or variable so it is not stored in the board
     * @param mode (optional) — type: send keystrokes. paste: paste through the clipboard, then restore it (use for fields that drop fast keystrokes)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function typeSecret({ session: Struct, secret: string, mode?: string }): Struct;

    /**
     * Types text using the keyboard. The text is stored in the board; use Type Secret for passwords
     * @node computer_key_type @alias computerKeyType
     * @param session — Computer session handle
     * @param text (optional) — Text to type
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function typeText({ session: Struct, text?: string }): Struct;

    // === Automation/Computer/Mouse ===

    /**
     * Clicks an element by its id from the latest Capture Screen State of this session, through the accessibility action when possible, otherwise with the mouse at its center
     * @node computer_click_element @alias computerClickElement
     * @param session — Computer session handle
     * @param elementId (optional) — The [id] from the screen state
     * @param generation (optional) — Generation of the screen state the id comes from; 0 accepts the latest capture
     * @param button (optional) — Mouse button
     * @param double (optional) — Click twice
     * @param preferAccessibilityAction (optional) — Press accessibility elements through the accessibility API (works when covered or off-focus) for single left clicks; falls back to the mouse
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns element — The clicked element
     * @returns method — accessibility or mouse
     * @impure has side effects / drives control flow
     */
    function clickElement({ session: Struct, elementId?: int, generation?: int, button?: string, double?: bool, preferAccessibilityAction?: bool }): { sessionOut: Struct, element: Struct, method: string };

    /**
     * Reads the mouse pointer position in desktop input coordinates and the display it is on. Not available on Wayland, which does not expose the global pointer
     * @node computer_cursor_position @alias computerCursorPosition
     * @param session — Computer session handle
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns x — Pointer X in desktop input coordinates
     * @returns y — Pointer Y in desktop input coordinates
     * @returns displayIndex — Index (as in List Displays) of the display under the pointer, or -1 if none
     * @returns frame — Frame of that display (input rectangle and pixel size), or null
     * @impure has side effects / drives control flow
     */
    function cursorPosition({ session: Struct }): { sessionOut: Struct, x: int, y: int, displayIndex: int, frame: Struct };

    /**
     * Clicks the mouse at the specified coordinates
     * @node computer_mouse_click @alias computerMouseClick
     * @param session — Computer session handle
     * @param x (optional) — X coordinate (horizontal position)
     * @param y (optional) — Y coordinate (vertical position)
     * @param button (optional) — Mouse button to click
     * @param useTemplateMatching (optional) — If enabled, use template matching to find the click target from a recorded screenshot
     * @param template — Template image for template matching
     * @param confidence (optional) — Minimum confidence threshold for template matching (0.0-1.0)
     * @param naturalMove (optional) — Use curved, human-like mouse movement to avoid bot detection
     * @param moveDurationMs (optional) — Duration of natural mouse movement in milliseconds
     * @param useFingerprint (optional) — Resolve a unique accessible element from the current desktop before clicking
     * @param fingerprint — Optional element fingerprint for pre-click validation
     * @param modifiers (optional) — Comma-separated ctrl, shift, alt, meta
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseClick({ session: Struct, x?: int, y?: int, button?: string, useTemplateMatching?: bool, template: Struct, confidence?: float, naturalMove?: bool, moveDurationMs?: int, useFingerprint?: bool, fingerprint: Struct, modifiers?: string }): Struct;

    /**
     * Double-clicks the mouse at the specified coordinates
     * @node computer_mouse_double_click @alias computerMouseDoubleClick
     * @param session — Computer session handle
     * @param x (optional) — X coordinate
     * @param y (optional) — Y coordinate
     * @param useTemplateMatching (optional) — If enabled, use template matching to find the click target from a recorded screenshot
     * @param template — Template image for template matching
     * @param confidence (optional) — Minimum confidence threshold for template matching (0.0-1.0)
     * @param naturalMove (optional) — Use curved, human-like mouse movement to avoid bot detection
     * @param moveDurationMs (optional) — Duration of natural mouse movement in milliseconds
     * @param useFingerprint (optional) — Resolve a unique accessible element from the current desktop before clicking
     * @param fingerprint — Optional element fingerprint for pre-click validation
     * @param button (optional) — Mouse button
     * @param modifiers (optional) — Comma-separated ctrl, shift, alt, meta
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseDoubleClick({ session: Struct, x?: int, y?: int, useTemplateMatching?: bool, template: Struct, confidence?: float, naturalMove?: bool, moveDurationMs?: int, useFingerprint?: bool, fingerprint: Struct, button?: string, modifiers?: string }): Struct;

    /**
     * Presses and keeps a mouse button down, optionally after moving to X/Y. Release it with Mouse Up; it is released automatically if the session closes or the run is cancelled
     * @node computer_mouse_down @alias computerMouseDown
     * @param session — Computer session handle
     * @param button (optional) — Mouse button to press
     * @param x (optional) — Optional desktop coordinate to move to first; leave both X and Y empty to use the current pointer position
     * @param y (optional) — Optional desktop coordinate to move to first; leave both X and Y empty to use the current pointer position
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseDown({ session: Struct, button?: string, x?: int, y?: int }): Struct;

    /**
     * Presses the button at the start point, moves to the end point in small steps over the duration, pauses, and releases so applications register a real drag
     * @node computer_mouse_drag @alias computerMouseDrag
     * @param session — Computer session handle
     * @param fromX (optional) — Starting X coordinate
     * @param fromY (optional) — Starting Y coordinate
     * @param toX (optional) — Ending X coordinate
     * @param toY (optional) — Ending Y coordinate
     * @param button (optional) — Mouse button to use for dragging
     * @param modifiers (optional) — Comma-separated ctrl, shift, alt, meta
     * @param durationMs (optional) — Time spent moving from start to end (0-60000 ms); the pointer moves in steps of about 16 ms
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseDrag({ session: Struct, fromX?: int, fromY?: int, toX?: int, toY?: int, button?: string, modifiers?: string, durationMs?: int }): Struct;

    /**
     * Moves the mouse cursor to the specified screen coordinates
     * @node computer_mouse_move @alias computerMouseMove
     * @param session — Computer session handle
     * @param x (optional) — X coordinate (horizontal position)
     * @param y (optional) — Y coordinate (vertical position)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseMove({ session: Struct, x?: int, y?: int }): Struct;

    /**
     * Triple-clicks at desktop coordinates, e.g. to select a whole line or paragraph of text
     * @node computer_mouse_triple_click @alias computerMouseTripleClick
     * @param session — Computer session handle
     * @param x (optional) — Desktop X coordinate
     * @param y (optional) — Desktop Y coordinate
     * @param button (optional) — Mouse button to click
     * @param modifiers (optional) — Comma-separated ctrl, shift, alt, meta held during the clicks
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseTripleClick({ session: Struct, x?: int, y?: int, button?: string, modifiers?: string }): Struct;

    /**
     * Releases a mouse button, optionally after moving to X/Y (for example to finish a drag started with Mouse Down)
     * @node computer_mouse_up @alias computerMouseUp
     * @param session — Computer session handle
     * @param button (optional) — Mouse button to release
     * @param x (optional) — Optional desktop coordinate to move to first; leave both X and Y empty to use the current pointer position
     * @param y (optional) — Optional desktop coordinate to move to first; leave both X and Y empty to use the current pointer position
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function mouseUp({ session: Struct, button?: string, x?: int, y?: int }): Struct;

    /**
     * Moves the mouse cursor naturally using curved paths with variable speed to avoid bot detection
     * @node computer_natural_mouse_move @alias computerNaturalMouseMove
     * @param session — Computer session handle
     * @param x (optional) — Target X coordinate
     * @param y (optional) — Target Y coordinate
     * @param durationMs (optional) — Approximate duration of the movement in milliseconds
     * @param curveIntensity (optional) — How curved the path is (0.0 = straight, 1.0 = very curved)
     * @param overshoot (optional) — Whether to slightly overshoot and correct (more human-like)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function naturalMouseMove({ session: Struct, x?: int, y?: int, durationMs?: int, curveIntensity?: float, overshoot?: bool }): Struct;

    /**
     * Scrolls the mouse wheel at the current pointer position, one wheel tick at a time. Positive Delta Y scrolls down (towards the end of a page); note that RPA Scroll uses the opposite sign
     * @node computer_scroll @alias computerScroll
     * @param session — Computer session handle
     * @param dx (optional) — Horizontal wheel ticks: positive scrolls right, negative scrolls left (-1000 to 1000)
     * @param dy (optional) — Vertical wheel ticks: positive scrolls DOWN (content moves up), negative scrolls up (-1000 to 1000)
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function scroll({ session: Struct, dx?: int, dy?: int }): Struct;

    // === Automation/Computer/Vision ===

    /**
     * Finds text on screen with OCR and clicks the center of the matched words
     * @node computer_click_text @alias computerClickText
     * @param session — Computer session handle
     * @param text (optional) — Text or pattern to find
     * @param matchMode (optional) — exact: whole words equal the text; contains: substring; regex: pattern; fuzzy: similar words
     * @param caseSensitive (optional) — Compare letter case; whitespace and typographic quotes are always normalized
     * @param fuzzyThreshold (optional) — Minimum similarity (0–1) for fuzzy matches
     * @param occurrence (optional) — Which match to use in reading order (1 = first)
     * @param windowTitle (optional) — Look only inside the window with this title; empty uses the region or display
     * @param displayIndex (optional) — Display to capture when no window or region is given
     * @param regionX (optional) — Left edge of the region in desktop input coordinates
     * @param regionY (optional) — Top edge of the region in desktop input coordinates
     * @param regionWidth (optional) — Region width; 0 captures the whole display
     * @param regionHeight (optional) — Region height; 0 captures the whole display
     * @param languages (optional) — Comma-separated languages such as en-US, de; empty lets the OS engine choose
     * @param button (optional) — Mouse button
     * @param double (optional) — Click twice
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns found — Whether the text was found and clicked
     * @returns x — Clicked X in desktop input coordinates
     * @returns y — Clicked Y in desktop input coordinates
     * @returns match — The clicked match
     * @impure has side effects / drives control flow
     */
    function clickText({ session: Struct, text?: string, matchMode?: string, caseSensitive?: bool, fuzzyThreshold?: float, occurrence?: int, windowTitle?: string, displayIndex?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int, languages?: string, button?: string, double?: bool }): { sessionOut: Struct, found: bool, x: int, y: int, match: Struct };

    /**
     * Finds text on a display, region or window with OCR and returns its position in desktop coordinates
     * @node computer_find_text @alias computerFindText
     * @param session — Computer session handle
     * @param text (optional) — Text or pattern to find
     * @param matchMode (optional) — exact: whole words equal the text; contains: substring; regex: pattern; fuzzy: similar words
     * @param caseSensitive (optional) — Compare letter case; whitespace and typographic quotes are always normalized
     * @param fuzzyThreshold (optional) — Minimum similarity (0–1) for fuzzy matches
     * @param occurrence (optional) — Which match to use in reading order (1 = first)
     * @param windowTitle (optional) — Look only inside the window with this title; empty uses the region or display
     * @param displayIndex (optional) — Display to capture when no window or region is given
     * @param regionX (optional) — Left edge of the region in desktop input coordinates
     * @param regionY (optional) — Top edge of the region in desktop input coordinates
     * @param regionWidth (optional) — Region width; 0 captures the whole display
     * @param regionHeight (optional) — Region height; 0 captures the whole display
     * @param languages (optional) — Comma-separated languages such as en-US, de; empty lets the OS engine choose
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns found — Whether the requested occurrence exists
     * @returns x — Center X of the match in desktop input coordinates
     * @returns y — Center Y of the match in desktop input coordinates
     * @returns bbox — Bounds of the match in desktop input coordinates
     * @returns match — The selected match
     * @returns matches — Every match in reading order
     * @returns frame — Screen frame of the searched capture
     * @impure has side effects / drives control flow
     */
    function findText({ session: Struct, text?: string, matchMode?: string, caseSensitive?: bool, fuzzyThreshold?: float, occurrence?: int, windowTitle?: string, displayIndex?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int, languages?: string }): { sessionOut: Struct, found: bool, x: int, y: int, bbox: Struct, match: Struct, matches: Struct[], frame: Struct };

    /**
     * Recognizes text in an image or on the screen with the operating system's OCR engine (Apple Vision, Windows OCR, Tesseract on Linux). Lines carry pixel boxes and, when the screen frame is known, desktop coordinates for the mouse nodes
     * @node computer_ocr @alias computerOcr
     * @param session — Computer session handle
     * @param image (optional) — Image to read; leave unconnected to capture the window, region or display below
     * @param imageFrame (optional) — Screen frame of the connected image; maps recognized boxes to desktop coordinates
     * @param windowTitle (optional) — Look only inside the window with this title; empty uses the region or display
     * @param displayIndex (optional) — Display to capture when no window or region is given
     * @param regionX (optional) — Left edge of the region in desktop input coordinates
     * @param regionY (optional) — Top edge of the region in desktop input coordinates
     * @param regionWidth (optional) — Region width; 0 captures the whole display
     * @param regionHeight (optional) — Region height; 0 captures the whole display
     * @param languages (optional) — Comma-separated languages such as en-US, de; empty lets the OS engine choose
     * @param languageCorrection (optional) — Let the engine correct words with its language model (Apple Vision); turn off for codes and identifiers
     * @param minConfidence (optional) — Drop lines below this confidence (0–1); engines without confidence keep all lines
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns text — All recognized lines in reading order, separated by newlines
     * @returns lines — Recognized lines with confidence, pixel boxes, desktop boxes and word boxes
     * @returns frame — Screen frame of the recognized image; empty when an image without frame was read
     * @impure has side effects / drives control flow
     */
    function ocr({ session: Struct, image?: Struct, imageFrame?: Struct, windowTitle?: string, displayIndex?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int, languages?: string, languageCorrection?: bool, minConfidence?: float }): { sessionOut: Struct, text: string, lines: Struct[], frame: Struct };

    // === Automation/Computer/Wait ===

    /**
     * Waits for the specified number of milliseconds
     * @node computer_wait @alias computerWait
     * @param session — Computer session handle
     * @param ms (optional) — Time to wait in milliseconds
     * @returns sessionOut — Computer session handle (pass-through)
     * @impure has side effects / drives control flow
     */
    function wait({ session: Struct, ms?: int }): Struct;

    /**
     * Verifies that an action had a visible effect: compares a display or desktop region against a baseline until at least Threshold of it has changed, or times out. Connect Baseline to a screenshot of the same area taken before the action to also catch instant changes; otherwise the baseline is captured when this node starts
     * @node computer_wait_screen_change @alias computerWaitScreenChange
     * @param session — Computer session handle
     * @param display (optional) — Display to watch when no region is set: index from List Displays, -1 = primary
     * @param regionX (optional) — Left edge of the watched region in desktop input coordinates
     * @param regionY (optional) — Top edge of the watched region in desktop input coordinates
     * @param regionWidth (optional) — Region width in desktop input coordinates; 0 with height 0 watches the whole display. The region must lie on one display
     * @param regionHeight (optional) — Region height in desktop input coordinates; 0 with width 0 watches the whole display
     * @param baseline (optional) — Optional image of the same area captured before the action
     * @param threshold (optional) — Fraction of pixels (0-1) that must differ from the baseline to count as changed
     * @param pollMs (optional) — Pause between captures (20-60000 ms)
     * @param timeoutMs (optional) — Maximum wait before taking the Timeout branch (0-3600000 ms)
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns changed — Whether the area changed before the timeout
     * @returns changeRatio — Fraction of pixels that differ from the baseline in the last frame
     * @returns before — Baseline image
     * @returns after — Last captured image
     * @returns frame — Desktop rectangle and pixel size of the After image
     * @impure has side effects / drives control flow
     */
    function waitScreenChange({ session: Struct, display?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int, baseline?: Struct, threshold?: float, pollMs?: int, timeoutMs?: int }): { sessionOut: Struct, changed: bool, changeRatio: float, before: Struct, after: Struct, frame: Struct };

    /**
     * Captures a display or desktop region repeatedly until consecutive frames stop changing for Stable (ms), e.g. after a page load or animation. Frames are compared on a downsampled grayscale copy
     * @node computer_wait_screen_stable @alias computerWaitScreenStable
     * @param session — Computer session handle
     * @param display (optional) — Display to watch when no region is set: index from List Displays, -1 = primary
     * @param regionX (optional) — Left edge of the watched region in desktop input coordinates
     * @param regionY (optional) — Top edge of the watched region in desktop input coordinates
     * @param regionWidth (optional) — Region width in desktop input coordinates; 0 with height 0 watches the whole display. The region must lie on one display
     * @param regionHeight (optional) — Region height in desktop input coordinates; 0 with width 0 watches the whole display
     * @param threshold (optional) — Largest fraction of pixels (0-1) that may change between two frames while still counting as stable
     * @param stableMs (optional) — How long the screen must stay unchanged (0-600000 ms)
     * @param pollMs (optional) — Pause between captures (20-60000 ms)
     * @param timeoutMs (optional) — Maximum wait before taking the Timeout branch (0-3600000 ms)
     * @returns sessionOut — Computer session handle (pass-through)
     * @returns image — Last captured frame
     * @returns frame — Desktop rectangle and pixel size of the image
     * @returns stable — Whether the screen became stable before the timeout
     * @returns lastChange — Fraction of pixels that changed between the last two frames
     * @returns waitedMs — Time spent waiting
     * @impure has side effects / drives control flow
     */
    function waitScreenStable({ session: Struct, display?: int, regionX?: int, regionY?: int, regionWidth?: int, regionHeight?: int, threshold?: float, stableMs?: int, pollMs?: int, timeoutMs?: int }): { sessionOut: Struct, image: Struct, frame: Struct, stable: bool, lastChange: float, waitedMs: int };

    // === Automation/Computer/Window ===

    /**
     * Captures a screenshot of a specific window. Frame maps image pixels to the desktop coordinates the mouse nodes use
     * @node computer_capture_window @alias computerCaptureWindow
     * @param session — Computer session handle
     * @param windowId — ID of the window to capture
     * @returns screenshot — Base64-encoded PNG image
     * @returns image — Screenshot as NodeImage
     * @returns frame — The window's desktop rectangle and the image's pixel size, or null when the window reports no geometry
     * @impure has side effects / drives control flow
     */
    function captureWindow({ session: Struct, windowId: string }): { screenshot: string, image: Struct, frame: Struct };

    /**
     * Finds a window by its title (partial match supported)
     * @node computer_find_window_by_title @alias computerFindWindowByTitle
     * @param session — Computer session handle
     * @param title — Window title to search for (partial match)
     * @param exactMatch (optional) — Require exact title match
     * @returns window — Found window information
     * @impure has side effects / drives control flow
     */
    function findWindowByTitle({ session: Struct, title: string, exactMatch?: bool }): Struct;

    /**
     * Brings a window to the front and gives it focus
     * @node computer_focus_window @alias computerFocusWindow
     * @param session — Computer session handle
     * @param windowId (optional) — Native window ID; fails if that window no longer exists
     * @param processName (optional) — Exact application name to disambiguate the window
     * @param windowTitle — Title or app name to search for (partial match on both title and app name)
     * @param exactMatch (optional) — Require exact title match
     * @param launchIfNotFound (optional) — Launch the program named in Application when no window matches (requires Application; the title is never executed)
     * @returns window — Focused window information
     * @impure has side effects / drives control flow
     */
    function focusWindow({ session: Struct, windowId?: string, processName?: string, windowTitle: string, exactMatch?: bool, launchIfNotFound?: bool }): Struct;

    /**
     * Gets the focused window of another application (Flow-Like's own windows are skipped), falling back to the front-most visible window
     * @node computer_get_active_window @alias computerGetActiveWindow
     * @param session — Computer session handle
     * @returns window — Active window information
     * @returns title — Window title
     * @impure has side effects / drives control flow
     */
    function getActiveWindow({ session: Struct }): { window: Struct, title: string };

    /**
     * Launches an application by path or name
     * @node computer_launch_app @alias computerLaunchApp
     * @param session — Computer session handle
     * @param path — Application path or command
     * @param args (optional) — Command line arguments (space-separated)
     * @param arguments (optional) — Arguments passed directly without shell evaluation
     * @param waitMs (optional) — Time to wait after launching (ms)
     * @returns pid — Process ID if available
     * @impure has side effects / drives control flow
     */
    function launchApp({ session: Struct, path: string, args?: string, arguments?: string[], waitMs?: int }): int;

    /**
     * Lists all visible windows on the desktop
     * @node computer_list_windows @alias computerListWindows
     * @param session — Computer session handle
     * @returns windows — List of window information
     * @returns count — Number of windows
     * @impure has side effects / drives control flow
     */
    function listWindows({ session: Struct }): { windows: Struct[], count: int };

    /**
     * Restores, minimizes, maximizes, moves, resizes, or closes a window by native ID
     * @node computer_window_operation @alias computerWindowOperation
     * @param session — Active session
     * @param windowId — ID returned by List Windows
     * @param operation (optional) — Native window operation
     * @param x (optional) — Desktop coordinates for move_resize
     * @param y (optional) — Desktop coordinates for move_resize
     * @param width (optional) — Desktop coordinates for move_resize
     * @param height (optional) — Desktop coordinates for move_resize
     * @returns sessionOut — Session
     * @impure has side effects / drives control flow
     */
    function manageWindow({ session: Struct, windowId: string, operation?: string, x?: int, y?: int, width?: int, height?: int }): Struct;

    /**
     * Waits until a window matches the title and application, or the timeout passes. When several windows match, the focused one is returned, otherwise the first in the operating system's window order (front to back)
     * @node computer_wait_for_window @alias computerWaitForWindow
     * @param session — Active session
     * @param windowTitle — Window title substring
     * @param processName (optional) — Exact application name
     * @param focused (optional) — Wait until the matched window is focused
     * @param timeoutMs (optional) — Maximum wait in milliseconds
     * @returns window — Matched window
     * @impure has side effects / drives control flow
     */
    function waitForWindow({ session: Struct, windowTitle: string, processName?: string, focused?: bool, timeoutMs?: int }): Struct;
}

declare namespace rpa {
    // === Automation/RPA ===

    /**
     * Asserts that a specific color exists at a position
     * @node rpa_assert_color @alias rpaAssertColor
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
    function assertColor({ session: Struct, x?: int, y?: int, red?: int, green?: int, blue?: int, tolerance?: int }): bool;

    /**
     * Asserts that a template image exists on screen
     * @node rpa_assert_template_exists @alias rpaAssertTemplateExists
     * @param session — RPA session handle
     * @param templatePath (optional) — Path to the template image
     * @param template — Template image from any FlowPath store; preferred over a local path
     * @param confidence (optional) — Minimum match confidence
     * @returns passed — Whether assertion passed
     * @impure has side effects / drives control flow
     */
    function assertTemplateExists({ session: Struct, templatePath?: string, template: Struct, confidence?: float }): bool;

    /**
     * Calculates elapsed time from a start timestamp
     * @node rpa_calculate_elapsed @alias rpaCalculateElapsed
     * @param startTime (optional) — Start timestamp (ms since epoch) from Start Timer node
     * @returns elapsedMs — Time elapsed in milliseconds
     * @returns elapsedSec — Time elapsed in seconds
     * @impure has side effects / drives control flow
     */
    function calculateElapsed({ startTime?: int }): { elapsedMs: int, elapsedSec: float };

    /**
     * Performs a click at a specific screen position
     * @node rpa_click_at_position @alias rpaClickAtPosition
     * @param session — Automation session
     * @param x (optional) — X coordinate
     * @param y (optional) — Y coordinate
     * @param clickType (optional) — Type of click to perform
     * @impure has side effects / drives control flow
     */
    function clickAtPosition({ session: Struct, x?: int, y?: int, clickType?: string }): void;

    /**
     * Captures diagnostic info when an automation fails
     * @node rpa_diagnose_failure @alias rpaDiagnoseFailure
     * @param session — RPA session handle
     * @param errorMessage (optional) — The error that occurred
     * @param screenshotPath (optional) — Path to save diagnostic screenshot
     * @returns diagnosticInfo — JSON string with diagnostic data
     * @impure has side effects / drives control flow
     */
    function diagnoseFailure({ session: Struct, errorMessage?: string, screenshotPath?: string }): string;

    /**
     * Performs a drag and drop operation
     * @node rpa_drag_drop @alias rpaDragDrop
     * @param session — Automation session
     * @param fromX (optional) — Start X coordinate
     * @param fromY (optional) — Start Y coordinate
     * @param toX (optional) — End X coordinate
     * @param toY (optional) — End Y coordinate
     * @param durationSec (optional) — Duration of drag in seconds
     * @impure has side effects / drives control flow
     */
    function dragDrop({ session: Struct, fromX?: int, fromY?: int, toX?: int, toY?: int, durationSec?: float }): void;

    /**
     * Defines recovery actions for specific error types
     * @node rpa_error_recovery @alias rpaErrorRecovery
     * @param errorType (optional) — Type of error to handle
     * @param actualError (optional) — The actual error message to check
     * @impure has side effects / drives control flow
     */
    function errorRecovery({ errorType?: string, actualError?: string }): void;

    /**
     * Finds a pixel on screen matching a specific color
     * @node rpa_locate_color @alias rpaLocateColor
     * @param session — RPA session handle
     * @param red (optional) — Red component (0-255)
     * @param green (optional) — Green component (0-255)
     * @param blue (optional) — Blue component (0-255)
     * @param tolerance (optional) — Color matching tolerance (0-255)
     * @returns x — X coordinate
     * @returns y — Y coordinate
     * @impure has side effects / drives control flow
     */
    function locateColor({ session: Struct, red?: int, green?: int, blue?: int, tolerance?: int }): { x: int, y: int };

    /**
     * Finds an element on screen using template matching
     * @node rpa_locate_template @alias rpaLocateTemplate
     * @param session — RPA session handle
     * @param templatePath (optional) — Path to the template image
     * @param template — Template image from any FlowPath store; preferred over a local path
     * @param confidence (optional) — Minimum match confidence (0.0-1.0)
     * @returns x — X coordinate
     * @returns y — Y coordinate
     * @impure has side effects / drives control flow
     */
    function locateTemplate({ session: Struct, templatePath?: string, template: Struct, confidence?: float }): { x: int, y: int };

    /**
     * Logs an automation action for debugging and auditing
     * @node rpa_log_action @alias rpaLogAction
     * @param action (optional) — Action being performed
     * @param details (optional) — Additional details about the action
     * @param level (optional) — Log level
     * @returns logEntry — Formatted log entry
     * @impure has side effects / drives control flow
     */
    function logAction({ action?: string, details?: string, level?: string }): string;

    /**
     * Parses checkpoint data from a saved JSON string
     * @node rpa_parse_checkpoint @alias rpaParseCheckpoint
     * @param checkpointData (optional) — Checkpoint JSON string to parse
     * @returns data — Extracted data from checkpoint
     * @returns name — Checkpoint name
     * @returns timestamp — When checkpoint was saved
     * @impure has side effects / drives control flow
     */
    function parseCheckpoint({ checkpointData?: string }): { data: string, name: string, timestamp: string };

    /**
     * Runs an action again after an error or a retry condition, with configurable backoff.
     * @node rpa_retry_loop @alias rpaRetryLoop
     * @param maxRetries (optional) — Maximum number of retry attempts
     * @param initialDelayMs (optional) — Initial delay before first retry
     * @param backoffType (optional) — Type of backoff strategy
     * @param shouldRetry (optional) — Retry even when the action succeeds (connect a condition if needed)
     * @returns attempt — Current attempt number
     * @returns totalAttempts — Total attempts made
     * @returns lastError — Most recent action failure, empty after success
     * @impure has side effects / drives control flow
     */
    function retryLoop({ maxRetries?: int, initialDelayMs?: int, backoffType?: string, shouldRetry?: bool }): { attempt: int, totalAttempts: int, lastError: string };

    /**
     * Creates checkpoint data for potential recovery
     * @node rpa_save_checkpoint @alias rpaSaveCheckpoint
     * @param checkpointName (optional) — Name to identify this checkpoint
     * @param data (optional) — JSON string of data to save at checkpoint
     * @returns checkpointData — Complete checkpoint data as JSON
     * @returns checkpointId — Unique ID for this checkpoint
     * @impure has side effects / drives control flow
     */
    function saveCheckpoint({ checkpointName?: string, data?: string }): { checkpointData: string, checkpointId: string };

    /**
     * Scrolls the mouse wheel vertically at the current pointer position. Positive Clicks scroll UP (towards the top of a page); note that the Computer Scroll node uses the opposite sign
     * @node rpa_scroll @alias rpaScroll
     * @param session — Automation session
     * @param clicks (optional) — Wheel ticks: positive scrolls UP (content moves down), negative scrolls down (-1000 to 1000)
     * @impure has side effects / drives control flow
     */
    function scroll({ session: Struct, clicks?: int }): void;

    /**
     * Returns the current timestamp for measuring action duration
     * @node rpa_start_timer @alias rpaStartTimer
     * @returns startTime — Timestamp when timer started (ms since epoch)
     * @impure has side effects / drives control flow
     */
    function startTimer(): int;

    /**
     * Captures a screen snapshot and saves to file
     * @node rpa_take_snapshot @alias rpaTakeSnapshot
     * @param session — Automation session
     * @param filePath — Path to save the snapshot image
     * @param monitor (optional) — Monitor index from List Displays (-1 = primary)
     * @returns success — Whether the snapshot was saved
     * @impure has side effects / drives control flow
     */
    function takeSnapshot({ session: Struct, filePath: Struct, monitor?: int }): bool;

    /**
     * Types text using keyboard simulation
     * @node rpa_type_text @alias rpaTypeText
     * @param session — Automation session
     * @param text (optional) — Text to type
     * @param intervalMs (optional) — Delay between keystrokes
     * @impure has side effects / drives control flow
     */
    function typeText({ session: Struct, text?: string, intervalMs?: int }): void;

    /**
     * Pauses execution for a specified duration
     * @node rpa_delay @alias rpaDelay
     * @param durationMs (optional) — Delay duration in milliseconds
     * @impure has side effects / drives control flow
     */
    function wait({ durationMs?: int }): void;

    /**
     * Waits for a specific color to appear at a position
     * @node rpa_wait_for_color @alias rpaWaitForColor
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
    function waitForColor({ session: Struct, x?: int, y?: int, red?: int, green?: int, blue?: int, tolerance?: int, timeoutMs?: int }): void;

    /**
     * Waits for a template to appear on screen
     * @node rpa_wait_for_template @alias rpaWaitForTemplate
     * @param session — Automation session
     * @param templatePath (optional) — Path to the template image
     * @param template — Template image from any FlowPath store; preferred over a local path
     * @param confidence (optional) — Minimum match confidence (0.0-1.0)
     * @param timeoutMs (optional) — Maximum wait time in milliseconds
     * @param pollIntervalMs (optional) — Check interval in milliseconds
     * @returns x — X coordinate
     * @returns y — Y coordinate
     * @impure has side effects / drives control flow
     */
    function waitForTemplate({ session: Struct, templatePath?: string, template: Struct, confidence?: float, timeoutMs?: int, pollIntervalMs?: int }): { x: int, y: int };

    /**
     * Executes an action with a timeout constraint
     * @node rpa_with_timeout @alias rpaWithTimeout
     * @param timeoutMs (optional) — Maximum time to wait for action
     * @param completed (optional) — Legacy input; completion is determined by the action branch
     * @returns elapsedMs — Time elapsed
     * @impure has side effects / drives control flow
     */
    function withTimeout({ timeoutMs?: int, completed?: bool }): int;
}
