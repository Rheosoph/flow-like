import type { KeyboardEvent } from "react";

/** An IME composition is open: the key belongs to the IME and never moves on or runs (spec §4). */
export function isComposing(event: KeyboardEvent): boolean {
	return event.nativeEvent.isComposing || event.keyCode === 229;
}

/** ⌘ on a Mac, Ctrl elsewhere. */
export function hasCommand(event: KeyboardEvent, mac: boolean): boolean {
	return mac ? event.metaKey : event.ctrlKey;
}

/** ⇧⌘⌫ (Ctrl+Shift+Backspace): reset the field to its starting value (spec M2). */
export function isResetChord(event: KeyboardEvent, mac: boolean): boolean {
	return (
		event.key === "Backspace" &&
		event.shiftKey &&
		!event.altKey &&
		hasCommand(event, mac)
	);
}

/** No modifier at all: the key means what it says. */
export function isPlain(event: KeyboardEvent): boolean {
	return !(event.metaKey || event.ctrlKey || event.altKey || event.shiftKey);
}

/** ↵ without ⇧, ⌘, Ctrl or Alt: moves on. ⌘↵ and ⇧⌘↵ belong to the interface root. */
export function isPlainEnter(event: KeyboardEvent): boolean {
	return event.key === "Enter" && isPlain(event);
}

/** A held ↵ or Space must not click a button again and again (spec M2): the repeat is swallowed. */
export function swallowRepeat(event: KeyboardEvent): void {
	if (event.repeat && (event.key === "Enter" || event.key === " "))
		event.preventDefault();
}

/** The value of `aria-keyshortcuts` for the reset chord. */
export function resetShortcut(mac: boolean): string {
	return mac ? "Meta+Shift+Backspace" : "Control+Shift+Backspace";
}
