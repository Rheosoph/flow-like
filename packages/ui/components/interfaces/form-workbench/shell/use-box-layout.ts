"use client";

import {
	type RefObject,
	useCallback,
	useLayoutEffect,
	useMemo,
	useState,
	useSyncExternalStore,
} from "react";
import { LAYOUT, type WorkbenchLayout } from "../contracts";

export interface BoxSize {
	readonly width: number;
	readonly height: number;
}

export interface PointerFlags {
	readonly touch: boolean;
	readonly finePointer: boolean;
}

/** The layout of an interface box: split from LAYOUT.splitMinWidth, compare when the stage beside the rail is wide enough. */
export function layoutOf(
	size: BoxSize,
	pointer: PointerFlags,
): WorkbenchLayout {
	const width = Math.floor(size.width);
	const split = width >= LAYOUT.splitMinWidth;
	return {
		width,
		height: Math.floor(size.height),
		split,
		touch: pointer.touch,
		finePointer: pointer.finePointer,
		compare: split && width - LAYOUT.railWidth >= LAYOUT.compareMinStageWidth,
	};
}

export function sameLayout(
	a: WorkbenchLayout | null | undefined,
	b: WorkbenchLayout | null | undefined,
) {
	if (!a || !b) return a === b;
	return (
		a.width === b.width &&
		a.height === b.height &&
		a.split === b.split &&
		a.touch === b.touch &&
		a.finePointer === b.finePointer &&
		a.compare === b.compare
	);
}

const COARSE = "(pointer: coarse)";
const FINE = "(pointer: fine)";

function matches(query: string, whenUnknown: boolean) {
	if (typeof window === "undefined" || typeof window.matchMedia !== "function")
		return whenUnknown;
	return window.matchMedia(query).matches;
}

function listenTo(query: string, notify: () => void) {
	if (typeof window === "undefined" || typeof window.matchMedia !== "function")
		return () => {};
	const list = window.matchMedia(query);
	list.addEventListener("change", notify);
	return () => list.removeEventListener("change", notify);
}

function useMediaMatch(query: string, whenUnknown: boolean) {
	const subscribe = useCallback(
		(notify: () => void) => listenTo(query, notify),
		[query],
	);
	return useSyncExternalStore(
		subscribe,
		() => matches(query, whenUnknown),
		() => whenUnknown,
	);
}

/**
 * Measures the interface root (ResizeObserver) and the pointer (`(pointer: coarse)`, `(pointer: fine)`)
 * into a `WorkbenchLayout`; null until the box has a width. Never reads the viewport. The box is read
 * once before the first paint, so the first frame already has its layout; `initialSize` does the same
 * where there is no layout engine (tests).
 */
export function useBoxLayout(
	ref: RefObject<HTMLElement | null>,
	initialSize?: BoxSize,
) {
	const touch = useMediaMatch(COARSE, false);
	const finePointer = useMediaMatch(FINE, true);
	const [size, setSize] = useState<BoxSize | null>(initialSize ?? null);

	useLayoutEffect(() => {
		const element = ref.current;
		if (!element) return;
		const take = (box: { width: number; height: number }) => {
			const width = Math.floor(box.width);
			const height = Math.floor(box.height);
			if (width <= 0) return;
			setSize((previous) =>
				previous && previous.width === width && previous.height === height
					? previous
					: { width, height },
			);
		};
		take(element.getBoundingClientRect());
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver((entries) => {
			const entry = entries[entries.length - 1];
			if (entry) take(entry.contentRect);
		});
		observer.observe(element);
		return () => observer.disconnect();
	}, [ref]);

	return useMemo(
		() => (size ? layoutOf(size, { touch, finePointer }) : null),
		[size, touch, finePointer],
	);
}
