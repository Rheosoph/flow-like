import type { KeyboardEvent } from "react";

/** Every key is ignored while an IME composition is open (spec §4). */
export const isComposing = (event: KeyboardEvent<HTMLElement>) =>
	event.nativeEvent.isComposing || event.keyCode === 229;

/** A held Enter or Space must not press Run again, Try again or Use these inputs again and again (spec M2). */
export function guardRepeat(event: KeyboardEvent<HTMLElement>) {
	if (isComposing(event)) return;
	if (event.repeat && (event.key === "Enter" || event.key === " "))
		event.preventDefault();
}
