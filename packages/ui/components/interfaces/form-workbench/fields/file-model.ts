/*
 * What dragging files over a file field means (spec M3): the state of the drag and what dropping would do. Pure.
 */

export interface DragState {
	/** Files are over the field. */
	readonly over: boolean;
	/** How many; null when the browser does not say. */
	readonly count: number | null;
}

export const NO_DRAG: DragState = { over: false, count: null };

export type DropKind = "many" | "unknown" | "replace";

const several = (drag: DragState, oneRunEach: boolean) =>
	oneRunEach && (drag.count ?? 0) > 1;

/** What dropping would do: start one run per file, an unknown number of them, or replace the attached file. */
export function dropKind(
	drag: DragState,
	oneRunEach: boolean,
	replace: boolean,
): DropKind | null {
	if (!drag.over) return null;
	if (several(drag, oneRunEach)) return "many";
	if (replace) return "replace";
	return oneRunEach && drag.count === null ? "unknown" : null;
}
