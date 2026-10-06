import { act } from "react";
import { fire } from "../testing/dom";

/*
 * Events as a person makes them, for DOM tests of the field controls: a cancelable keydown whose
 * `defaultPrevented` tells whether the control took the key, and focus and blur inside `act`.
 */

/** A cancelable keydown, so a test can see whether the control took the key (`defaultPrevented`). */
export async function press(
	element: Element,
	key: string,
	init: KeyboardEventInit = {},
) {
	const view = element.ownerDocument
		.defaultView as unknown as typeof globalThis;
	const event = new view.KeyboardEvent("keydown", {
		key,
		bubbles: true,
		cancelable: true,
		...init,
	});
	await fire(element, event);
	return event;
}

/** Focus and blur the way a person's cursor does: inside `act`, so the control's updates flush. */
export async function focusOn(element: HTMLElement) {
	await act(async () => element.focus());
}

export async function blurOff(element: HTMLElement) {
	await act(async () => element.blur());
}
