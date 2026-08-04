import type React from "react";
import { useCallback, useRef } from "react";

/**
 * Merges a forwarded ref with a local one. The returned setter has to be attached to the node so
 * both are populated during commit: animation libraries (framer-motion's `motion.create`) read the
 * forwarded ref in a layout effect, which runs before any passive effect could fill it in — a ref
 * handed over later leaves the element frozen on its `initial` styles.
 */
export function useForwardedRef<T>(ref: React.ForwardedRef<T>) {
	const innerRef = useRef<T | null>(null);

	const setRef = useCallback(
		(node: T | null) => {
			innerRef.current = node;
			if (typeof ref === "function") {
				ref(node);
			} else if (ref) {
				ref.current = node;
			}
		},
		[ref],
	);

	return [innerRef, setRef] as const;
}
