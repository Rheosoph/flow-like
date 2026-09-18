"use client";

import { useCallback, useEffect, useRef, useState } from "react";

/** Allow controls ignore activation this long after they could have moved under the pointer. */
export const MICRO_WIDGET_CONSENT_INERT_MS = 500;

function now(): number {
	return typeof performance !== "undefined" ? performance.now() : Date.now();
}

export interface MicroWidgetInertWindow {
	/** For `aria-disabled`; the guard itself reads the clock. */
	inert: boolean;
	isInert: () => boolean;
	restart: () => void;
}

/**
 * Clickjacking guard for consent controls (§14.5.3): inert on mount, and again
 * whenever the page becomes visible, the window regains focus or the caller
 * restarts it because the content under the pointer changed. It never moves
 * focus and has no animation.
 */
export function useMicroWidgetInertWindow(): MicroWidgetInertWindow {
	const [inert, setInert] = useState(true);
	const until = useRef(Number.POSITIVE_INFINITY);
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

	const restart = useCallback(() => {
		until.current = now() + MICRO_WIDGET_CONSENT_INERT_MS;
		setInert(true);
		if (timer.current !== null) clearTimeout(timer.current);
		timer.current = setTimeout(() => {
			timer.current = null;
			setInert(false);
		}, MICRO_WIDGET_CONSENT_INERT_MS);
	}, []);

	useEffect(() => {
		restart();
		const onVisibility = () => {
			if (document.visibilityState === "visible") restart();
		};
		window.addEventListener("focus", restart);
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			window.removeEventListener("focus", restart);
			document.removeEventListener("visibilitychange", onVisibility);
			if (timer.current !== null) clearTimeout(timer.current);
			timer.current = null;
		};
	}, [restart]);

	const isInert = useCallback(() => now() < until.current, []);
	return { inert, isInert, restart };
}
