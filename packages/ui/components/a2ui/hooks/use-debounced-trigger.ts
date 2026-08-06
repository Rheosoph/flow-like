"use client";

import { useCallback, useEffect, useRef } from "react";

/** Fastest pause an author may configure. Below this, "typing paused" would
 * effectively become "per keystroke" and every character would start a run. */
export const MIN_EVENT_DEBOUNCE_MS = 100;
export const DEFAULT_EVENT_DEBOUNCE_MS = 400;

export function resolveEventDebounceMs(value: number | undefined): number {
	if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
		return DEFAULT_EVENT_DEBOUNCE_MS;
	}
	return Math.max(MIN_EVENT_DEBOUNCE_MS, value);
}

/**
 * Run a callback once the user has paused for `delayMs`.
 *
 * `cancel` exists so a commit can swallow the pending pause: typing and then
 * immediately blurring should dispatch the committed value, not a stale
 * mid-typing one right after it.
 */
export function useDebouncedTrigger(delayMs: number) {
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	const cancel = useCallback(() => {
		if (timerRef.current !== null) {
			clearTimeout(timerRef.current);
			timerRef.current = null;
		}
	}, []);

	useEffect(() => cancel, [cancel]);

	const schedule = useCallback(
		(callback: () => void) => {
			cancel();
			timerRef.current = setTimeout(() => {
				timerRef.current = null;
				callback();
			}, delayMs);
		},
		[cancel, delayMs],
	);

	return { schedule, cancel };
}
