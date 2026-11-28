import { useEffect, useRef } from "react";

export const EDGE_SWIPE_ZONE = 20;
export const EDGE_SWIPE_COMMIT_FRACTION = 0.35;
export const EDGE_SWIPE_COMMIT_VELOCITY = 0.5;
const LOCK_DISTANCE = 8;
const VELOCITY_WINDOW_MS = 100;

export function swipeCommits(
	distance: number,
	width: number,
	velocity: number,
): boolean {
	if (width <= 0 || distance <= 0) return false;
	return (
		distance / width >= EDGE_SWIPE_COMMIT_FRACTION ||
		(velocity >= EDGE_SWIPE_COMMIT_VELOCITY && distance >= EDGE_SWIPE_ZONE)
	);
}

export type EdgeSwipeLock = "horizontal" | "abort" | undefined;

export function lockSwipe(dx: number, dy: number): EdgeSwipeLock {
	if (Math.abs(dx) < LOCK_DISTANCE && Math.abs(dy) < LOCK_DISTANCE)
		return undefined;
	return dx > 0 && dx > Math.abs(dy) ? "horizontal" : "abort";
}

type Sample = { x: number; t: number };

export type EdgeSwipeHandlers = {
	/** Fraction of the width dragged, 0 to 1. */
	onDrag: (fraction: number) => void;
	onEnd: (commit: boolean, fraction: number) => void;
};

/** A touch starting at the element's left edge and dragged right; vertical and mid-screen touches are left alone. */
export function useEdgeSwipeBack(
	element: HTMLElement | null,
	enabled: boolean,
	handlers: EdgeSwipeHandlers,
) {
	const latest = useRef(handlers);
	latest.current = handlers;

	useEffect(() => {
		if (!element || !enabled) return;
		let startX = 0;
		let startY = 0;
		let width = 0;
		let candidate = false;
		let locked = false;
		let samples: Sample[] = [];

		const reset = () => {
			candidate = false;
			locked = false;
			samples = [];
		};

		const onStart = (event: TouchEvent) => {
			reset();
			if (event.touches.length !== 1) return;
			const touch = event.touches[0];
			const rect = element.getBoundingClientRect();
			if (
				touch.clientX - rect.left > EDGE_SWIPE_ZONE ||
				touch.clientX < rect.left
			)
				return;
			candidate = true;
			startX = touch.clientX;
			startY = touch.clientY;
			width = rect.width;
			samples = [{ x: touch.clientX, t: event.timeStamp }];
		};

		const onMove = (event: TouchEvent) => {
			if (!candidate) return;
			if (event.touches.length !== 1) {
				if (locked) latest.current.onEnd(false, 0);
				reset();
				return;
			}
			const touch = event.touches[0];
			const dx = touch.clientX - startX;
			if (!locked) {
				const lock = lockSwipe(dx, touch.clientY - startY);
				if (lock === "abort") {
					reset();
					return;
				}
				if (!lock) return;
				locked = true;
			}
			if (event.cancelable) event.preventDefault();
			samples.push({ x: touch.clientX, t: event.timeStamp });
			const cutoff = event.timeStamp - VELOCITY_WINDOW_MS;
			while (samples.length > 2 && samples[0].t < cutoff) samples.shift();
			latest.current.onDrag(Math.min(1, Math.max(0, dx / width)));
		};

		const onEnd = (event: TouchEvent) => {
			if (!locked) {
				reset();
				return;
			}
			const touch = event.changedTouches[0];
			const dx = Math.max(0, (touch?.clientX ?? startX) - startX);
			const first = samples[0];
			const last = samples[samples.length - 1];
			const span = last.t - first.t;
			const velocity = span > 0 ? (last.x - first.x) / span : 0;
			const commit = swipeCommits(dx, width, velocity);
			reset();
			latest.current.onEnd(commit, Math.min(1, dx / width));
		};

		const onCancel = () => {
			if (locked) latest.current.onEnd(false, 0);
			reset();
		};

		element.addEventListener("touchstart", onStart, { passive: true });
		element.addEventListener("touchmove", onMove, { passive: false });
		element.addEventListener("touchend", onEnd, { passive: true });
		element.addEventListener("touchcancel", onCancel, { passive: true });
		return () => {
			element.removeEventListener("touchstart", onStart);
			element.removeEventListener("touchmove", onMove);
			element.removeEventListener("touchend", onEnd);
			element.removeEventListener("touchcancel", onCancel);
		};
	}, [element, enabled]);
}
