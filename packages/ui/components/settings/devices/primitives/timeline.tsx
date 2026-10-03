"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleDot, CircleSlash, type LucideIcon } from "lucide-react";
import { type ReactNode, useState } from "react";
import { type AreaTime, useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { Segmented } from "./segmented";
import { cx } from "./tone";

export type TimelineKind =
	| "command"
	| "instance"
	| "update"
	| "device"
	| "access";
export type TimelineFilter =
	| "all"
	| "commands"
	| "instances"
	| "updates"
	| "device"
	| "access";

const FILTER_KIND: Record<Exclude<TimelineFilter, "all">, TimelineKind> = {
	commands: "command",
	instances: "instance",
	updates: "update",
	device: "device",
	access: "access",
};

export type TimelineTone = "good" | "warning" | "critical" | "info";

const ICON_TONE: Record<TimelineTone, string> = {
	good: "text-good",
	warning: "text-warning",
	critical: "text-critical",
	info: "text-info",
};

export interface TimelineEntry {
	id: string;
	/** Unix seconds. */
	at: number;
	kind: TimelineKind;
	icon?: LucideIcon;
	tone?: TimelineTone;
	text: ReactNode;
	/** "process 48211 · command a0e5259e · by you". */
	meta?: ReactNode;
}

/** A hole in the history ("Older entries were deleted to save space …"). */
export interface TimelineGap {
	id: string;
	gap: true;
	text: ReactNode;
}

export type TimelineRow = TimelineEntry | TimelineGap;

const isGap = (row: TimelineRow): row is TimelineGap => "gap" in row;

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

/** The largest time value a Date can hold; beyond it (or NaN) Date and Intl throw. */
const MAX_DATE_MS = 8.64e15;

/** Entry times come from the device: a wrong unit or a broken clock must not throw. */
function isDate(atS: number) {
	return Number.isFinite(atS) && Math.abs(atS * 1000) <= MAX_DATE_MS;
}

function dayKey(atS: number) {
	const date = new Date(atS * 1000);
	return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function useDayLabel(time: AreaTime) {
	const { t } = useTranslation("devices");
	const today = dayKey(time.nowS);
	const yesterday = dayKey(time.nowS - 86_400);
	return (atS: number) => {
		if (!isDate(atS)) return t("view.timeline.unknownDay", "Unknown time");
		const key = dayKey(atS);
		if (key === today) return t("view.timeline.today", "Today");
		if (key === yesterday) return t("view.timeline.yesterday", "Yesterday");
		let formatter = dayFormatters.get(time.locale);
		if (!formatter) {
			formatter = new Intl.DateTimeFormat(time.locale, {
				weekday: "short",
				day: "numeric",
				month: "short",
			});
			dayFormatters.set(time.locale, formatter);
		}
		return formatter.format(atS * 1000);
	};
}

interface DayGroup {
	day: string | null;
	rows: TimelineRow[];
}

function groupByDay(
	rows: readonly TimelineRow[],
	label: (atS: number) => string,
): DayGroup[] {
	const groups: DayGroup[] = [];
	for (const row of rows) {
		const last = groups.at(-1);
		if (isGap(row)) {
			if (last) last.rows.push(row);
			else groups.push({ day: null, rows: [row] });
			continue;
		}
		const day = label(row.at);
		if (last && last.day === day) last.rows.push(row);
		else groups.push({ day, rows: [row] });
	}
	return groups;
}

interface EntryItemProps {
	entry: TimelineEntry;
}

function EntryItem({ entry }: Readonly<EntryItemProps>) {
	const time = useAreaTime();
	const Icon = entry.icon ?? CircleDot;
	return (
		<li
			data-kind={entry.kind}
			data-tone={entry.tone}
			className="grid grid-cols-[66px_18px_minmax(0,1fr)] items-start gap-2.5 border-t border-hairline py-1.75 text-ui first:border-t-0"
		>
			<time
				dateTime={
					isDate(entry.at) ? new Date(entry.at * 1000).toISOString() : undefined
				}
				title={time.abs(entry.at)}
				className="font-mono text-xs leading-4.5 text-muted-foreground tabular-nums"
			>
				{time.clock(entry.at)}
			</time>
			<span
				className={cx(
					"pt-px",
					entry.tone ? ICON_TONE[entry.tone] : "text-muted-foreground",
				)}
			>
				<Icon aria-hidden className="size-3.5" />
			</span>
			<div className="min-w-0">
				<p className="text-ui">{entry.text}</p>
				{entry.meta ? (
					<p className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
						{entry.meta}
					</p>
				) : null}
			</div>
		</li>
	);
}

export interface TimelineGapRowProps {
	children: ReactNode;
}

export function TimelineGapRow({ children }: Readonly<TimelineGapRowProps>) {
	return (
		<li
			data-gap=""
			className="flex items-start gap-1.5 border-t border-dashed border-unknown-line py-2 text-xs text-muted-foreground first:border-t-0"
		>
			<CircleSlash aria-hidden className="mt-px size-3.25 shrink-0" />
			<span>{children}</span>
		</li>
	);
}

interface TimelineFiltersProps {
	value: TimelineFilter;
	onChange: (filter: TimelineFilter) => void;
}

function TimelineFilters({ value, onChange }: Readonly<TimelineFiltersProps>) {
	const { t } = useTranslation("devices");
	return (
		<Segmented
			label={t("view.timeline.show", "Show")}
			size="sm"
			wrap
			value={value}
			onChange={onChange}
			options={[
				{ value: "all", label: t("view.timeline.all", "All") },
				{ value: "commands", label: t("view.timeline.commands", "Commands") },
				{
					value: "instances",
					label: t("view.timeline.instances", "Instances"),
				},
				{ value: "updates", label: t("view.timeline.updates", "Updates") },
				{ value: "device", label: t("view.timeline.device", "Device") },
				{ value: "access", label: t("view.timeline.access", "Access") },
			]}
		/>
	);
}

interface DaySectionProps {
	group: DayGroup;
}

interface DayHeadingProps {
	day: string;
}

function DayHeading({ day }: Readonly<DayHeadingProps>) {
	return (
		<h4 className="pt-1.5 text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
			{day}
		</h4>
	);
}

const rowItem = (row: TimelineRow) =>
	isGap(row) ? (
		<TimelineGapRow key={row.id}>{row.text}</TimelineGapRow>
	) : (
		<EntryItem key={row.id} entry={row} />
	);

function DaySection({ group }: Readonly<DaySectionProps>) {
	const heading = group.day ? <DayHeading day={group.day} /> : null;
	return (
		<section aria-label={group.day ?? undefined}>
			{heading}
			<ol className="flex flex-col">{group.rows.map(rowItem)}</ol>
		</section>
	);
}

interface DayGroupsProps {
	groups: readonly DayGroup[];
	filtered: boolean;
}

function DayGroups({ groups, filtered }: Readonly<DayGroupsProps>) {
	const { t } = useTranslation("devices");
	if (!groups.length)
		return (
			<p className="text-ui text-muted-foreground">
				{filtered
					? t("view.timeline.noMatch", "No entries match this filter.")
					: t("view.timeline.none", "No entries yet.")}
			</p>
		);
	return groups.map((group) => (
		<DaySection key={group.rows[0]?.id ?? group.day} group={group} />
	));
}

export interface TimelineProps {
	rows: readonly TimelineRow[];
	filter?: TimelineFilter;
	onFilterChange?: (filter: TimelineFilter) => void;
	showFilters?: boolean;
	/** Extra controls after the filter (the person select). */
	tools?: ReactNode;
	onLoadOlder?: () => void;
	loadingOlder?: boolean;
	className?: string;
}

/**
 * SPEC §4.29: entries grouped by day, newest first as given, with gap markers
 * that never hide behind a filter. The filter is controlled when
 * `onFilterChange` is passed.
 */
export function Timeline(props: Readonly<TimelineProps>) {
	const { rows, tools, onLoadOlder, className } = props;
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const dayLabel = useDayLabel(time);
	const [own, setOwn] = useState<TimelineFilter>("all");
	const active = props.filter ?? own;
	const kind = active === "all" ? null : FILTER_KIND[active];
	const shown = rows.filter((row) => isGap(row) || !kind || row.kind === kind);
	const filters =
		props.showFilters === false ? null : (
			<TimelineFilters
				value={active}
				onChange={props.onFilterChange ?? setOwn}
			/>
		);
	const older = onLoadOlder ? (
		<DvButton size="sm" busy={props.loadingOlder} onClick={onLoadOlder}>
			{t("view.timeline.older", "Load older")}
		</DvButton>
	) : null;

	return (
		<div
			data-timeline=""
			className={cx("flex min-w-0 flex-col gap-2", className)}
		>
			<div className="flex flex-wrap items-center gap-2 empty:hidden">
				{filters}
				{tools}
			</div>
			<DayGroups
				groups={groupByDay(shown, dayLabel)}
				filtered={kind !== null}
			/>
			<div className="empty:hidden">{older}</div>
		</div>
	);
}
