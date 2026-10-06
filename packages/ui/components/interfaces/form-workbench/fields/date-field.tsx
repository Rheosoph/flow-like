"use client";

import { useTranslation } from "@flow-like/locales";
import { Calendar as CalendarIcon, TriangleAlert } from "lucide-react";
import { type KeyboardEvent, useRef, useState } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { DateReading } from "../contracts";
import { formatDate, formatReading } from "../model/date-text";
import { readDate } from "../model/dates";
import { textOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { CalendarPanel } from "./calendar-panel";
import { ghostIcon, lineControl } from "./control-style";
import { handleEnter, useSelectOnTab } from "./enter";
import {
	FloatingAnchor,
	FloatingContent,
	FloatingRoot,
	FloatingTrigger,
} from "./floating";
import { isComposing, resetShortcut } from "./keys";
import { lowerLabel } from "./problem-text";

/** "Reads as Fri 18 Sep 2026", or with the warning "· 29 days before the last date" (spec M4). */
function ReadsAs({
	reading,
	locale,
}: Readonly<{ reading: DateReading; locale: string }>) {
	const { t } = useTranslation("interfaces");
	const date = formatReading(reading.iso, locale);
	const late = reading.past > 0;
	return (
		<output
			className={cx(
				"m-0 flex items-start gap-1 text-xs/4 tabular-nums",
				late ? "text-warning" : "text-muted-foreground",
			)}
		>
			{late ? (
				<TriangleAlert aria-hidden className="mt-px size-3.25 shrink-0" />
			) : null}
			<span>
				{late
					? t(
							"workbench.field.date.readsAsPast",
							"Reads as {{date}} · {{count}} days before the last date",
							{
								date,
								count: reading.past,
								defaultValue_one:
									"Reads as {{date}} · {{count}} day before the last date",
							},
						)
					: t("workbench.field.date.readsAs", "Reads as {{date}}", { date })}
			</span>
		</output>
	);
}

/** The reading of uncommitted text, or null when there is none to show. */
function readingOf(
	props: ControlProps,
	pending: string | undefined,
): DateReading | null {
	if (pending === undefined || pending.trim() === "") return null;
	const { field, viewer, today } = props;
	const reading = readDate(
		pending,
		props.dateAnchorFor(field.key),
		today,
		viewer.dateLocale,
	);
	return reading && reading.iso !== "" ? reading : null;
}

/** A blur that follows a press on the calendar button within this many ms is the person opening the calendar. */
const HOLD_MS = 400;

/**
 * Leaving the input for the calendar is not leaving the field: typed text is committed, validation waits
 * (a required date must not complain while its calendar opens). Any other blur validates (`blurField` also
 * commits typed text in the reducer).
 */
function useDateBlur(
	props: ControlProps,
	pending: string | undefined,
	open: boolean,
) {
	const heldAt = useRef(Number.NEGATIVE_INFINITY);
	const { field, actions } = props;
	return {
		hold: () => {
			heldAt.current = Date.now();
		},
		onBlur: () => {
			const staying = open || Date.now() - heldAt.current < HOLD_MS;
			if (!staying) actions.blurField(field.key);
			else if (pending !== undefined) actions.commitText(field.key);
		},
	};
}

interface CalendarButtonProps {
	readonly props: ControlProps;
	readonly open: boolean;
	readonly onHold: () => void;
}

function CalendarButton({
	props,
	open,
	onHold,
}: Readonly<CalendarButtonProps>) {
	const { t } = useTranslation("interfaces");
	return (
		<FloatingTrigger asChild>
			<button
				type="button"
				tabIndex={-1}
				disabled={props.disabled}
				aria-label={t(
					"workbench.field.date.calendarAria",
					"Choose {{label}} from a calendar",
					{ label: lowerLabel(props.field.label) },
				)}
				aria-expanded={open}
				onPointerDown={onHold}
				className={cx(
					ghostIcon(props.bind.touch, "size-7"),
					"absolute top-1 right-1",
				)}
			>
				<CalendarIcon aria-hidden className="size-3.75" />
			</button>
		</FloatingTrigger>
	);
}

/**
 * A typed date with a calendar (fine pointers): short input in the viewer's order, "Reads as" under it while it is
 * not committed, ↵ / Tab / blur commit, ⌥↓ opens the calendar at the anchor's month with the cursor on a day.
 */
function TypedDate(props: ControlProps) {
	const { t } = useTranslation("interfaces");
	const { field, rail, actions, bind, viewer, today, disabled } = props;
	const iso = textOf(valueAt(rail.values, field.key));
	const pending = rail.texts[field.key];
	const text = pending ?? (iso === "" ? "" : formatDate(iso, viewer.locale));
	const reading = readingOf(props, pending);
	const input = useRef<HTMLInputElement>(null);
	const outside = useRef(false);
	const [open, setOpen] = useState(false);
	const [byKey, setByKey] = useState(false);
	const select = useSelectOnTab<HTMLInputElement>();
	const blur = useDateBlur(props, pending, open);
	const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
		if (isComposing(event) || handleEnter(event, field.key, actions)) return;
		if (event.altKey && event.key === "ArrowDown") {
			event.preventDefault();
			setByKey(true);
			setOpen(true);
		}
	};
	const shortcuts = [
		"Alt+ArrowDown",
		bind.markers.reset ? resetShortcut(viewer.mac) : null,
	]
		.filter(Boolean)
		.join(" ");
	return (
		<>
			<FloatingRoot
				open={open}
				modal={false}
				onOpenChange={(next) => {
					setOpen(next);
					if (!next) setByKey(false);
				}}
			>
				<FloatingAnchor asChild>
					<div className="relative">
						<input
							ref={input}
							id={bind.ids.control}
							type="text"
							autoComplete="off"
							value={text}
							disabled={disabled}
							placeholder={t("workbench.field.date.placeholder", "Pick a date")}
							enterKeyHint={props.enterHint(field.key)}
							aria-invalid={bind.invalid || undefined}
							aria-describedby={bind.describedBy()}
							aria-keyshortcuts={shortcuts}
							{...bind.focus}
							onPointerDown={select.onPointerDown}
							onFocus={select.onFocus}
							onChange={(event) =>
								actions.setText(field.key, event.target.value)
							}
							onKeyDown={onKeyDown}
							onBlur={blur.onBlur}
							className={cx(
								lineControl({
									touch: bind.touch,
									differs: bind.markers.differs,
								}),
								"pr-9",
							)}
						/>
						<CalendarButton props={props} open={open} onHold={blur.hold} />
					</div>
				</FloatingAnchor>
				<FloatingContent
					aria-label={t("workbench.field.date.dialog", "Choose a date")}
					className="w-63"
					onOpenAutoFocus={(event) => event.preventDefault()}
					onInteractOutside={() => {
						outside.current = true;
					}}
					onCloseAutoFocus={(event) => {
						event.preventDefault();
						if (!outside.current) input.current?.focus();
						outside.current = false;
					}}
				>
					<CalendarPanel
						selected={reading ? reading.iso : iso}
						anchor={props.dateAnchorFor(field.key)}
						today={today}
						locale={viewer.locale}
						focusDay={byKey}
						onPick={(picked) => {
							actions.setValue(field.key, picked);
							setOpen(false);
						}}
					/>
				</FloatingContent>
			</FloatingRoot>
			{reading ? <ReadsAs reading={reading} locale={viewer.locale} /> : null}
			<span id={bind.ids.hint} hidden>
				{t(
					"workbench.field.date.shortcutHint",
					"Alt and Down open the calendar.",
				)}
			</span>
		</>
	);
}

/** Touch screens keep the native date picker (spec M4). */
function NativeDate(props: ControlProps) {
	const { field, rail, actions, bind, disabled } = props;
	return (
		<input
			id={bind.ids.control}
			type="date"
			value={textOf(valueAt(rail.values, field.key))}
			disabled={disabled}
			aria-invalid={bind.invalid || undefined}
			aria-describedby={bind.describedBy()}
			{...bind.focus}
			onChange={(event) => actions.setValue(field.key, event.target.value)}
			onBlur={() => actions.blurField(field.key)}
			className={lineControl({
				touch: bind.touch,
				differs: bind.markers.differs,
			})}
		/>
	);
}

export function DateField(props: ControlProps) {
	return props.layout.finePointer ? (
		<TypedDate {...props} />
	) : (
		<NativeDate {...props} />
	);
}
