import { type Ref, useMemo } from "react";

/** Keep a renderer's ref and the editor's ref attached to the same element. */
export function useElementRef<T extends HTMLElement | SVGElement>(
	elementRef: ((element: HTMLElement | SVGElement | null) => void) | undefined,
	localRef: Ref<T>,
) {
	return useMemo(() => {
		return (element: T | null) => {
			let cleanup: (() => void) | undefined;
			if (typeof localRef === "function") {
				const result = localRef(element);
				if (typeof result === "function") cleanup = result;
			} else if (localRef) {
				localRef.current = element;
			}
			elementRef?.(element);
			return () => {
				if (typeof cleanup === "function") {
					cleanup();
				} else if (typeof localRef === "function") {
					localRef(null);
				} else if (localRef) {
					localRef.current = null;
				}
				elementRef?.(null);
			};
		};
	}, [elementRef, localRef]);
}
