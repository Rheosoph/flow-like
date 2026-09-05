"use client";

export type ShortcutModifiers = {
	ctrl?: boolean;
	shift?: boolean;
	alt?: boolean;
	meta?: boolean;
};

export type KeyboardShortcut = {
	key: string;
	modifiers?: ShortcutModifiers;
	action: string;
	description: string;
	when?: string;
};

const DEFAULT_SHORTCUTS: KeyboardShortcut[] = [
	// Selection
	{
		key: "a",
		modifiers: { ctrl: true },
		action: "selectAll",
		description: "Select All",
	},
	{ key: "Escape", action: "deselectAll", description: "Deselect All" },
	{ key: "Delete", action: "delete", description: "Delete Selected" },
	{ key: "Backspace", action: "delete", description: "Delete Selected" },

	// Clipboard
	{ key: "c", modifiers: { ctrl: true }, action: "copy", description: "Copy" },
	{ key: "x", modifiers: { ctrl: true }, action: "cut", description: "Cut" },
	{
		key: "v",
		modifiers: { ctrl: true },
		action: "paste",
		description: "Paste",
	},
	{
		key: "d",
		modifiers: { ctrl: true },
		action: "duplicate",
		description: "Duplicate",
	},

	// History
	{ key: "z", modifiers: { ctrl: true }, action: "undo", description: "Undo" },
	{
		key: "z",
		modifiers: { ctrl: true, shift: true },
		action: "redo",
		description: "Redo",
	},
	{ key: "y", modifiers: { ctrl: true }, action: "redo", description: "Redo" },

	// View
	{
		key: "=",
		modifiers: { ctrl: true },
		action: "zoomIn",
		description: "Zoom In",
	},
	{
		key: "-",
		modifiers: { ctrl: true },
		action: "zoomOut",
		description: "Zoom Out",
	},
	{
		key: "0",
		modifiers: { ctrl: true },
		action: "zoomReset",
		description: "Reset Zoom",
	},
	{
		key: "1",
		modifiers: { ctrl: true },
		action: "fitToScreen",
		description: "Fit to Screen",
	},
	{
		key: "g",
		modifiers: { ctrl: true },
		action: "toggleGrid",
		description: "Toggle Grid",
	},

	// Arrangement
	{
		key: "[",
		modifiers: { ctrl: true },
		action: "sendBackward",
		description: "Send Backward",
	},
	{
		key: "]",
		modifiers: { ctrl: true },
		action: "bringForward",
		description: "Bring Forward",
	},
	{
		key: "[",
		modifiers: { ctrl: true, shift: true },
		action: "sendToBack",
		description: "Send to Back",
	},
	{
		key: "]",
		modifiers: { ctrl: true, shift: true },
		action: "bringToFront",
		description: "Bring to Front",
	},

	// Grouping
	{
		key: "g",
		modifiers: { ctrl: true, shift: true },
		action: "group",
		description: "Group",
	},
	{
		key: "u",
		modifiers: { ctrl: true, shift: true },
		action: "ungroup",
		description: "Ungroup",
	},

	// Lock/Hide
	{
		key: "l",
		modifiers: { ctrl: true },
		action: "toggleLock",
		description: "Toggle Lock",
	},
	{
		key: "h",
		modifiers: { ctrl: true },
		action: "toggleHide",
		description: "Toggle Hide",
	},

	// Save
	{ key: "s", modifiers: { ctrl: true }, action: "save", description: "Save" },
	{
		key: "e",
		modifiers: { ctrl: true },
		action: "export",
		description: "Export",
	},

	// Arrow nudge
	{ key: "ArrowUp", action: "nudgeUp", description: "Move Up" },
	{ key: "ArrowDown", action: "nudgeDown", description: "Move Down" },
	{ key: "ArrowLeft", action: "nudgeLeft", description: "Move Left" },
	{ key: "ArrowRight", action: "nudgeRight", description: "Move Right" },
	{
		key: "ArrowUp",
		modifiers: { shift: true },
		action: "nudgeUpLarge",
		description: "Move Up (Large)",
	},
	{
		key: "ArrowDown",
		modifiers: { shift: true },
		action: "nudgeDownLarge",
		description: "Move Down (Large)",
	},
	{
		key: "ArrowLeft",
		modifiers: { shift: true },
		action: "nudgeLeftLarge",
		description: "Move Left (Large)",
	},
	{
		key: "ArrowRight",
		modifiers: { shift: true },
		action: "nudgeRightLarge",
		description: "Move Right (Large)",
	},
];

export type ShortcutHandler = (action: string, event: KeyboardEvent) => void;

export function isEditableKeyboardTarget(target: EventTarget | null): boolean {
	const element = target as HTMLElement | null;
	return Boolean(
		element?.closest?.(
			'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="searchbox"], [role="combobox"], [role="spinbutton"]',
		),
	);
}

export function createShortcutManager(
	handler: ShortcutHandler,
	shortcuts = DEFAULT_SHORTCUTS,
	shouldHandle?: (event: KeyboardEvent) => boolean,
) {
	const handleKeyDown = (event: KeyboardEvent) => {
		if (
			event.defaultPrevented ||
			event.isComposing ||
			isEditableKeyboardTarget(event.target) ||
			(shouldHandle && !shouldHandle(event))
		) {
			return;
		}

		for (const shortcut of shortcuts) {
			if (matchesShortcut(event, shortcut)) {
				event.preventDefault();
				handler(shortcut.action, event);
				return;
			}
		}
	};

	return {
		bind: () => {
			window.addEventListener("keydown", handleKeyDown);
		},
		unbind: () => {
			window.removeEventListener("keydown", handleKeyDown);
		},
		shortcuts,
	};
}

function matchesShortcut(
	event: KeyboardEvent,
	shortcut: KeyboardShortcut,
): boolean {
	if (event.key.toLowerCase() !== shortcut.key.toLowerCase()) return false;

	const mods = shortcut.modifiers || {};
	const isMac = navigator.platform.includes("Mac");

	const ctrlExpected = mods.ctrl || false;
	const shiftExpected = mods.shift || false;
	const altExpected = mods.alt || false;
	const metaExpected = mods.meta || false;

	const ctrlPressed = isMac ? event.metaKey : event.ctrlKey;
	const actualCtrl = isMac ? false : event.ctrlKey;
	const actualMeta = event.metaKey;

	if (ctrlExpected) {
		if (!ctrlPressed) return false;
	} else {
		if (isMac && actualMeta) return false;
		if (!isMac && actualCtrl) return false;
	}

	if (shiftExpected !== event.shiftKey) return false;
	if (altExpected !== event.altKey) return false;
	if (!ctrlExpected && metaExpected !== event.metaKey) return false;

	return true;
}

export function formatShortcut(shortcut: KeyboardShortcut): string {
	const isMac =
		typeof navigator !== "undefined" && navigator.platform.includes("Mac");
	const parts: string[] = [];
	const mods = shortcut.modifiers || {};

	if (mods.ctrl) parts.push(isMac ? "⌘" : "Ctrl");
	if (mods.shift) parts.push(isMac ? "⇧" : "Shift");
	if (mods.alt) parts.push(isMac ? "⌥" : "Alt");
	if (mods.meta) parts.push(isMac ? "⌘" : "Win");

	const key =
		shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key;
	parts.push(key);

	return parts.join(isMac ? "" : "+");
}

export { DEFAULT_SHORTCUTS };
