"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../settings/devices/primitives/tone";
import { valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { rowControl } from "./control-style";
import { handleEnter } from "./enter";

/** The switch's track and knob: drawn, never focusable (spec M2: Space toggles, ↵ moves on and never toggles). */
function Track({ on, touch }: Readonly<{ on: boolean; touch: boolean }>) {
	const knob = touch ? "size-4" : "size-3.5";
	const x = touch ? (on ? "left-4.5" : "left-0.5") : on ? "left-4" : "left-0.5";
	return (
		<span
			aria-hidden
			className={cx(
				"relative shrink-0 rounded-full",
				touch ? "h-5 w-9" : "h-4.5 w-8",
				on ? "bg-foreground" : "bg-unknown-solid",
			)}
		>
			<span
				className={cx(
					"absolute top-0.5 rounded-full",
					knob,
					x,
					on ? "bg-background" : "bg-card",
				)}
			/>
		</span>
	);
}

/** A full-width switch row. On a phone the row carries the label; on a desktop the label line above does. */
export function BoolField(props: ControlProps) {
	const { t } = useTranslation("interfaces");
	const { field, rail, actions, bind, disabled } = props;
	const on = valueAt(rail.values, field.key) === true;
	const word = on
		? t("workbench.field.on", "On")
		: t("workbench.field.off", "Off");
	return (
		<button
			type="button"
			role="switch"
			id={bind.ids.control}
			aria-checked={on}
			aria-labelledby={bind.touch ? undefined : bind.ids.label}
			aria-describedby={bind.describedBy()}
			disabled={disabled}
			{...bind.focus}
			onClick={() => actions.setValue(field.key, !on)}
			onKeyDown={(event) => handleEnter(event, field.key, actions)}
			className={cx(
				rowControl({
					touch: bind.touch,
					differs: bind.markers.differs,
					className: "gap-2.5 text-[13.5px]/5",
				}),
				bind.touch ? "px-3 text-sm/5" : "justify-between pr-2.25 pl-2.75",
			)}
		>
			{bind.touch ? (
				<span className="min-w-0 flex-1 font-medium">{field.label}</span>
			) : null}
			<span className={bind.touch ? "text-muted-foreground" : undefined}>
				{word}
			</span>
			<Track on={on} touch={bind.touch} />
		</button>
	);
}
