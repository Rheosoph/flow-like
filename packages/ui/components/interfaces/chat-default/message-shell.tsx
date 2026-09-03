"use client";

import {
	type ReactNode,
	type RefObject,
	useEffect,
	useRef,
	useState,
} from "react";

/** How far from the scroll container's edge a row mounts: 1.5 viewports either side. */
const REVEAL_MARGIN = "150% 0px";

const PLACEHOLDER_MIN_HEIGHT = 96;

const canObserve = () => typeof IntersectionObserver !== "undefined";

/**
 * Defers mounting a settled chat row until it scrolls near the viewport, so
 * opening a long session parses and mounts only the rows around the initial
 * scroll position. Reveal is per row and observer-driven — dragging the
 * scrollbar to the top mounts what it passes near, not everything between —
 * and a revealed row never returns to a placeholder.
 */
export function MessageShell({
	children,
	immediate = false,
	root,
}: Readonly<{
	children: ReactNode;
	/** Mount at once without observing; used for the rows the initial scroll lands on. */
	immediate?: boolean;
	/** The scroll container; without one the viewport is the root. */
	root?: RefObject<Element | null>;
}>) {
	const [revealed, setRevealed] = useState(() => immediate || !canObserve());
	const placeholderRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (revealed) return;

		const element = placeholderRef.current;
		if (immediate || !canObserve() || !element) {
			setRevealed(true);
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((entry) => entry.isIntersecting)) setRevealed(true);
			},
			{ root: root?.current ?? null, rootMargin: REVEAL_MARGIN },
		);
		observer.observe(element);

		return () => observer.disconnect();
	}, [immediate, revealed, root]);

	if (revealed) return <>{children}</>;

	return (
		<div
			ref={placeholderRef}
			aria-busy="true"
			data-fl-chat-message-placeholder
			style={{ minHeight: PLACEHOLDER_MIN_HEIGHT }}
		/>
	);
}
