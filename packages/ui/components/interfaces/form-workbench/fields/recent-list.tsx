"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactElement, type RefObject, useMemo } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { RecentValue, ViewerHabits } from "../contracts";
import { formatWhen, shortWordsOf } from "../model/date-text";
import { FloatingAnchor, FloatingContent, FloatingRoot } from "./floating";

/*
 * The recent-values list under a text field or a list entry (spec M6): up to six values, newest first, each with the
 * time of the run that had it. Focus never leaves the input: this is the listbox of a combobox, driven by the
 * input's keys (↑/↓ move, ↵ fills, Delete forgets, Esc closes). Mouse presses are kept from taking focus.
 */

export interface RecentListProps {
	readonly open: boolean;
	readonly items: readonly RecentValue[];
	/** The highlighted row; −1 for none. */
	readonly active: number;
	readonly listId: string;
	readonly optionId: (index: number) => string;
	readonly label: string;
	readonly viewer: Pick<ViewerHabits, "locale">;
	readonly anchorRef: RefObject<HTMLElement | null>;
	/** The element the list hangs under: the field's box. */
	readonly anchor: ReactElement;
	readonly onFill: (value: string) => void;
	readonly onDontSave: () => void;
	readonly onClose: () => void;
}

function Row({
	id,
	item,
	selected,
	when,
	onFill,
}: Readonly<{
	id: string;
	item: RecentValue;
	selected: boolean;
	when: string;
	onFill: (value: string) => void;
}>) {
	return (
		// biome-ignore lint/a11y/useKeyWithClickEvents: focus stays in the input, which owns the keys of the list (combobox pattern)
		// biome-ignore lint/a11y/useSemanticElements: the listbox of a combobox popup, not a native select
		<div
			role="option"
			tabIndex={-1}
			id={id}
			aria-selected={selected}
			onClick={() => onFill(item.value)}
			className={cx(
				"flex h-8 cursor-default items-center gap-3 rounded-md px-2 text-[13px]/4.5 text-foreground",
				selected ? "bg-row-selected" : "hover:bg-row-hover",
			)}
		>
			<span className="min-w-0 flex-1 truncate">{item.value}</span>
			<span className="shrink-0 text-xs/4 text-muted-foreground">{when}</span>
		</div>
	);
}

function Rows({
	props,
	whenOf,
}: Readonly<{ props: RecentListProps; whenOf: (at: number) => string }>) {
	const { t } = useTranslation("interfaces");
	return (
		// biome-ignore lint/a11y/useSemanticElements: the listbox of a combobox popup, not a native select
		<div
			role="listbox"
			tabIndex={-1}
			id={props.listId}
			aria-label={t(
				"workbench.field.recent.aria",
				"Recent values for {{label}}",
				{
					label: props.label,
				},
			)}
		>
			{props.items.map((item, index) => (
				<Row
					key={item.value}
					id={props.optionId(index)}
					item={item}
					selected={index === props.active}
					when={whenOf(item.at)}
					onFill={props.onFill}
				/>
			))}
		</div>
	);
}

function Footer({ props }: Readonly<{ props: RecentListProps }>) {
	const { t } = useTranslation("interfaces");
	return (
		<>
			<div className="mt-1 border-t border-hairline pt-1">
				<button
					type="button"
					tabIndex={-1}
					onClick={props.onDontSave}
					className="flex h-8 w-full items-center rounded-md px-2 text-left text-[12.5px]/4.5 text-ink-2 hover:bg-row-hover"
				>
					{t(
						"workbench.field.recent.dontSave",
						"Don't save {{label}} on this device",
						{ label: props.label },
					)}
				</button>
			</div>
			<p className="m-0 h-6 px-2 text-xs/6 text-muted-foreground">
				{t("workbench.field.recent.hint", "↑↓ move · ↵ choose · esc close")}
			</p>
		</>
	);
}

export function RecentList(props: Readonly<RecentListProps>) {
	const { t } = useTranslation("interfaces");
	const words = useMemo(
		() => shortWordsOf(t, props.viewer.locale),
		[t, props.viewer.locale],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: "now" is read each time the list opens
	const now = useMemo(() => Date.now(), [props.open]);
	const whenOf = (at: number) =>
		formatWhen(at, now, props.viewer.locale, words);
	return (
		<FloatingRoot
			open={props.open}
			modal={false}
			onOpenChange={(next) => {
				if (!next) props.onClose();
			}}
		>
			<FloatingAnchor asChild>{props.anchor}</FloatingAnchor>
			<FloatingContent
				className="p-1"
				style={{ width: "max(var(--radix-popover-trigger-width), 240px)" }}
				onOpenAutoFocus={(event) => event.preventDefault()}
				onCloseAutoFocus={(event) => event.preventDefault()}
				onMouseDown={(event) => event.preventDefault()}
				onInteractOutside={(event) => {
					if (props.anchorRef.current?.contains(event.target as Node))
						event.preventDefault();
				}}
			>
				<p
					aria-hidden
					className="m-0 h-6 px-2 text-label/6 font-semibold tracking-[0.06em] text-muted-foreground uppercase"
				>
					{t("workbench.field.recent.title", "Recent")}
				</p>
				<Rows props={props} whenOf={whenOf} />
				<Footer props={props} />
			</FloatingContent>
		</FloatingRoot>
	);
}
