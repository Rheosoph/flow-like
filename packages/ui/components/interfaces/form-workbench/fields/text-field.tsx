"use client";

import { useTranslation } from "@flow-like/locales";
import { useRef } from "react";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { textBox } from "./control-style";
import { handleEnter, useSelectOnTab } from "./enter";
import { resetShortcut } from "./keys";
import { RecentList } from "./recent-list";
import { useAutoGrow } from "./use-auto-grow";
import { useEntry } from "./use-entry";

/** The placeholder of an empty text control: a withheld default, or the pointer to recent values. */
export function usePlaceholder(
	props: ControlProps,
	empty: boolean,
	hasRecent: boolean,
) {
	const { t } = useTranslation("interfaces");
	const { field } = props;
	if (!empty) return undefined;
	if (field.defaultOmitted && !field.required)
		return t(
			"workbench.field.withheldPlaceholder",
			"The app's own value is used when this is empty.",
		);
	return hasRecent
		? t(
				"workbench.field.recentPlaceholder",
				"Type, or press ↓ for recent values",
			)
		: undefined;
}

/**
 * A text field (spec M2, M6): a textarea that is one line until its text holds a line break (⇧↵ adds one),
 * with the inline suggestion and the recent-values list for fine pointers. ↵ moves on, ↵ with a suggestion
 * showing keeps exactly what was typed.
 */
export function TextField(props: ControlProps) {
	const { field, rail, actions, bind, disabled } = props;
	const typed = textOf(valueAt(rail.values, field.key));
	const multiline = typed.includes("\n");
	const entry = useEntry<HTMLTextAreaElement>({
		props,
		typed,
		multiline,
		commit: (next, how) => actions.setValue(field.key, next, how),
		onEnter: (event) => handleEnter(event, field.key, actions),
	});
	useAutoGrow(entry.element, entry.display, bind.touch);
	const select = useSelectOnTab<HTMLTextAreaElement>();
	const placeholder = usePlaceholder(props, typed === "", entry.hasRecent);
	const anchor = useRef<HTMLDivElement>(null);
	const listId = `${bind.ids.control}-list`;
	const optionId = (index: number) => `${bind.ids.control}-option-${index}`;
	const open = entry.listOpen;
	const box = (
		<div ref={anchor} className="relative">
			<textarea
				ref={entry.element}
				id={bind.ids.control}
				rows={1}
				value={entry.display}
				disabled={disabled}
				placeholder={placeholder}
				autoComplete="off"
				enterKeyHint={props.enterHint(field.key)}
				aria-invalid={bind.invalid || undefined}
				aria-describedby={bind.describedBy()}
				aria-keyshortcuts={
					bind.markers.reset ? resetShortcut(props.viewer.mac) : undefined
				}
				{...(entry.combobox
					? {
							role: "combobox",
							"aria-autocomplete": "both" as const,
							"aria-expanded": open,
							"aria-controls": open ? listId : undefined,
							"aria-activedescendant":
								open && entry.active >= 0 ? optionId(entry.active) : undefined,
						}
					: {})}
				{...bind.focus}
				onPointerDown={select.onPointerDown}
				onFocus={select.onFocus}
				onChange={entry.onChange}
				onKeyDown={entry.onKeyDown}
				onPaste={entry.remember}
				onCut={entry.remember}
				onMouseUp={entry.onMouseUp}
				onBlur={() => {
					entry.onBlur();
					actions.blurField(field.key);
				}}
				className={textBox({
					touch: bind.touch,
					differs: bind.markers.differs,
				})}
			/>
		</div>
	);
	return (
		<RecentList
			open={open}
			items={entry.items}
			active={entry.active}
			listId={listId}
			optionId={optionId}
			label={field.label}
			viewer={props.viewer}
			anchorRef={anchor}
			anchor={box}
			onFill={entry.fill}
			onDontSave={() => actions.dontSave(field.key)}
			onClose={() => actions.closeList()}
		/>
	);
}
