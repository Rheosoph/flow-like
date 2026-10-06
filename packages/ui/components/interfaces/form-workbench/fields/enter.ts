import type {
	FocusEvent,
	KeyboardEvent,
	PointerEvent as ReactPointerEvent,
} from "react";
import { useCallback, useRef } from "react";
import type { FieldKey, FormSessionActions } from "../contracts";
import { isComposing, isPlainEnter } from "./keys";

/**
 * ↵ moves on (spec M2): `enter(key)` asks the reducer for the next empty required field, or to run. A held ↵ is
 * swallowed without asking again, an open IME composition owns its ↵. Returns whether the key was taken.
 */
export function handleEnter(
	event: KeyboardEvent,
	key: FieldKey,
	actions: Pick<FormSessionActions, "enter">,
): boolean {
	if (isComposing(event) || !isPlainEnter(event)) return false;
	event.preventDefault();
	if (!event.repeat) actions.enter(key);
	return true;
}

type SelectableField = HTMLInputElement | HTMLTextAreaElement;

/** A focus that follows a pointer press within this many ms came from the pointer. */
const POINTER_FOCUS_MS = 400;

/**
 * Arriving by Tab selects the whole value (spec M2): textareas do not do that by themselves. A pointer press
 * focuses without selecting, so a click still places the caret where it landed.
 */
export function useSelectOnTab<T extends SelectableField>() {
	const pressedAt = useRef(Number.NEGATIVE_INFINITY);
	const onPointerDown = useCallback((_event: ReactPointerEvent<T>) => {
		pressedAt.current = Date.now();
	}, []);
	const onFocus = useCallback((event: FocusEvent<T>) => {
		const byPointer = Date.now() - pressedAt.current < POINTER_FOCUS_MS;
		if (!byPointer) event.currentTarget.select();
	}, []);
	return { onPointerDown, onFocus };
}
