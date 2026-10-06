"use client";

import { useTranslation } from "@flow-like/locales";
import { type KeyboardEvent, useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { fitsSegments, targetOf } from "./choice-model";
import { DIFFERS_EDGE } from "./control-style";
import { handleEnter } from "./enter";
import { isComposing } from "./keys";

function Segmented({
	props,
	options,
}: Readonly<{ props: ControlProps; options: readonly string[] }>) {
	const { field, rail, actions, bind, disabled } = props;
	const value = textOf(valueAt(rail.values, field.key));
	const checked = options.indexOf(value);
	const stop = checked >= 0 ? checked : 0;
	const radios = useRef<(HTMLButtonElement | null)[]>([]);
	const pick = (at: number) => {
		actions.setValue(field.key, options[at]);
		radios.current[at]?.focus();
	};
	const onKeyDown = (event: KeyboardEvent) => {
		if (isComposing(event) || handleEnter(event, field.key, actions)) return;
		const at = targetOf(options, checked, event);
		if (at < 0) return;
		event.preventDefault();
		pick(at);
	};
	return (
		<div
			role="radiogroup"
			aria-labelledby={bind.ids.label}
			aria-invalid={bind.invalid || undefined}
			aria-describedby={bind.describedBy()}
			style={{
				gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))`,
			}}
			className={cx(
				"grid gap-0.5 rounded-lg border border-input bg-card p-0.5",
				bind.touch ? "h-11.5 gap-0 p-0" : "h-9",
				bind.markers.differs && DIFFERS_EDGE,
			)}
		>
			{options.map((option, at) => (
				<button
					key={option}
					ref={(node) => {
						radios.current[at] = node;
					}}
					type="button"
					// biome-ignore lint/a11y/useSemanticElements: a segmented control is a roving radiogroup of buttons
					role="radio"
					aria-checked={at === checked}
					tabIndex={at === stop ? 0 : -1}
					disabled={disabled}
					{...(at === stop ? bind.focus : {})}
					onClick={() => pick(at)}
					onKeyDown={onKeyDown}
					className={cx(
						"min-w-0 truncate rounded-md border-0 px-1 text-[13px] font-medium focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring",
						at === checked
							? "bg-foreground text-background"
							: "bg-transparent text-ink-2",
					)}
				>
					{option}
				</button>
			))}
		</div>
	);
}

const SELECT_TRIGGER =
	"w-full rounded-lg border-input bg-card shadow-none hover:border-border-strong focus-visible:border-input focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid aria-invalid:border-critical-line aria-invalid:ring-0 dark:bg-card dark:hover:bg-card";

function Menu({
	props,
	options,
}: Readonly<{ props: ControlProps; options: readonly string[] }>) {
	const { t } = useTranslation("interfaces");
	const { field, rail, actions, bind, disabled } = props;
	const value = textOf(valueAt(rail.values, field.key));
	return (
		<Select
			value={value}
			disabled={disabled}
			onValueChange={(next) => actions.setValue(field.key, next)}
		>
			<SelectTrigger
				id={bind.ids.control}
				aria-labelledby={bind.ids.label}
				aria-invalid={bind.invalid || undefined}
				aria-describedby={bind.describedBy()}
				{...bind.focus}
				className={cx(
					SELECT_TRIGGER,
					bind.touch
						? "data-[size=default]:h-11 px-3 text-base"
						: "data-[size=default]:h-9 px-2.75 text-[13.5px]",
					bind.markers.differs && DIFFERS_EDGE,
				)}
			>
				<SelectValue
					placeholder={t("workbench.field.choice.placeholder", "Choose one")}
				/>
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-floating backdrop-blur-none">
				{options.map((option) => (
					<SelectItem
						key={option}
						value={option}
						className="text-[13px]/4.5 focus:bg-row-hover focus:text-foreground"
					>
						{option}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

/** A choice from `valid_values` (the typed value is sent): segmented for 2–4 short options, else a select. */
export function ChoiceField(props: ControlProps) {
	const options = (props.field.options ?? []).filter((option) => option !== "");
	return fitsSegments(options) ? (
		<Segmented props={props} options={options} />
	) : (
		<Menu props={props} options={options} />
	);
}
