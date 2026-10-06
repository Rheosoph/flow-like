/*
 * Moving the cursor from the dock. The ▲▼ buttons of "N fields need a look" and "N fields to fill in" go to a field by its
 * `data-fw-focus` hook inside the interface root, the same way the shell's focus executor finds it.
 */
import { FOCUS_ATTR, FOCUS_VALUE, type FieldKey } from "../contracts";

export const ROOT_SELECTOR = "[data-fw-root]";

const escapeValue = (value: string) =>
	typeof CSS !== "undefined" && typeof CSS.escape === "function"
		? CSS.escape(value)
		: value.replace(/["\\]/g, "\\$&");

/** The interface root an element sits in, else its document (the visual harness has no root). */
export function interfaceRootOf(element: Element | null): ParentNode | null {
	return element?.closest(ROOT_SELECTOR) ?? element?.ownerDocument ?? null;
}

export const fieldHook = (key: FieldKey) => `${FOCUS_VALUE.fieldPrefix}${key}`;

export function hookedElement(root: ParentNode, value: string) {
	return root.querySelector<HTMLElement>(
		`[${FOCUS_ATTR}="${escapeValue(value)}"]`,
	);
}

/** The FieldKey the cursor is in, or null when it is not in a field. */
export function focusedFieldKey(from: Element | null): FieldKey | null {
	const active = from?.ownerDocument.activeElement;
	const hooked = active?.closest(
		`[${FOCUS_ATTR}^="${FOCUS_VALUE.fieldPrefix}"]`,
	);
	const value = hooked?.getAttribute(FOCUS_ATTR);
	return value ? value.slice(FOCUS_VALUE.fieldPrefix.length) : null;
}

const SELECTABLE = new Set(["INPUT", "TEXTAREA"]);

function selectWhole(target: HTMLElement) {
	if (!SELECTABLE.has(target.tagName)) return;
	try {
		(target as HTMLInputElement | HTMLTextAreaElement).select();
	} catch {
		// inputs that take no selection (date, file) just keep the cursor
	}
}

/**
 * Puts the cursor in a field and brings it into view. Arriving from the dock selects a text control's whole value
 * (spec M2); giving the cursor back to a field keeps it where it was (`select` false).
 */
export function focusField(root: ParentNode, key: FieldKey, select = true) {
	const target = hookedElement(root, fieldHook(key));
	if (!target) return false;
	target.focus();
	target.scrollIntoView?.({ block: "nearest" });
	if (select) selectWhole(target);
	return true;
}
