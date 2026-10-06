"use client";

import { type RefObject, useEffect, useLayoutEffect, useRef } from "react";
import {
	FOCUS_ATTR,
	FOCUS_VALUE,
	FORM_LIMITS,
	type FieldKey,
	type FocusRequest,
	type FocusTarget,
	type FormSessionState,
	QUIET_FOCUS_ATTR,
	type WorkbenchLayout,
} from "../contracts";
import { targets } from "../model/fields";
import { blockedNames } from "../model/validate";
import { isEmpty, valueAt } from "../model/values";

export const useIsoLayoutEffect =
	typeof window === "undefined" ? useEffect : useLayoutEffect;

const FOCUS_VALUE_OF: {
	readonly [K in FocusTarget["kind"]]: (
		target: Extract<FocusTarget, { kind: K }>,
	) => string;
} = {
	field: (target) => `${FOCUS_VALUE.fieldPrefix}${target.key}`,
	tab: (target) => `${FOCUS_VALUE.tabPrefix}${target.runId}`,
	run: () => FOCUS_VALUE.run,
	runAgain: () => FOCUS_VALUE.runAgain,
	stop: () => FOCUS_VALUE.stop,
	change: () => FOCUS_VALUE.change,
	copy: () => FOCUS_VALUE.copy,
	presetButton: () => FOCUS_VALUE.presetButton,
	filter: () => FOCUS_VALUE.filter,
};

/** The `data-fw-focus` value of the element a focus target names. */
export const focusValueOf = (target: FocusTarget): string =>
	(FOCUS_VALUE_OF[target.kind] as (target: FocusTarget) => string)(target);

/** Where focus goes when the named element is gone: Stop and Copy fall back to Run, Run to Run again and back. */
const FALLBACK_VALUES: Readonly<
	Partial<Record<FocusTarget["kind"], readonly string[]>>
> = {
	run: [FOCUS_VALUE.runAgain],
	runAgain: [FOCUS_VALUE.run],
	stop: [FOCUS_VALUE.run, FOCUS_VALUE.runAgain],
	copy: [FOCUS_VALUE.run, FOCUS_VALUE.runAgain],
	change: [FOCUS_VALUE.run],
};

const isDisabled = (element: HTMLElement) =>
	(element as HTMLButtonElement).disabled === true;

/** The first enabled element with this `data-fw-focus` value, in document order. */
export function findFocusable(root: ParentNode, value: string) {
	for (const element of root.querySelectorAll<HTMLElement>(`[${FOCUS_ATTR}]`)) {
		if (element.getAttribute(FOCUS_ATTR) === value && !isDisabled(element))
			return element;
	}
	return null;
}

export function resolveFocus(root: ParentNode, target: FocusTarget) {
	const wanted = [
		focusValueOf(target),
		...(FALLBACK_VALUES[target.kind] ?? []),
	];
	for (const value of wanted) {
		const found = findFocusable(root, value);
		if (found) return found;
	}
	return null;
}

/** Selects the whole value of a text control; anything else is left alone. */
export function selectContents(element: HTMLElement) {
	const control = element as HTMLInputElement;
	if (typeof control.select === "function") control.select();
}

/** Puts the caret after the value of a text control, nothing selected; anything else is left alone. */
export function caretAtEnd(element: HTMLElement) {
	const control = element as HTMLInputElement;
	if (typeof control.value !== "string") return;
	const end = control.value.length;
	try {
		control.setSelectionRange(end, end);
	} catch {
		return;
	}
}

/** Where focus was: the element and, for a text control, its selection (the caret). */
export interface FocusSpot {
	readonly element: HTMLElement;
	readonly selection: readonly [number, number] | null;
}

function selectionOf(element: HTMLElement): FocusSpot["selection"] {
	try {
		const { selectionStart, selectionEnd } = element as HTMLInputElement;
		if (typeof selectionStart !== "number" || typeof selectionEnd !== "number")
			return null;
		return [selectionStart, selectionEnd];
	} catch {
		return null;
	}
}

/** Where focus is now; null while only the page has it. */
export function focusSpotOf(doc: Document): FocusSpot | null {
	if (focusOnPage(doc)) return null;
	const element = doc.activeElement as HTMLElement;
	return { element, selection: selectionOf(element) };
}

function restoreSelection(spot: FocusSpot) {
	if (!spot.selection) return;
	try {
		const control = spot.element as HTMLInputElement;
		control.setSelectionRange(spot.selection[0], spot.selection[1]);
	} catch {
		return;
	}
}

/**
 * Gives focus back to where it was (a dialog or a menu closed): a text control gets its caret back, not
 * the select-all a field does when it is focused from outside. False when the element is gone.
 */
export function returnFocusTo(spot: FocusSpot | null) {
	if (!spot?.element.isConnected) return false;
	spot.element.focus();
	restoreSelection(spot);
	return spot.element.ownerDocument.activeElement === spot.element;
}

/**
 * Carries out a session focus request. A scroll-only field target (after a run) only scrolls on a box
 * without the split, so no keyboard opens; with the split it takes focus. A field selects its whole value
 * only when asked; otherwise the caret goes after it (a field selects on every arrival but a click). When
 * nothing matches, focus stays in the interface instead of falling to the page.
 */
export function applyFocusRequest(
	root: HTMLElement,
	request: FocusRequest,
	split = false,
) {
	const element = resolveFocus(root, request.target);
	if (!element) {
		root.focus({ preventScroll: true });
		return false;
	}
	const { target } = request;
	if (target.kind === "field" && target.scrollOnly && !split) {
		element.scrollIntoView?.({ block: "nearest" });
		return true;
	}
	element.focus();
	if (target.kind === "field")
		(target.select ? selectContents : caretAtEnd)(element);
	return true;
}

/** Executes `view.focus` after the render that carries it, then tells the session it was handled. */
export function useFocusExecutor(
	rootRef: RefObject<HTMLElement | null>,
	request: FocusRequest | null,
	onHandled: (seq: number) => void,
	split: boolean,
) {
	useIsoLayoutEffect(() => {
		if (!request) return;
		const root = rootRef.current;
		if (root && !root.querySelector("[data-fw-modal]"))
			applyFocusRequest(root, request, split);
		onHandled(request.seq);
	}, [request, rootRef, onHandled, split]);
}

type AutofocusState = Pick<
	FormSessionState,
	"form" | "rail" | "runs" | "memory"
>;

type AutofocusGate = (
	state: AutofocusState,
	layout: WorkbenchLayout | null,
) => boolean;

/** Every condition of S6 that needs no field list: a page, a split box with a fine pointer, history to go by. */
const AUTOFOCUS_GATES: readonly AutofocusGate[] = [
	(_state, layout) => layout?.split === true && layout.finePointer,
	(_state, layout) => layout?.touch === false,
	(state) => state.form.host.presentation === "page",
	(state) => state.form.fields.length > 0,
	(state) => state.memory.loaded && state.rail.tab === "inputs",
	(state) => state.runs.length >= FORM_LIMITS.autofocusFromRuns,
];

function firstStopKey(state: AutofocusState) {
	const { form, rail } = state;
	const stops = targets(
		form.fields,
		blockedNames(form.fields, form.host),
	).filter((stop) => stop.field.kind !== "unsupported");
	const empty = stops.find(
		(stop) =>
			stop.field.required &&
			isEmpty(stop.field, valueAt(rail.values, stop.key)),
	);
	return (empty ?? stops[0])?.key ?? null;
}

/**
 * S6: where the cursor starts for someone who has used this form before: the first empty required
 * field, else the first field. Only on a page (not a tile) in a split box with a fine pointer, for a
 * form with fields and at least two runs on this device, once the device memory is in.
 */
export function autofocusKey(
	state: AutofocusState,
	layout: WorkbenchLayout | null,
): FieldKey | null {
	return AUTOFOCUS_GATES.every((gate) => gate(state, layout))
		? firstStopKey(state)
		: null;
}

/** Nothing in the document has focus but the page itself. */
export function focusOnPage(doc: Document) {
	const active = doc.activeElement;
	return !active || active === doc.body || active === doc.documentElement;
}

/** Nothing has focus yet, or only the page or the interface root itself. */
function focusIsFree(root: HTMLElement) {
	return (
		focusOnPage(root.ownerDocument) || root.ownerDocument.activeElement === root
	);
}

/**
 * Puts the cursor in the form when it opens (S6) and hides the focus ring until the first key or click.
 * The caret goes after the value: a selection would put a second block of colour beside Run.
 */
export function useAutofocus(
	rootRef: RefObject<HTMLElement | null>,
	state: AutofocusState,
	layout: WorkbenchLayout | null,
) {
	const done = useRef(false);
	useEffect(() => {
		if (done.current) return;
		const key = autofocusKey(state, layout);
		if (key === null) return;
		done.current = true;
		const root = rootRef.current;
		if (!root || !focusIsFree(root)) return;
		const element = findFocusable(root, `${FOCUS_VALUE.fieldPrefix}${key}`);
		if (!element) return;
		root.setAttribute(QUIET_FOCUS_ATTR, "");
		element.focus();
		caretAtEnd(element);
	});
}

/** The first key or click gives the focus ring back (S6). */
export function showFocusRings(root: Element | null) {
	root?.removeAttribute(QUIET_FOCUS_ATTR);
}

const QUIET = `[data-fw-root][${QUIET_FOCUS_ATTR}]`;

/** A field's (`field`) or an object property's (`prop`) frame while the pointer is not over it. */
const restingFrame = (scope: "field" | "prop") =>
	`${QUIET} .group\\/${scope}:not(:hover)`;

/**
 * While the root carries the quiet attribute (S6) no focus shows: no ring on the focused element or on a box
 * that holds it (`focus-within`, `:has(:focus-visible)`; one `:is()` list, so a browser without `:has` still
 * hides the rest), and a label line's focus reveals stay as the hover alone sets them ("Reset" / "Clear"
 * hidden, "Optional" shown). No child combinator and no quotes, so server rendering cannot mangle it.
 */
export const QUIET_FOCUS_CSS = [
	`${QUIET} :is(:focus,:focus-visible,:focus-within,:has(:focus-visible)){outline:none !important;box-shadow:none !important}`,
	`${restingFrame("field")} [data-fw-reveal=field],${restingFrame("prop")} [data-fw-reveal=prop]{opacity:0 !important;pointer-events:none !important}`,
	`${restingFrame("field")} [data-fw-reveal-hide=field],${restingFrame("prop")} [data-fw-reveal-hide=prop]{visibility:visible !important}`,
].join("");

/**
 * Selected text in the form and its dialogs is neutral, not the app's coral: coral stays the one primary
 * action and the ring (SURFACE). The text colour is set, not inherited, because a browser with highlight
 * inheritance would take the app's white selection text. Same rules for server rendering as above.
 */
export const SELECTION_CSS =
	"[data-fw-root] ::selection,[data-fw-modal] ::selection{background:color-mix(in oklab,var(--foreground) 18%,transparent);color:var(--foreground)}";
