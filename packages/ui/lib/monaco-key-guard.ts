/**
 * React Flow skips its global key handling only for INPUT/SELECT/TEXTAREA/contenteditable
 * targets, or for anything inside an element carrying this marker class. Monaco types through
 * an EditContext `<div class="native-edit-context">` on Chromium, which matches none of those,
 * so any mounted canvas swallows Space (pan activation) and Backspace/Delete (which then wipe
 * the canvas selection) while the caret sits in an editor. Put this on an ancestor of every
 * Monaco surface.
 */
export const FLOW_KEY_OPT_OUT_CLASS = "nokey";

/**
 * The `nokey` marker only reaches React Flow's own `useKeyPress` — the board
 * additionally installs document-level keydown handlers (delete-selection on
 * Backspace/Delete, board undo/redo on ⌘Z/⌘⇧Z/⌘Y, node placement on ⌘B/⌘F/⌘P/⌘S,
 * search on ⌘F/⌘⇧F, pages on ⌘⇧P) that check only INPUT/TEXTAREA/contenteditable
 * targets. Monaco's Chromium EditContext host is none of those, so those handlers
 * fire — and some preventDefault — while the caret sits in an editor.
 *
 * `shieldFlowBoardKeys` stops exactly those keys at a container around the
 * editor. It listens in the BUBBLE phase (capture would starve Monaco's own
 * target-phase keydown handlers) — Monaco sees the event first, then the shield
 * stops it before it bubbles to the document/window listeners. It never calls
 * preventDefault, so native EditContext/textarea behavior is untouched.
 */
const SHIELDED_PLAIN_KEYS = new Set(["Backspace", "Delete", " "]);
const SHIELDED_MODIFIER_KEYS = new Set(["z", "y", "b", "f", "p", "s"]);

export function isShieldedFlowBoardKey(event: KeyboardEvent): boolean {
	const hasPrimaryModifier = event.metaKey || event.ctrlKey;
	if (hasPrimaryModifier) {
		return SHIELDED_MODIFIER_KEYS.has(event.key.toLowerCase());
	}
	return SHIELDED_PLAIN_KEYS.has(event.key);
}

export function shieldFlowBoardKeys(
	container: HTMLElement,
	isEditorFocused: () => boolean,
): () => void {
	const handler = (event: KeyboardEvent) => {
		if (!isEditorFocused()) return;
		if (!isShieldedFlowBoardKey(event)) return;
		event.stopPropagation();
	};
	container.addEventListener("keydown", handler);
	return () => container.removeEventListener("keydown", handler);
}
