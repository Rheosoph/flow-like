"use client";

import { useEffect, useRef } from "react";

const EDITABLE = "input, textarea, select, [contenteditable]";
const OVERLAY =
	"[role=dialog], [role=alertdialog], [role=menu], [role=listbox]";
/** The rail's filter field (W2-CHROME marks it). */
export const RAIL_FILTER_SELECTOR = "[data-rail-filter]";

/** Typing in a field, or inside a sheet or menu that isn't the rail itself. */
function isClaimed(target: EventTarget | null): boolean {
	const element = target as Element | null;
	if (typeof element?.closest !== "function") return false;
	if (element.closest(EDITABLE)) return true;
	const overlay = element.closest(OVERLAY);
	return (
		overlay !== null && overlay.querySelector(RAIL_FILTER_SELECTOR) === null
	);
}

export function isFilterHotkey(event: KeyboardEvent): boolean {
	return (
		event.key === "/" &&
		!event.defaultPrevented &&
		!(event.metaKey || event.ctrlKey || event.altKey) &&
		!isClaimed(event.target)
	);
}

/** SPEC §3.4: `/` focuses the device filter while the area is mounted. `onFilter` returns whether it took the key. */
export function useAreaHotkeys({
	onFilter,
	enabled = true,
}: Readonly<{ onFilter(): boolean; enabled?: boolean }>): void {
	const handler = useRef(onFilter);
	useEffect(() => {
		handler.current = onFilter;
	}, [onFilter]);

	useEffect(() => {
		if (!enabled) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (isFilterHotkey(event) && handler.current()) event.preventDefault();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [enabled]);
}
