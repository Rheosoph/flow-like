import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import type { EditHow } from "../contracts";

/*
 * The inline suggestion of a text field (spec M6): from the first typed character the rest of the newest
 * recent value that starts with the text is inserted as real selected text, as in a browser's address bar.
 * It is display only: the rail's value is always exactly what was typed, so ↵ and Tab use that, and only
 * → or End take the suggestion.
 */

export interface SelectionRange {
	readonly start: number;
	readonly end: number;
}

/** How an edit began: in an empty field or over a fully selected value it replaces (the offer's `replaced`), else it edits in place. */
export function editHow(previous: string, selection: SelectionRange): EditHow {
	if (previous === "") return "replace";
	return selection.start === 0 && selection.end >= previous.length
		? "replace"
		: "inPlace";
}

/** What the field shows while a suggestion is inserted: what was typed, then the rest of the suggestion. */
export function displayWith(typed: string, suggestion: string | null): string {
	return suggestion === null ? typed : typed + suggestion.slice(typed.length);
}

export interface SuggestionState {
	/** The suggestion for the typed text, or null. */
	readonly suggestion: string | null;
	/** Typing armed the suggestion for this text. */
	arm(typed: string): void;
	/** Drop it (Esc, ⌫, ←, Tab, ↵, blur); `caret` places the cursor after the text shrinks back. */
	drop(caret?: number): void;
	/** The selection before the edit in progress, read on keydown, paste and cut. */
	readonly selection: RefObject<SelectionRange>;
	readonly remember: (element: HTMLTextAreaElement | HTMLInputElement) => void;
}

/**
 * Armed by typing, dropped by anything else. `find(typed)` is `completion(...)` for the field. The
 * suggestion's rest is selected after every render that shows it.
 */
export function useSuggestion(
	element: RefObject<HTMLTextAreaElement | HTMLInputElement | null>,
	typed: string,
	find: (typed: string) => string | null,
): SuggestionState {
	const [armed, setArmed] = useState<string | null>(null);
	const caret = useRef<number | null>(null);
	const selection = useRef<SelectionRange>({ start: 0, end: 0 });
	const showing = useRef(false);
	const suggestion = armed === typed && typed !== "" ? find(typed) : null;
	showing.current = suggestion !== null;
	const shown = displayWith(typed, suggestion);
	useLayoutEffect(() => {
		const node = element.current;
		if (!node) return;
		if (suggestion !== null && node.ownerDocument.activeElement === node) {
			node.setSelectionRange(typed.length, shown.length);
			return;
		}
		if (caret.current !== null) {
			node.setSelectionRange(caret.current, caret.current);
			caret.current = null;
		}
	}, [element, suggestion, typed, shown]);
	const arm = useCallback((text: string) => setArmed(text), []);
	const drop = useCallback((place?: number) => {
		caret.current = showing.current ? (place ?? null) : null;
		setArmed(null);
	}, []);
	const remember = useCallback(
		(node: HTMLTextAreaElement | HTMLInputElement) => {
			selection.current = {
				start: node.selectionStart ?? 0,
				end: node.selectionEnd ?? 0,
			};
		},
		[],
	);
	return { suggestion, arm, drop, selection, remember };
}
