// Logging — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

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

