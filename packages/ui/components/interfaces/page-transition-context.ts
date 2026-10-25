"use client";

import { createContext, useContext, useLayoutEffect } from "react";

export interface PageTransitionSlot {
	/** Mounted out of sight while the page before it stays on screen. */
	readonly offscreen: boolean;
	/** Keeps the page before this one on screen until this one can show content. */
	readonly hold: (holding: boolean) => void;
}

export const PageTransitionContext = createContext<PageTransitionSlot | null>(
	null,
);

/**
 * Reports whether the page still has nothing to show. A layout effect, so a page that is
 * presentable at once replaces its predecessor before the first paint. Returns whether the page
 * is currently mounted out of sight.
 */
export function usePageTransitionHold(holding: boolean): boolean {
	const slot = useContext(PageTransitionContext);
	const hold = slot?.hold;
	useLayoutEffect(() => {
		if (!hold) return;
		hold(holding);
		return () => hold(false);
	}, [hold, holding]);
	return slot?.offscreen ?? false;
}
