"use client";

import {
	type KeyboardEvent,
	type MouseEvent,
	type PointerEvent,
	useCallback,
	useEffect,
	useRef,
} from "react";

/** Keep an activation bound to the control the user pressed, even across a render. */
export function useActivationSnapshot<T extends HTMLElement>(
	onActivate: (event: MouseEvent<T>) => void,
) {
	const pending = useRef<
		| {
				activate: typeof onActivate;
				kind: "pointer" | "keyboard";
				dispose?: () => void;
		  }
		| undefined
	>(undefined);
	const clear = useCallback(() => {
		pending.current?.dispose?.();
		pending.current = undefined;
	}, []);
	useEffect(() => clear, [clear]);

	return {
		onPointerDown: (event: PointerEvent<T>) => {
			clear();
			if (event.button !== 0 || event.isPrimary === false) return;
			const element = event.currentTarget;
			const document = element.ownerDocument;
			const release = (up: globalThis.PointerEvent) => {
				if (!element.contains(up.target as Node)) clear();
			};
			// Only an active gesture installs document listeners. A release elsewhere
			// or a new press must not leave an old action waiting for a later click.
			document.addEventListener("pointerup", release, true);
			document.addEventListener("pointercancel", clear, true);
			document.addEventListener("pointerdown", clear, true);
			pending.current = {
				activate: onActivate,
				kind: "pointer",
				dispose: () => {
					document.removeEventListener("pointerup", release, true);
					document.removeEventListener("pointercancel", clear, true);
					document.removeEventListener("pointerdown", clear, true);
				},
			};
		},
		onPointerCancel: clear,
		onBlur: clear,
		onKeyDown: (event: KeyboardEvent<T>) => {
			if (event.key === "Enter" || event.key === " ") {
				if (!event.repeat || pending.current?.kind !== "keyboard") {
					clear();
					pending.current = { activate: onActivate, kind: "keyboard" };
				}
			} else {
				clear();
			}
		},
		onClick: (event: MouseEvent<T>) => {
			const intent = pending.current;
			clear();
			if (intent && (intent.kind === "keyboard" || event.detail > 0)) {
				intent.activate(event);
			} else if (event.isTrusted && event.detail === 0) {
				// Assistive technology can activate a control without pointer or key events.
				onActivate(event);
			} else {
				event.preventDefault();
			}
		},
	};
}
