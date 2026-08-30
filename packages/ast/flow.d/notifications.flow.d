// Notifications — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace notify {
    // === Notifications ===

    /**
     * Send a notification to a specific user in this project
     * @node notify_project_user @alias notifyProjectUser
     * @param flowUserSub (optional) — Project user to notify
     * @param title (optional) — Notification title
     * @param description (optional) — Notification description (optional)
     * @param icon — FlowPath to a notification icon image (optional)
     * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
     * @returns success — Whether the notification was sent successfully
     * @impure has side effects / drives control flow
     */
    function projectUser({ flowUserSub?: string, title?: string, description?: string, icon: Struct, link?: string }): bool;

    /**
     * Send a notification to the user who executed this workflow
     * @node notify_user @alias notifyUser
     * @param title (optional) — Notification title
     * @param description (optional) — Notification description (optional)
     * @param icon — FlowPath to a notification icon image (optional)
     * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
     * @param showDesktop (optional) — Show desktop notification if available
     * @returns success — Whether the notification was sent successfully
     * @impure has side effects / drives control flow
     */
    function user({ title?: string, description?: string, icon: Struct, link?: string, showDesktop?: bool }): bool;
}
