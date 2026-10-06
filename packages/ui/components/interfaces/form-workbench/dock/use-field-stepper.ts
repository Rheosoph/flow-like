import { type RefObject, useCallback, useLayoutEffect, useRef } from "react";
import type { FieldKey } from "../contracts";
import { focusField, focusedFieldKey, interfaceRootOf } from "./focus";

/** How long the dock waits for the Inputs pane to show a field it asked for. */
const REVEAL_WAIT_MS = 600;

/**
 * The index ▼ (1) or ▲ (−1) lands on, wrapping. `from` is where the cursor is, else where the last step went, else
 * nowhere: the first ▼ lands on the first field and the first ▲ on the last.
 */
export function stepIndex(
	count: number,
	from: number | null,
	direction: 1 | -1,
) {
	const start = from ?? (direction > 0 ? -1 : 0);
	return (start + direction + count) % count;
}

function startOf(
	keys: readonly FieldKey[],
	focused: FieldKey | null,
	remembered: number | null,
) {
	const known = focused === null ? -1 : keys.indexOf(focused);
	if (known >= 0) return known;
	return remembered === null ? null : Math.min(remembered, keys.length - 1);
}

interface Pending {
	readonly key: FieldKey;
	readonly since: number;
}

export interface FieldStepperOptions {
	/** Any element of the dock: the root and the cursor are read from it. */
	readonly dockRef: RefObject<HTMLElement | null>;
	/** The fields to visit, in form order. */
	readonly keys: readonly FieldKey[];
	/** Asks for the Inputs tab, the Inputs pane and an empty filter, so every field is in the page. */
	readonly reveal: () => void;
}

/**
 * The ▲▼ buttons of "N fields need a look" and "N fields to fill in" (spec M5, fix-report). A field that is not in the
 * page yet (Runs tab, Output pane, a filter) is revealed first and focused after the next render.
 */
export function useFieldStepper({
	dockRef,
	keys,
	reveal,
}: Readonly<FieldStepperOptions>) {
	const lastAt = useRef<number | null>(null);
	const pending = useRef<Pending | null>(null);

	useLayoutEffect(() => {
		const wanted = pending.current;
		if (!wanted) return;
		const root = interfaceRootOf(dockRef.current);
		const arrived = root !== null && focusField(root, wanted.key);
		if (arrived || Date.now() - wanted.since > REVEAL_WAIT_MS)
			pending.current = null;
	});

	return useCallback(
		(direction: 1 | -1) => {
			if (keys.length === 0) return;
			const focused = focusedFieldKey(dockRef.current);
			const from = startOf(keys, focused, lastAt.current);
			const at = stepIndex(keys.length, from, direction);
			lastAt.current = at;
			const root = interfaceRootOf(dockRef.current);
			if (root && focusField(root, keys[at])) return;
			pending.current = { key: keys[at], since: Date.now() };
			reveal();
		},
		[dockRef, keys, reveal],
	);
}
