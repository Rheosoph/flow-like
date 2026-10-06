"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { lineControl, numberControl, textButton } from "./control-style";
import { handleEnter, useSelectOnTab } from "./enter";
import { resetShortcut } from "./keys";
import { usePlaceholder } from "./text-field";

/** The Show / Hide toggle at the right end of a masked input (spec M6: `aria-pressed`, a Tab stop of its own). */
function Toggle({
	label,
	shown,
	touch,
	onToggle,
}: Readonly<{
	label: string;
	shown: boolean;
	touch: boolean;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const word = shown
		? t("workbench.field.hide", "Hide")
		: t("workbench.field.show", "Show");
	const name = shown
		? t("workbench.field.hideAria", "Hide {{label}}", { label })
		: t("workbench.field.showAria", "Show {{label}}", { label });
	return (
		<button
			type="button"
			aria-pressed={shown}
			aria-label={name}
			onClick={onToggle}
			className={cx(
				textButton(touch),
				"absolute top-1/2 right-1.5 -translate-y-1/2 px-2",
			)}
		>
			{word}
		</button>
	);
}

/**
 * A sensitive text or number (spec M6, 10.1): a masked input with Show / Hide, never a suggestion, never a
 * recent value. A withheld default reads as the placeholder of an optional field.
 */
export function MaskedField(props: ControlProps) {
	const { field, rail, actions, bind, disabled } = props;
	const [shown, setShown] = useState(false);
	const text = textOf(valueAt(rail.values, field.key));
	const select = useSelectOnTab<HTMLInputElement>();
	const placeholder = usePlaceholder(props, text === "", false);
	const number = field.kind === "number";
	const style = { touch: bind.touch, differs: bind.markers.differs };
	return (
		<div className="relative">
			<input
				id={bind.ids.control}
				type={shown ? "text" : "password"}
				inputMode={number ? (field.integer ? "numeric" : "decimal") : undefined}
				autoComplete="off"
				autoCapitalize="off"
				autoCorrect="off"
				spellCheck={false}
				data-1p-ignore=""
				data-lpignore="true"
				value={text}
				disabled={disabled}
				placeholder={placeholder}
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
				onKeyDown={(event) => handleEnter(event, field.key, actions)}
				onBlur={() => actions.blurField(field.key)}
				className={cx(
					number ? numberControl(style) : lineControl(style),
					"pr-16",
				)}
			/>
			<Toggle
				label={field.label}
				shown={shown}
				touch={bind.touch}
				onToggle={() => setShown((current) => !current)}
			/>
		</div>
	);
}
