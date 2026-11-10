"use client";

import { useSyncExternalStore } from "react";

/** Width of the bottom-right lane corner overlays (the activity pill) sit in. */
const CORNER_LANE_PX = 200;
/** Only elements docked this close to the viewport's bottom edge count as obstacles. */
const BOTTOM_REACH_PX = 160;
const GAP_PX = 12;
/**
 * Set on `:root` while a corner overlay is lifted over content, so scroll
 * areas can pad their end and the last item scrolls clear of the overlay.
 */
const RESERVE_VAR = "--fl-corner-overlay-reserve";

const obstacles = new Map<symbol, number>();
const listeners = new Set<() => void>();
let clearance = 0;
let overlayHeight = 0;
let reserve = 0;

function publish() {
	let next = 0;
	for (const inset of obstacles.values()) next = Math.max(next, inset);
	if (next !== clearance) {
		clearance = next;
		for (const listener of listeners) listener();
	}
	const nextReserve =
		clearance > 0 && overlayHeight > 0 ? overlayHeight + GAP_PX : 0;
	if (nextReserve === reserve) return;
	reserve = nextReserve;
	const root = document.documentElement.style;
	if (reserve > 0) root.setProperty(RESERVE_VAR, `${reserve}px`);
	else root.removeProperty(RESERVE_VAR);
}

function cornerInset(element: HTMLElement): number {
	const rect = element.getBoundingClientRect();
	const { innerWidth, innerHeight } = window;
	const inCorner =
		rect.width > 0 &&
		rect.right > innerWidth - CORNER_LANE_PX &&
		rect.bottom > innerHeight - BOTTOM_REACH_PX;
	return inCorner ? Math.max(0, Math.ceil(innerHeight - rect.top)) : 0;
}

/** Re-measures on size changes, and after animations that move the element without resizing it. */
function observeLayout(element: HTMLElement, measure: () => void) {
	let frame = 0;
	const schedule = () => {
		if (frame) return;
		frame = requestAnimationFrame(() => {
			frame = 0;
			measure();
		});
	};
	measure();
	const observer =
		typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
	observer?.observe(element);
	window.addEventListener("resize", schedule);
	document.addEventListener("animationend", schedule, true);
	document.addEventListener("transitionend", schedule, true);
	return () => {
		cancelAnimationFrame(frame);
		observer?.disconnect();
		window.removeEventListener("resize", schedule);
		document.removeEventListener("animationend", schedule, true);
		document.removeEventListener("transitionend", schedule, true);
	};
}

/**
 * Callback ref for a bottom-docked element such as a chat composer: while it
 * reaches into the bottom-right corner, corner overlays rise above it.
 */
export function cornerObstacleRef(element: HTMLElement | null) {
	if (!element) return;
	const key = Symbol("corner-obstacle");
	const stop = observeLayout(element, () => {
		obstacles.set(key, cornerInset(element));
		publish();
	});
	return () => {
		stop();
		obstacles.delete(key);
		publish();
	};
}

/** Callback ref for the corner overlay itself, so lifted overlays reserve their height. */
export function cornerOverlayRef(element: HTMLElement | null) {
	if (!element) return;
	const stop = observeLayout(element, () => {
		overlayHeight = element.offsetHeight;
		publish();
	});
	return () => {
		stop();
		overlayHeight = 0;
		publish();
	};
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

const getOffset = () => (clearance > 0 ? clearance + GAP_PX : 0);
const getServerOffset = () => 0;

/** How far above the viewport's bottom edge a corner overlay must sit; 0 when nothing is in the way. */
export function useCornerOffset(): number {
	return useSyncExternalStore(subscribe, getOffset, getServerOffset);
}
