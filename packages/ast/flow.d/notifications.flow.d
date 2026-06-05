// Notifications — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

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

