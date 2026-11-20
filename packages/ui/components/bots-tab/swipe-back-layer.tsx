"use client";

import {
	type ReactNode,
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { EDGE_SWIPE_ZONE, useEdgeSwipeBack } from "./use-edge-swipe-back";

const DURATION_MS = 300;
const EASE = "cubic-bezier(0.32, 0.72, 0, 1)";

function reducedMotion() {
	return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function place(element: HTMLElement, fraction: number, animate: boolean) {
	element.style.transition = animate
		? `transform ${DURATION_MS}ms ${EASE}`
		: "";
	element.style.transform = `translate3d(${fraction * 100}%, 0, 0)`;
}

/**
 * A layer over a chat that goes back to it the iOS way: it slides in from the right, and its back
 * button or a swipe from its left edge slides it out before `onBack`. A swipe that starts on it never
 * reaches the dock's own swipe, so it closes this layer, not the chat.
 */
export function SwipeBackLayer({
	onBack,
	className,
	children,
}: Readonly<{
	onBack: () => void;
	className?: string;
	/** Gets the animated way back, for the layer's own back button. */
	children: (back: () => void) => ReactNode;
}>) {
	const [element, setElement] = useState<HTMLDivElement | null>(null);
	const leaving = useRef(false);
	const latestBack = useRef(onBack);
	latestBack.current = onBack;

	const leave = useCallback(() => {
		if (!element || leaving.current) return;
		leaving.current = true;
		if (reducedMotion()) {
			latestBack.current();
			return;
		}
		place(element, 1, true);
		window.setTimeout(() => latestBack.current(), DURATION_MS);
	}, [element]);

	useLayoutEffect(() => {
		if (!element || reducedMotion()) return;
		place(element, 1, false);
		const frame = requestAnimationFrame(() => place(element, 0, true));
		return () => cancelAnimationFrame(frame);
	}, [element]);

	useEffect(() => {
		if (!element) return;
		const keepFromDock = (event: TouchEvent) => {
			const touch = event.touches[0];
			const left = element.getBoundingClientRect().left;
			if (touch && touch.clientX - left <= EDGE_SWIPE_ZONE) {
				event.stopPropagation();
			}
		};
		element.addEventListener("touchstart", keepFromDock, { passive: true });
		return () => element.removeEventListener("touchstart", keepFromDock);
	}, [element]);

	useEdgeSwipeBack(element, true, {
		onDrag: (fraction) => {
			if (element) place(element, fraction, false);
		},
		onEnd: (commit) => {
			if (!element) return;
			if (commit) leave();
			else place(element, 0, true);
		},
	});

	return (
		<div
			ref={setElement}
			className={cn(
				"absolute inset-0 z-30 flex flex-col shadow-[-12px_0_32px_-16px_rgb(0_0_0/0.45)] will-change-transform",
				className,
			)}
		>
			{children(leave)}
		</div>
	);
}
