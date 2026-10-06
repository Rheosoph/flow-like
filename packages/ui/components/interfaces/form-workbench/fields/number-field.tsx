"use client";

import type { ClipboardEvent, KeyboardEvent } from "react";
import { cleanAmount, nudge } from "../model/numbers";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { numberControl } from "./control-style";
import { handleEnter, useSelectOnTab } from "./enter";
import { isComposing, resetShortcut } from "./keys";

/** ↑/↓ change the number by one step, with ⇧ by ten (spec S3); ⌥ combinations stay with the browser. */
function nudgeOf(
	event: KeyboardEvent<HTMLInputElement>,
	props: ControlProps,
	text: string,
): string | null {
	if (event.altKey || event.metaKey || event.ctrlKey) return null;
	if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return null;
	const { field } = props;
	return nudge(
		text,
		event.key === "ArrowUp" ? 1 : -1,
		event.shiftKey,
		field.integer,
		textOf(field.defaultValue),
		field.range,
		field.step,
	);
}

/** The text with the pasted amount cleaned (currency marks, group signs, the viewer's decimal sign), or null to paste as is. */
export function pastedAmount(
	event: ClipboardEvent<HTMLInputElement>,
	props: ControlProps,
	text: string,
): string | null {
	const cleaned = cleanAmount(
		event.clipboardData.getData("text"),
		props.field.integer,
		props.viewer.decimalSign,
	);
	if (cleaned === null) return null;
	const { selectionStart, selectionEnd } = event.currentTarget;
	const start = selectionStart ?? text.length;
	const end = selectionEnd ?? start;
	return text.slice(0, start) + cleaned + text.slice(end);
}

/**
 * A number: mono, tabular, right-aligned, `inputMode` for the keypad. Strict parsing is the model's; the control
 * only types. ↑/↓ nudge within the pin's range and step, a pasted amount is cleaned, the wheel never changes it.
 */
export function NumberField(props: ControlProps) {
	const { field, rail, actions, bind, disabled } = props;
	const text = textOf(valueAt(rail.values, field.key));
	const select = useSelectOnTab<HTMLInputElement>();
	const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
		if (isComposing(event) || handleEnter(event, field.key, actions)) return;
		const next = nudgeOf(event, props, text);
		if (next === null) return;
		event.preventDefault();
		if (next !== text) actions.setValue(field.key, next);
	};
	return (
		<input
			id={bind.ids.control}
			type="text"
			inputMode={field.integer ? "numeric" : "decimal"}
			autoComplete="off"
			value={text}
			disabled={disabled}
			enterKeyHint={props.enterHint(field.key)}
			aria-invalid={bind.invalid || undefined}
			aria-describedby={bind.describedBy()}
			aria-keyshortcuts={
				bind.markers.reset ? resetShortcut(props.viewer.mac) : undefined
			}
			{...bind.focus}
			onPointerDown={select.onPointerDown}
			onFocus={select.onFocus}
			onChange={(event) => actions.setValue(field.key, event.target.value)}
			onKeyDown={onKeyDown}
			onPaste={(event) => {
				const next = pastedAmount(event, props, text);
				if (next === null) return;
				event.preventDefault();
				actions.setValue(field.key, next);
			}}
			onBlur={() => actions.blurField(field.key)}
			className={numberControl({
				touch: bind.touch,
				differs: bind.markers.differs,
			})}
		/>
	);
}
