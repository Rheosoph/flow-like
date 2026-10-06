import type { WorkbenchField } from "../contracts";
import { formatDate } from "../model/date-text";

/*
 * What the entry of a list field does with its text and where the cursor goes between chips (spec M2): ← in an empty
 * entry goes into the chips, ←/→ move between them, ⌫ or Delete removes the focused one and → past the last one comes
 * back to the entry. Pure, so the key table is testable without a DOM.
 */

/** A chip, or the entry. */
export type ChipTarget = number | "entry";

export type ChipMove =
	| { readonly kind: "focus"; readonly to: ChipTarget }
	| { readonly kind: "remove"; readonly after: ChipTarget };

/** The chip to focus once one was removed: the one at `index` of what is left, else the entry. */
export function neighbour(index: number, left: number): ChipTarget {
	if (left <= 0) return "entry";
	return Math.min(Math.max(index, 0), left - 1);
}

/** What a key does on the chip at `index` of `count` chips; null for a key that means nothing here. */
export function chipMove(
	key: string,
	index: number,
	count: number,
): ChipMove | null {
	if (key === "ArrowLeft")
		return index > 0 ? { kind: "focus", to: index - 1 } : null;
	if (key === "ArrowRight")
		return { kind: "focus", to: index < count - 1 ? index + 1 : "entry" };
	if (key === "Backspace")
		return { kind: "remove", after: neighbour(index - 1, count - 1) };
	if (key === "Delete")
		return { kind: "remove", after: neighbour(index, count - 1) };
	return null;
}

/** The entry takes the cursor into the chips on ← or ⌫ while it is empty. */
export function entersChips(key: string, entry: string, count: number) {
	return (
		entry === "" && count > 0 && (key === "ArrowLeft" || key === "Backspace")
	);
}

/**
 * The chips a text adds: typed text split at commas; a list of dates reads each part in the viewer's order
 * (`read` returns the day or null). null when a date cannot be read: nothing is added.
 */
export function itemsFrom(
	text: string,
	field: Pick<WorkbenchField, "itemKind">,
	read: (typed: string) => string | null,
): readonly string[] | null {
	const parts = text
		.split(",")
		.map((part) => part.trim())
		.filter((part) => part !== "");
	if (field.itemKind !== "date") return parts;
	const days = parts.map(read);
	return days.every((day): day is string => day !== null) ? days : null;
}

/** What a chip reads as: dates in the viewer's format, everything else as typed. */
export function chipText(
	kind: "text" | "number" | "date",
	item: string,
	locale: string,
): string {
	return kind === "date" ? formatDate(item, locale) : item;
}
