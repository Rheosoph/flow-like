"use client";

import {
	type UIEvent,
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";

/** Scroll positions by list; they outlive the component, so a pane switch or a remount finds its place again. */
const positions = new Map<string, number>();

export interface ScrollMemory {
	readonly ref: (element: HTMLElement | null) => void;
	readonly onScroll: (event: UIEvent<HTMLElement>) => void;
	/** The content is taller than the box: the list shows its top hairline. */
	readonly over: boolean;
}

/**
 * Keeps a scroll container's position across tab and pane switches (the field list keeps its place)
 * and reports whether its content overflows, which draws the hairline under the rail's head.
 */
export function useScrollMemory(key: string): ScrollMemory {
	const element = useRef<HTMLElement | null>(null);
	const [over, setOver] = useState(false);

	const ref = useCallback((node: HTMLElement | null) => {
		element.current = node;
	}, []);

	const measure = useCallback(() => {
		const node = element.current;
		if (node) setOver(node.scrollHeight > node.clientHeight + 1);
	}, []);

	useLayoutEffect(() => {
		const node = element.current;
		const saved = positions.get(key) ?? 0;
		if (node && saved > 0) node.scrollTop = saved;
	}, [key]);

	useEffect(() => {
		const node = element.current;
		if (!node) return;
		measure();
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(measure);
		observer.observe(node);
		if (node.firstElementChild) observer.observe(node.firstElementChild);
		return () => observer.disconnect();
	}, [measure]);

	const onScroll = useCallback(
		(event: UIEvent<HTMLElement>) => {
			positions.set(key, event.currentTarget.scrollTop);
		},
		[key],
	);

	return { ref, onScroll, over };
}

/** Forget every saved position (tests). */
export function clearScrollMemory() {
	positions.clear();
}
