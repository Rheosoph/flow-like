"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlarmClockIcon,
	AlignLeftIcon,
	ArrowUpRightIcon,
	BellRingIcon,
	CalendarDaysIcon,
	Loader2Icon,
	MapPinIcon,
	PencilIcon,
	RepeatIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { cn } from "../../lib/utils";
import type {
	ICalendarEntryWake,
	ICalendarWake,
	IUpdateCalendarWake,
} from "../../state/backend-state/bot-state";
import type { CalendarEvent } from "../a2ui/types";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";
import {
	CALENDAR_LAYER_COLORS,
	type CalendarLayer,
	type EventDraft,
	MAX_WAKE_OFFSETS,
	MAX_WAKE_PATTERNS,
	MAX_WAKE_PATTERN_CHARS,
	REPEAT_CHOICES,
	type RepeatChoice,
	WAKE_OFFSET_CHOICES,
	type WakeChoice,
	addWakePattern,
	formatWakeOffset,
	formatWakeOffsets,
	formatWhen,
	toggleWakeOffset,
	withAllDay,
} from "./bot-calendar-model";

/** The colored dot every layer shows in the toolbar and the details. */
export function LayerDot({ layer }: Readonly<{ layer: CalendarLayer }>) {
	return (
		<span
			aria-hidden
			className="inline-block size-2.5 shrink-0 rounded-full"
			style={{ backgroundColor: CALENDAR_LAYER_COLORS[layer] }}
		/>
	);
}

/** Toggle buttons in a row; the pressed one is the value. */
function Choices<T extends string>({
	value,
	options,
	label,
	disabled,
	onChange,
}: Readonly<{
	value: T;
	options: readonly { value: T; label: string }[];
	label: string;
	disabled?: boolean;
	onChange: (value: T) => void;
}>) {
	return (
		<fieldset
			aria-label={label}
			className="flex flex-wrap gap-1 border-0 p-0"
			disabled={disabled}
		>
			{options.map((option) => (
				<Button
					key={option.value}
					type="button"
					size="sm"
					variant={value === option.value ? "secondary" : "ghost"}
					aria-pressed={value === option.value}
					className="h-7 px-2.5 text-xs"
					onClick={() => onChange(option.value)}
				>
					{option.label}
				</Button>
			))}
		</fieldset>
	);
}

/** 5 min … 1 day; at most four picked. */
function OffsetChips({
	offsets,
	disabled,
	keepOne = false,
	onChange,
}: Readonly<{
	offsets: readonly number[];
	disabled?: boolean;
	/** The bot's settings need at least one time. */
	keepOne?: boolean;
	onChange: (offsets: number[]) => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	return (
		<fieldset
			aria-label={t("calendar.wake.offsetsLabel", "Wake up before the start")}
			className="flex flex-wrap gap-1 border-0 p-0"
			disabled={disabled}
		>
			{WAKE_OFFSET_CHOICES.map((minutes) => {
				const picked = offsets.includes(minutes);
				const locked =
					(!picked && offsets.length >= MAX_WAKE_OFFSETS) ||
					(picked && keepOne && offsets.length === 1);
				return (
					<Button
						key={minutes}
						type="button"
						size="sm"
						variant="outline"
						aria-pressed={picked}
						disabled={locked}
						className={cn(
							"h-7 rounded-full px-2.5 text-xs tabular-nums",
							picked &&
								"border-primary/60 bg-primary/15 text-foreground hover:bg-primary/25 dark:border-primary/60 dark:bg-primary/15 dark:hover:bg-primary/25",
						)}
						onClick={() => onChange(toggleWakeOffset(offsets, minutes))}
					>
						{formatWakeOffset(minutes, i18n?.language)}
					</Button>
				);
			})}
		</fieldset>
	);
}

/** An event's own wake-up: like the bot's settings, at chosen times, or never. */
export function EventWakeField({
	choice,
	offsets,
	disabled,
	onChange,
}: Readonly<{
	choice: WakeChoice;
	offsets: readonly number[];
	disabled?: boolean;
	onChange: (choice: WakeChoice, offsets: number[]) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-col gap-2" data-slot="event-wake">
			<Choices
				value={choice}
				label={t("calendar.wake.title", "Wake-up")}
				disabled={disabled}
				options={[
					{
						value: "default",
						label: t("calendar.wake.default", "Like the settings"),
					},
					{ value: "on", label: t("calendar.wake.on", "Wake") },
					{ value: "off", label: t("calendar.wake.off", "Never") },
				]}
				onChange={(next) => onChange(next, [...offsets])}
			/>
			{choice === "on" && (
				<>
					<OffsetChips
						offsets={offsets}
						disabled={disabled}
						onChange={(next) => onChange("on", next)}
					/>
					<p className="text-xs text-muted-foreground">
						{t(
							"calendar.wake.offsetsHint",
							"Pick up to four times before the start. None picked uses the times of the settings.",
						)}
					</p>
				</>
			)}
		</div>
	);
}

/** "Wakes 15 min, 1 day before · next Nov 4, 8:45", or that it doesn't wake the bot. */
export function EntryWakeLine({
	wake,
	name,
}: Readonly<{ wake?: ICalendarEntryWake | null; name: string }>) {
	const { t, i18n } = useTranslation("bots");
	const locale = i18n?.language;
	const next = wake?.next_at ? new Date(wake.next_at) : null;
	return (
		<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
			<BellRingIcon className="mt-px size-3.5 shrink-0" aria-hidden />
			<span className="min-w-0">
				{wake && wake.offsets_minutes.length > 0
					? t("calendar.wake.state", "Wakes {{name}} {{offsets}} before", {
							name,
							offsets: formatWakeOffsets(wake, locale),
						})
					: t("calendar.wake.stateNone", "Doesn't wake {{name}}", { name })}
				{next &&
					!Number.isNaN(next.getTime()) &&
					` · ${t("calendar.wake.next", "next {{when}}", {
						when: new Intl.DateTimeFormat(locale, {
							dateStyle: "medium",
							timeStyle: "short",
						}).format(next),
					})}`}
			</span>
		</p>
	);
}

function DetailRow({
	icon,
	children,
}: Readonly<{ icon: ReactNode; children: ReactNode }>) {
	return (
		<div className="flex items-start gap-2.5 text-sm">
			<span className="mt-0.5 shrink-0 text-muted-foreground">{icon}</span>
			<div className="min-w-0 flex-1 wrap-anywhere">{children}</div>
		</div>
	);
}

/** The details of one calendar chip; `children` adds what its layer can do. */
export function CalendarItemDialog({
	event,
	layer,
	source,
	recurring,
	link,
	open,
	actions,
	children,
	onOpenChange,
}: Readonly<{
	event: CalendarEvent | null;
	layer: CalendarLayer;
	/** Where the item comes from, such as a calendar's name or a task's schedule. */
	source?: string;
	recurring?: boolean;
	link?: string | null;
	open: boolean;
	actions?: ReactNode;
	children?: ReactNode;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="max-h-[85vh] overflow-y-auto outline-none sm:max-w-md"
				// The first control can be Forget or a wake-up choice; Enter must not hit it by accident.
				onOpenAutoFocus={(focus) => {
					focus.preventDefault();
					(focus.currentTarget as HTMLElement | null)?.focus();
				}}
			>
				{event && (
					<>
						<DialogHeader className="gap-1.5">
							<DialogTitle className="flex items-center gap-2 pr-6 text-base leading-snug">
								<LayerDot layer={layer} />
								<span className="min-w-0 wrap-anywhere">{event.title}</span>
							</DialogTitle>
							<DialogDescription>
								{formatWhen(
									event,
									i18n?.language,
									t("calendar.details.allDay", "All day"),
								)}
							</DialogDescription>
						</DialogHeader>
						<div className="flex flex-col gap-2.5">
							{source && (
								<DetailRow icon={<CalendarDaysIcon className="size-4" />}>
									<span className="text-muted-foreground">{source}</span>
								</DetailRow>
							)}
							{recurring && (
								<DetailRow icon={<RepeatIcon className="size-4" />}>
									<span className="text-muted-foreground">
										{t("calendar.details.repeats", "Repeats")}
									</span>
								</DetailRow>
							)}
							{event.location && (
								<DetailRow icon={<MapPinIcon className="size-4" />}>
									{event.location}
								</DetailRow>
							)}
							{event.description && (
								<DetailRow icon={<AlignLeftIcon className="size-4" />}>
									<p className="whitespace-pre-wrap text-muted-foreground">
										{event.description}
									</p>
								</DetailRow>
							)}
							{link && (
								<DetailRow icon={<ArrowUpRightIcon className="size-4" />}>
									<a
										href={link}
										target="_blank"
										rel="noopener noreferrer"
										className="font-medium text-primary underline-offset-4 hover:underline"
									>
										{t("calendar.details.openLink", "Open in its calendar")}
									</a>
								</DetailRow>
							)}
							{children}
						</div>
						{actions && (
							<DialogFooter className="gap-2 sm:justify-between">
								{actions}
							</DialogFooter>
						)}
					</>
				)}
			</DialogContent>
		</Dialog>
	);
}

/** Edit and Delete for an event of the bot's own calendar. */
export function OwnEventActions({
	busy,
	onEdit,
	onDelete,
}: Readonly<{ busy: boolean; onEdit: () => void; onDelete: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<Button
				type="button"
				variant="ghost"
				size="sm"
				disabled={busy}
				className="text-destructive hover:text-destructive"
				onClick={onDelete}
			>
				<Trash2Icon aria-hidden />
				{t("calendar.details.delete", "Delete")}
			</Button>
			<Button type="button" size="sm" disabled={busy} onClick={onEdit}>
				<PencilIcon aria-hidden />
				{t("calendar.details.edit", "Edit")}
			</Button>
		</>
	);
}

/** A wake-up block inside the details, saved as soon as it changes. */
export function WakeSection({
	choice,
	offsets,
	wake,
	name,
	busy,
	onChange,
}: Readonly<{
	choice: WakeChoice;
	offsets: readonly number[];
	wake?: ICalendarEntryWake | null;
	name: string;
	busy: boolean;
	onChange: (choice: WakeChoice, offsets: number[]) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<section className="mt-1 flex flex-col gap-2 rounded-xl border border-border/70 bg-muted/30 p-3">
			<h3 className="flex items-center gap-1.5 text-xs font-medium">
				<AlarmClockIcon className="size-3.5" aria-hidden />
				{t("calendar.wake.title", "Wake-up")}
				{busy && (
					<Loader2Icon
						className="size-3.5 animate-spin text-muted-foreground"
						aria-hidden
					/>
				)}
			</h3>
			<EntryWakeLine wake={wake} name={name} />
			<EventWakeField
				choice={choice}
				offsets={offsets}
				disabled={busy}
				onChange={onChange}
			/>
		</section>
	);
}

function useRepeatLabel() {
	const { t } = useTranslation("bots");
	return (choice: RepeatChoice, cron: string) =>
		({
			none: t("calendar.repeats.none", "Never"),
			daily: t("calendar.repeats.daily", "Every day"),
			weekdays: t("calendar.repeats.weekdays", "Every weekday"),
			weekly: t("calendar.repeats.weekly", "Every week"),
			monthly: t("calendar.repeats.monthly", "Every month"),
			custom: t("calendar.repeats.custom", "Custom ({{cron}})", { cron }),
		})[choice];
}

function Field({
	label,
	htmlFor,
	children,
}: Readonly<{ label: string; htmlFor?: string; children: ReactNode }>) {
	return (
		<div className="flex flex-col gap-1.5">
			<Label htmlFor={htmlFor} className="text-xs text-muted-foreground">
				{label}
			</Label>
			{children}
		</div>
	);
}

/** New and edited events of the bot's own calendar. `ready` is false while the form can't be saved. */
export function EventEditorDialog({
	open,
	mode,
	draft,
	ready,
	busy,
	name,
	onDraftChange,
	onSubmit,
	onOpenChange,
}: Readonly<{
	open: boolean;
	mode: "create" | "edit";
	draft: EventDraft;
	ready: boolean;
	busy: boolean;
	name: string;
	onDraftChange: (draft: EventDraft) => void;
	onSubmit: () => void;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const id = useId();
	const repeatLabel = useRepeatLabel();
	const set = (patch: Partial<EventDraft>) =>
		onDraftChange({ ...draft, ...patch });
	const repeatChoices: readonly RepeatChoice[] =
		draft.repeat === "custom" ? [...REPEAT_CHOICES, "custom"] : REPEAT_CHOICES;
	const inputType = draft.allDay ? "date" : "datetime-local";
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-md">
				<form
					className="flex flex-col gap-4"
					onSubmit={(event) => {
						event.preventDefault();
						if (ready && !busy) onSubmit();
					}}
				>
					<DialogHeader>
						<DialogTitle>
							{mode === "create"
								? t("calendar.editor.newTitle", "New event")
								: t("calendar.editor.editTitle", "Edit event")}
						</DialogTitle>
						<DialogDescription>
							{t(
								"calendar.editor.description",
								"An event in {{name}}'s own calendar.",
								{ name },
							)}
						</DialogDescription>
					</DialogHeader>
					<Field
						label={t("calendar.editor.title", "Title")}
						htmlFor={`${id}-title`}
					>
						<Input
							id={`${id}-title`}
							value={draft.title}
							maxLength={200}
							autoFocus
							onChange={(event) => set({ title: event.target.value })}
						/>
					</Field>
					<label
						htmlFor={`${id}-all-day`}
						className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-sm"
					>
						{t("calendar.editor.allDay", "All day")}
						<Switch
							id={`${id}-all-day`}
							checked={draft.allDay}
							onCheckedChange={(allDay) =>
								onDraftChange(withAllDay(draft, allDay))
							}
						/>
					</label>
					<div className="grid grid-cols-2 gap-2">
						<Field
							label={t("calendar.editor.start", "Start")}
							htmlFor={`${id}-start`}
						>
							<Input
								id={`${id}-start`}
								type={inputType}
								value={draft.start}
								className="text-xs"
								onChange={(event) => set({ start: event.target.value })}
							/>
						</Field>
						<Field
							label={t("calendar.editor.end", "End")}
							htmlFor={`${id}-end`}
						>
							<Input
								id={`${id}-end`}
								type={inputType}
								value={draft.end}
								className="text-xs"
								onChange={(event) => set({ end: event.target.value })}
							/>
						</Field>
					</div>
					<Field label={t("calendar.editor.repeatLabel", "Repeat")}>
						<Select
							value={draft.repeat}
							onValueChange={(repeat) =>
								set({ repeat: repeat as RepeatChoice })
							}
						>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{repeatChoices.map((choice) => (
									<SelectItem key={choice} value={choice}>
										{repeatLabel(choice, draft.cron)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</Field>
					<Field
						label={t("calendar.editor.location", "Location")}
						htmlFor={`${id}-location`}
					>
						<Input
							id={`${id}-location`}
							value={draft.location}
							maxLength={300}
							onChange={(event) => set({ location: event.target.value })}
						/>
					</Field>
					<Field
						label={t("calendar.editor.notes", "Notes for {{name}}", { name })}
						htmlFor={`${id}-notes`}
					>
						<Textarea
							id={`${id}-notes`}
							value={draft.notes}
							rows={3}
							maxLength={4000}
							onChange={(event) => set({ notes: event.target.value })}
						/>
					</Field>
					<Field label={t("calendar.wake.title", "Wake-up")}>
						<EventWakeField
							choice={draft.wake}
							offsets={draft.offsets}
							onChange={(wake, offsets) => set({ wake, offsets })}
						/>
					</Field>
					<DialogFooter className="gap-2">
						<Button
							type="button"
							variant="outline"
							onClick={() => onOpenChange(false)}
						>
							{t("calendar.editor.cancel", "Cancel")}
						</Button>
						<Button type="submit" disabled={!ready || busy}>
							{busy && <Loader2Icon className="animate-spin" aria-hidden />}
							{mode === "create"
								? t("calendar.editor.create", "Create")
								: t("calendar.editor.save", "Save")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

/** Words that pick events by title or location; case doesn't matter. */
function PatternField({
	label,
	patterns,
	disabled,
	onChange,
}: Readonly<{
	label: string;
	patterns: readonly string[];
	disabled?: boolean;
	onChange: (patterns: string[]) => void;
}>) {
	const { t } = useTranslation("bots");
	const id = useId();
	const [text, setText] = useState("");
	const add = () => {
		onChange(addWakePattern(patterns, text));
		setText("");
	};
	return (
		<div className="flex flex-col gap-1.5">
			<Label htmlFor={id} className="text-xs text-muted-foreground">
				{label}
			</Label>
			{patterns.length > 0 && (
				<ul className="flex flex-wrap gap-1">
					{patterns.map((pattern) => (
						<li
							key={pattern}
							className="inline-flex h-6 items-center gap-0.5 rounded-full bg-muted pr-0.5 pl-2.5 text-xs"
						>
							<span className="max-w-40 truncate">{pattern}</span>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								disabled={disabled}
								className="size-5 rounded-full text-muted-foreground"
								aria-label={t(
									"calendar.settings.removePattern",
									"Remove {{pattern}}",
									{
										pattern,
									},
								)}
								onClick={() =>
									onChange(patterns.filter((item) => item !== pattern))
								}
							>
								<XIcon className="size-3" aria-hidden />
							</Button>
						</li>
					))}
				</ul>
			)}
			<Input
				id={id}
				value={text}
				disabled={disabled || patterns.length >= MAX_WAKE_PATTERNS}
				maxLength={MAX_WAKE_PATTERN_CHARS}
				placeholder={t(
					"calendar.settings.patternPlaceholder",
					"A word in the title or location, then Enter",
				)}
				className="h-8 text-xs"
				onChange={(event) => setText(event.target.value)}
				onKeyDown={(event) => {
					if (event.key !== "Enter") return;
					event.preventDefault();
					add();
				}}
				onBlur={() => text.trim() && add()}
			/>
		</div>
	);
}

export function SettingSwitch({
	label,
	checked,
	disabled,
	onChange,
}: Readonly<{
	label: string;
	checked: boolean;
	disabled?: boolean;
	onChange: (checked: boolean) => void;
}>) {
	const id = useId();
	return (
		<label
			htmlFor={id}
			className={cn(
				"flex items-center justify-between gap-3 text-sm",
				disabled && "opacity-60",
			)}
		>
			{label}
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onChange}
			/>
		</label>
	);
}

/** The bot's wake-up settings, saved together. */
export function WakeSettingsDialog({
	open,
	wake,
	busy,
	name,
	onSave,
	onOpenChange,
}: Readonly<{
	open: boolean;
	/** Null while the settings load. */
	wake: ICalendarWake | null;
	busy: boolean;
	name: string;
	onSave: (update: IUpdateCalendarWake) => void;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("calendar.settings.title", "Wake-ups")}</DialogTitle>
					<DialogDescription>
						{t(
							"calendar.settings.description",
							"{{name}} can wake up a set time before an event, decide whether it needs it, and remind you or prepare what you need.",
							{ name },
						)}
					</DialogDescription>
				</DialogHeader>
				{wake ? (
					<WakeSettingsForm
						key={JSON.stringify(wake)}
						wake={wake}
						busy={busy}
						onSave={onSave}
						onCancel={() => onOpenChange(false)}
					/>
				) : (
					<Loader2Icon
						className="mx-auto my-6 size-5 animate-spin text-muted-foreground"
						aria-label={t("crew.dock.loading", "Loading…")}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function WakeSettingsForm({
	wake,
	busy,
	onSave,
	onCancel,
}: Readonly<{
	wake: ICalendarWake;
	busy: boolean;
	onSave: (update: IUpdateCalendarWake) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState<ICalendarWake>(wake);
	const set = (patch: Partial<ICalendarWake>) =>
		setDraft((current) => ({ ...current, ...patch }));
	const off = !draft.enabled;
	return (
		<form
			className="flex flex-col gap-4"
			data-slot="calendar-wake-settings"
			onSubmit={(event) => {
				event.preventDefault();
				onSave(draft);
			}}
		>
			<SettingSwitch
				label={t("calendar.settings.enabled", "Wake up before events")}
				checked={draft.enabled}
				onChange={(enabled) => set({ enabled })}
			/>
			<div className="flex flex-col gap-1.5">
				<span className="text-xs text-muted-foreground">
					{t("calendar.settings.offsets", "When, up to four times")}
				</span>
				<OffsetChips
					offsets={draft.offsets_minutes}
					keepOne
					disabled={off}
					onChange={(offsets_minutes) => set({ offsets_minutes })}
				/>
			</div>
			<div className="flex flex-col gap-2.5">
				<SettingSwitch
					label={t("calendar.settings.internal", "Own calendar")}
					checked={draft.internal}
					disabled={off}
					onChange={(internal) => set({ internal })}
				/>
				<SettingSwitch
					label={t("calendar.settings.external", "Connected calendars")}
					checked={draft.external}
					disabled={off}
					onChange={(external) => set({ external })}
				/>
				<SettingSwitch
					label={t("calendar.settings.skipAllDay", "Skip whole-day events")}
					checked={draft.skip_all_day}
					disabled={off}
					onChange={(skip_all_day) => set({ skip_all_day })}
				/>
				<SettingSwitch
					label={t(
						"calendar.settings.quiet",
						"Stay quiet when there's nothing to do",
					)}
					checked={draft.quiet}
					onChange={(quiet) => set({ quiet })}
				/>
			</div>
			<PatternField
				label={t("calendar.settings.onlyMatching", "Only events containing")}
				patterns={draft.only_matching}
				disabled={off}
				onChange={(only_matching) => set({ only_matching })}
			/>
			<PatternField
				label={t(
					"calendar.settings.skipMatching",
					"Never for events containing",
				)}
				patterns={draft.skip_matching}
				disabled={off}
				onChange={(skip_matching) => set({ skip_matching })}
			/>
			<p className="text-xs text-muted-foreground">
				{t(
					"calendar.settings.eventHint",
					"An event's own wake-up, set in its details, overrides these settings.",
				)}
			</p>
			<DialogFooter className="gap-2">
				<Button type="button" variant="outline" onClick={onCancel}>
					{t("calendar.editor.cancel", "Cancel")}
				</Button>
				<Button type="submit" disabled={busy}>
					{busy && <Loader2Icon className="animate-spin" aria-hidden />}
					{t("calendar.editor.save", "Save")}
				</Button>
			</DialogFooter>
		</form>
	);
}
