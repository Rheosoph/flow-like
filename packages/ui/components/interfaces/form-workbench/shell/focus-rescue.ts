"use client";

import {
	type FocusEvent,
	type RefObject,
	useCallback,
	useEffect,
	useRef,
} from "react";
import { FOCUS_ATTR, FOCUS_VALUE, type FormSessionState } from "../contracts";
import { stageRunOf } from "../run/run-view";
import { findFocusable, focusOnPage, useIsoLayoutEffect } from "./focus";
import { useLatest } from "./use-latest";

/*
 * Focus never falls to the page (PLAN §7): the root remembers the element that had focus last. When that
 * element leaves the DOM (inside the root, or a portal at the end of the page) and the page is left holding
 * focus, focus comes back into the interface. The rescue waits a frame and a task, so whatever hands focus
 * back on purpose goes first: a closing dialog (its opener), Radix popovers and menus (their trigger), the
 * dock (Run).
 */

/** One step of the way back: a `data-fw-focus` value, or the selected run tab. */
export type RescueStep = string | { readonly kind: "selectedTab" };

type RescueSource = Pick<FormSessionState, "runs" | "view">;

const SELECTED_TAB: RescueStep = { kind: "selectedTab" };
const HOME: readonly RescueStep[] = [FOCUS_VALUE.run, FOCUS_VALUE.runAgain];
const AFTER_STOPPED: readonly RescueStep[] = [
	FOCUS_VALUE.runAgain,
	FOCUS_VALUE.copy,
];
const AFTER_ENDED: readonly RescueStep[] = [
	FOCUS_VALUE.copy,
	FOCUS_VALUE.runAgain,
];
const SELECTED_TAB_SELECTOR = `[${FOCUS_ATTR}^="${FOCUS_VALUE.tabPrefix}"][aria-selected="true"]`;

/**
 * Where focus goes when the element that had it is gone: the same hook again (the element was drawn anew,
 * a file row after Undo), after Stop the stopped run's Run again or the ended run's Copy, a run tab's
 * selected neighbour, then Run. The interface root is the last resort.
 */
export function rescueSteps(
	hook: string | null,
	source: RescueSource,
): readonly RescueStep[] {
	if (hook === null) return HOME;
	if (hook === FOCUS_VALUE.stop) {
		const stopped = stageRunOf(source)?.status === "stopped";
		return [hook, ...(stopped ? AFTER_STOPPED : AFTER_ENDED), ...HOME];
	}
	if (hook.startsWith(FOCUS_VALUE.tabPrefix))
		return [hook, SELECTED_TAB, ...HOME];
	return [hook, ...HOME];
}

function findStep(root: HTMLElement, step: RescueStep) {
	if (typeof step === "string") return findFocusable(root, step);
	return root.querySelector<HTMLElement>(SELECTED_TAB_SELECTOR);
}

/** Puts focus back into the interface; returns the element that took it (one that cannot is skipped). */
export function rescueFocus(
	root: HTMLElement,
	hook: string | null,
	source: RescueSource,
): HTMLElement {
	for (const step of rescueSteps(hook, source)) {
		const element = findStep(root, step);
		element?.focus();
		if (element && root.ownerDocument.activeElement === element) return element;
	}
	root.focus({ preventScroll: true });
	return root;
}

/** Runs `then` after the next frame and one more task; returns a cancel. */
function afterRestores(then: () => void): () => void {
	let timer: ReturnType<typeof setTimeout> | null = null;
	const frame = requestAnimationFrame(() => {
		timer = setTimeout(then, 0);
	});
	return () => {
		cancelAnimationFrame(frame);
		if (timer !== null) clearTimeout(timer);
	};
}

interface Held {
	readonly element: HTMLElement;
	readonly hook: string | null;
}

const hookOf = (element: HTMLElement) =>
	element.closest(`[${FOCUS_ATTR}]`)?.getAttribute(FOCUS_ATTR) ?? null;

/**
 * The root's focus guard. `onFocus` and `onBlur` go on the interface root (React's focus events also bubble
 * out of portals, so popovers and menus count). A blur that leaves the interface on purpose (another element
 * outside, or a click on the page) forgets the element; a window that loses focus keeps it.
 */
export function useFocusRescue(
	rootRef: RefObject<HTMLElement | null>,
	state: FormSessionState,
) {
	const source = useLatest<RescueSource>(state);
	const held = useRef<Held | null>(null);
	const pending = useRef<(() => void) | null>(null);

	const lost = useCallback(() => {
		const root = rootRef.current;
		const last = held.current;
		return (
			root !== null &&
			last !== null &&
			!last.element.isConnected &&
			focusOnPage(root.ownerDocument)
		);
	}, [rootRef]);

	const check = useCallback(() => {
		if (pending.current !== null || !lost()) return;
		pending.current = afterRestores(() => {
			pending.current = null;
			const root = rootRef.current;
			if (root?.isConnected && lost())
				rescueFocus(root, held.current?.hook ?? null, source.current);
		});
	}, [lost, rootRef, source]);

	useIsoLayoutEffect(check);

	useEffect(() => {
		const root = rootRef.current;
		if (!root || typeof MutationObserver === "undefined") return;
		const observer = new MutationObserver(check);
		observer.observe(root, { childList: true, subtree: true });
		const body = root.ownerDocument.body;
		if (body && !root.contains(body))
			observer.observe(body, { childList: true });
		return () => {
			observer.disconnect();
			pending.current?.();
			pending.current = null;
		};
	}, [rootRef, check]);

	const onFocus = useCallback((event: FocusEvent<HTMLElement>) => {
		const element = event.target as HTMLElement;
		held.current = { element, hook: hookOf(element) };
	}, []);

	const onBlur = useCallback((event: FocusEvent<HTMLElement>) => {
		const element = event.target as HTMLElement;
		queueMicrotask(() => {
			const left =
				held.current?.element === element &&
				element.isConnected &&
				element.ownerDocument.activeElement !== element;
			if (left) held.current = null;
		});
	}, []);

	return { onFocus, onBlur };
}
