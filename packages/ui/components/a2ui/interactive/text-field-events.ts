export type EnterIntent =
	| { kind: "ignore" }
	| { kind: "newline" }
	| { kind: "submit"; via: "enter" | "modEnter" };

export interface EnterKeyState {
	key: string;
	metaKey: boolean;
	ctrlKey: boolean;
}

/**
 * Decide what Enter means for a text field.
 *
 * A multiline field owns plain Enter for newlines, so only ⌘/Ctrl+Enter can
 * submit there. A single-line field submits on either, and never on any other
 * key — the caller must not preventDefault for `ignore` or `newline`.
 */
export function resolveEnterIntent(
	event: EnterKeyState,
	multiline: boolean | undefined,
): EnterIntent {
	if (event.key !== "Enter") return { kind: "ignore" };

	const withModifier = event.metaKey || event.ctrlKey;
	if (multiline && !withModifier) return { kind: "newline" };
	return { kind: "submit", via: withModifier ? "modEnter" : "enter" };
}

/**
 * A commit reports `change` only when the value actually moved. Blur fires
 * whenever focus leaves, so without this a user tabbing through an untouched
 * form would run every field's workflow.
 */
export function shouldReportCommit(
	lastCommitted: string,
	next: string,
): boolean {
	return lastCommitted !== next;
}
