/**
 * React Flow skips its global key handling only for INPUT/SELECT/TEXTAREA/contenteditable
 * targets, or for anything inside an element carrying this marker class. Monaco types through
 * an EditContext `<div class="native-edit-context">` on Chromium, which matches none of those,
 * so any mounted canvas swallows Space (pan activation) and Backspace/Delete (which then wipe
 * the canvas selection) while the caret sits in an editor. Put this on an ancestor of every
 * Monaco surface.
 */
export const FLOW_KEY_OPT_OUT_CLASS = "nokey";
