"use client";

import { type RefObject, useEffect, useRef } from "react";
import { observeRuntimeTailwind } from "./runtime-tailwind";

export function useRuntimeTailwindStyles(
	rootRef: RefObject<HTMLElement | null>,
): void {
	const active = useRef<{
		root: HTMLElement;
		disconnect: () => void;
	} | null>(null);
	// Reattach after a render that replaces the root, including an empty surface
	// receiving its first component or a preview moving into an iframe.
	useEffect(() => {
		const root = rootRef.current;
		if (active.current?.root === root) return;
		active.current?.disconnect();
		active.current = root
			? { root, disconnect: observeRuntimeTailwind(root) }
			: null;
	});
	useEffect(
		() => () => {
			active.current?.disconnect();
			active.current = null;
		},
		[],
	);
}
