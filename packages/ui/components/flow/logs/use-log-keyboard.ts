import { type KeyboardEvent, useCallback, useRef } from "react";
import type { ILog } from "../../../lib/schema/flow/log";

function isTypingTarget(target: EventTarget | null): boolean {
	return (
		target instanceof HTMLElement &&
		!!target.closest(
			"input, textarea, select, [contenteditable='true'], [role='menu']",
		)
	);
}

function isButton(target: EventTarget | null): boolean {
	return target instanceof HTMLElement && !!target.closest("button");
}

export interface ILogKeyboardActions {
	focused?: { index?: number; log: ILog };
	selectedCount: number;
	copySelected(): void;
	stepError(direction: 1 | -1): void;
	moveFocus(delta: number): void;
	toggleSelect(index: number, log: ILog, shift: boolean): void;
	clearSelection(): void;
	clearFocus(): void;
	focusQuery(): void;
}

/**
 * Shortcuts for the panel element only, never the window: E / Shift+E step
 * through errors, arrows move the focused row, Space selects it, Escape
 * clears, `/` jumps to the query bar, and Cmd/Ctrl+C copies selected rows
 * when no text is highlighted.
 */
export function useLogKeyboard(actions: ILogKeyboardActions) {
	const latest = useRef(actions);
	latest.current = actions;

	return useCallback((event: KeyboardEvent<HTMLElement>) => {
		if (isTypingTarget(event.target)) return;
		const current = latest.current;
		const mod = event.metaKey || event.ctrlKey;
		if (mod && event.key.toLowerCase() === "c" && current.selectedCount > 0) {
			if (globalThis.getSelection?.()?.isCollapsed ?? true) {
				event.preventDefault();
				current.copySelected();
			}
			return;
		}
		if (mod || event.altKey) return;
		switch (event.key) {
			case "e":
			case "E":
				event.preventDefault();
				current.stepError(event.shiftKey ? -1 : 1);
				return;
			case "ArrowDown":
			case "ArrowUp":
				event.preventDefault();
				current.moveFocus(event.key === "ArrowDown" ? 1 : -1);
				return;
			case " ": {
				const focused = current.focused;
				if (focused?.index === undefined || isButton(event.target)) return;
				event.preventDefault();
				current.toggleSelect(focused.index, focused.log, event.shiftKey);
				return;
			}
			case "Escape":
				if (current.selectedCount > 0) current.clearSelection();
				else current.clearFocus();
				return;
			case "/":
				event.preventDefault();
				current.focusQuery();
				return;
		}
	}, []);
}
