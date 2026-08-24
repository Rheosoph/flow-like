// Logging — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace log {
    // === Logging ===

    /**
     * Logs / Prints an Error
     * @node log_error @alias logError
     * @param message (optional) — Print Error Message
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function error({ message?: any, toast?: bool }): void;

    /**
     * Print Debugging Information
     * @node log_info @alias logInfo
     * @param message (optional) — The message to log
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function info({ message?: any, toast?: bool }): void;

    /**
     * Shows a progress toast to the user that can be updated
     * @node log_progress @alias logProgress
     * @param id (optional) — Unique identifier for this progress. Use the same ID to update the progress.
     * @param message (optional) — The message shown to the user
     * @param progress (optional) — Progress value between 0 and 100. Leave empty to show indeterminate progress.
     * @impure has side effects / drives control flow
     */
    function progress({ id?: string, message?: string, progress?: int }): void;

    /**
     * Completes a progress toast with a success or error state
     * @node log_progress_done @alias logProgressDone
     * @param id (optional) — The ID of the progress toast to complete (must match the ID used in Show Progress)
     * @param message (optional) — Final message to show (e.g., 'Completed!' or 'Failed')
     * @param success (optional) — Whether the operation was successful (true shows success toast, false shows error)
     * @impure has side effects / drives control flow
     */
    function progressDone({ id?: string, message?: string, success?: bool }): void;

    /**
     * Logs a Warning
     * @node log_warning @alias logWarning
     * @param message (optional) — Print Warning
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function warn({ message?: any, toast?: bool }): void;
}
