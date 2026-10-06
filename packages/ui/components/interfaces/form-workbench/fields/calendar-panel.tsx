"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useMemo, useRef } from "react";
import { Calendar } from "../../../ui/calendar";
import { dateToIso, isoToDate, startOf, weekStartOf } from "./date-iso";

/*
 * The calendar of a typed date (spec M4): opens at the anchor's month, arrows move by day and week, PgUp/PgDn by
 * month, ↵ picks, Esc closes (the popover's). DayPicker's own keys do all of that; this panel dresses it in the
 * house style (neutral selection, 30 px days) and reads names and weeks in the viewer's language through Intl.
 */

const CLASSES = {
	months: "flex flex-col",
	month: "space-y-1.5",
	caption: "relative flex items-center justify-center",
	caption_label: "text-[13px]/7 font-semibold",
	nav: "flex items-center",
	nav_button:
		"inline-flex size-7 items-center justify-center rounded-md bg-transparent p-0 text-foreground opacity-100 hover:bg-row-hover hover:opacity-100",
	nav_button_previous: "absolute left-0",
	nav_button_next: "absolute right-0",
	table: "m-0 w-full border-collapse",
	head_row: "grid grid-cols-7 gap-0.5",
	head_cell:
		"flex h-5.5 items-center justify-center border-0 bg-transparent p-0 text-xs font-medium text-muted-foreground",
	row: "mt-0.5 grid grid-cols-7 gap-0.5",
	cell: "relative border-0 bg-transparent p-0 text-center",
	day: "h-7.5 w-full rounded-md border border-transparent bg-transparent p-0 text-[12.5px] font-normal tabular-nums text-foreground hover:border-border-strong hover:bg-transparent focus-visible:outline-2 focus-visible:outline-offset-0 focus-visible:outline-solid focus-visible:outline-ring",
	day_selected:
		"border-foreground bg-foreground text-background hover:border-foreground hover:bg-foreground hover:text-background focus:bg-foreground focus:text-background",
	day_today: "border-border-strong bg-transparent text-foreground",
	day_outside: "text-muted-foreground",
	day_disabled: "text-muted-foreground opacity-50",
	day_hidden: "invisible",
} as const;

export interface CalendarPanelProps {
	/** The chosen day (`YYYY-MM-DD`) or "". */
	readonly selected: string;
	/** Where an empty field opens and where the cursor starts: the anchor's day. */
	readonly anchor: string;
	readonly today: string;
	readonly locale: string;
	/** Move the cursor onto the day at once (opened with ⌥↓). */
	readonly focusDay: boolean;
	readonly onPick: (iso: string) => void;
}

function focusDayButton(root: HTMLElement | null, day: number) {
	const buttons = root?.querySelectorAll<HTMLButtonElement>("button[name=day]");
	for (const button of buttons ?? []) {
		if (
			button.textContent === String(day) &&
			!button.className.includes("outside")
		) {
			button.focus();
			return;
		}
	}
}

export function CalendarPanel(props: Readonly<CalendarPanelProps>) {
	const { t } = useTranslation("interfaces");
	const root = useRef<HTMLDivElement>(null);
	const start = useMemo(
		() => startOf(props.selected, props.anchor, props.today),
		[props.selected, props.anchor, props.today],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: the cursor moves once, when the panel opens
	useEffect(() => {
		if (props.focusDay) focusDayButton(root.current, start.getDate());
	}, []);
	const caption = useMemo(
		() =>
			new Intl.DateTimeFormat(props.locale, { month: "long", year: "numeric" }),
		[props.locale],
	);
	const weekday = useMemo(
		() => new Intl.DateTimeFormat(props.locale, { weekday: "short" }),
		[props.locale],
	);
	const full = useMemo(
		() => new Intl.DateTimeFormat(props.locale, { dateStyle: "full" }),
		[props.locale],
	);
	return (
		<div ref={root} className="p-2.5">
			<Calendar
				mode="single"
				required
				className="p-0"
				classNames={CLASSES}
				selected={isoToDate(props.selected)}
				defaultMonth={start}
				today={isoToDate(props.today)}
				weekStartsOn={weekStartOf(props.locale)}
				showOutsideDays={false}
				onSelect={(day) => {
					if (day) props.onPick(dateToIso(day));
				}}
				formatters={{
					formatCaption: (month) => caption.format(month),
					formatWeekdayName: (day) => weekday.format(day),
				}}
				labels={{
					labelPrevious: () =>
						t("workbench.field.date.previousMonth", "Previous month"),
					labelNext: () => t("workbench.field.date.nextMonth", "Next month"),
					labelDay: (day) => full.format(day),
				}}
			/>
		</div>
	);
}
