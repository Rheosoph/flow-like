"use client";

import {
	addDays,
	addMonths,
	addWeeks,
	isSameDay,
	isSameMonth,
	isToday,
	set,
	startOfDay,
} from "date-fns";
import {
	CalendarDaysIcon,
	ChevronLeftIcon,
	ChevronRightIcon,
	Loader2Icon,
	PlusIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/index";
import {
	useComponentActionTrigger,
	useIsComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	type WeekStartsOn,
	eventDurationMinutes,
	eventEnd,
	getMonthWeeks,
	getWeekDays,
	normalizeCalendarEvents,
	parseTimeToMinutes,
	toDate,
} from "../planning-utils";
import type {
	BoundValue,
	CalendarComponent,
	CalendarEvent,
	CalendarView,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const VIEWS: CalendarView[] = ["month", "week", "day", "agenda"];
const DEFAULT_COMPACT_BREAKPOINT = 640;

function iso(date: Date): string {
	return date.toISOString();
}

function minutesOf(date: Date): number {
	return date.getHours() * 60 + date.getMinutes();
}

/** Move an event's start (and its end by the same delta) to a new instant. */
function shiftEvent(ev: CalendarEvent, newStart: Date): CalendarEvent {
	const durationMs = eventEnd(ev).getTime() - toDate(ev.start).getTime();
	return {
		...ev,
		start: iso(newStart),
		end: iso(new Date(newStart.getTime() + durationMs)),
	};
}

export function A2UICalendar({
	component,
	componentId,
	style,
}: ComponentProps<CalendarComponent>) {
	const containerRef = useRef<HTMLDivElement>(null);
	const trigger = useComponentActionTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);

	const rawEvents = useResolved<unknown>(component.events);
	const viewProp =
		(useResolved<string>(component.view) as CalendarView) ?? "month";
	const dateProp = useResolved<string>(component.date);
	const editable = useResolved<boolean>(component.editable) ?? true;
	const selectable = useResolved<boolean>(component.selectable) ?? true;
	const firstDayOfWeek = (useResolved<number>(component.firstDayOfWeek) ??
		1) as WeekStartsOn;
	const minTime = parseTimeToMinutes(useResolved<string>(component.minTime), 0);
	const maxTime = parseTimeToMinutes(
		useResolved<string>(component.maxTime),
		24 * 60,
	);
	const slotDuration = useResolved<number>(component.slotDuration) ?? 30;
	const showWeekends = useResolved<boolean>(component.showWeekends) ?? true;
	const showNowIndicator =
		useResolved<boolean>(component.showNowIndicator) ?? true;
	const locale = useResolved<string>(component.locale) || undefined;
	const height = useResolved<string>(component.height);
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const compactBreakpoint =
		useResolved<number>(component.compactBreakpoint) ??
		DEFAULT_COMPACT_BREAKPOINT;

	const resolvedEvents = useMemo(
		() => normalizeCalendarEvents(rawEvents),
		[rawEvents],
	);
	// Local overlay lets drag/resize feel instant before the workflow round-trips.
	const [events, setEvents] = useState<CalendarEvent[]>(resolvedEvents);
	useEffect(() => setEvents(resolvedEvents), [resolvedEvents]);

	const [view, setView] = useState<CalendarView>(viewProp);
	useEffect(() => setView(viewProp), [viewProp]);

	const [focusDate, setFocusDate] = useState<Date>(() =>
		dateProp ? toDate(dateProp) : new Date(),
	);
	useEffect(() => {
		if (dateProp) setFocusDate(toDate(dateProp));
	}, [dateProp]);

	// Auto-collapse to the agenda view on narrow containers.
	const [isNarrow, setIsNarrow] = useState(false);
	useEffect(() => {
		if (!responsive || typeof ResizeObserver === "undefined") {
			setIsNarrow(false);
			return;
		}
		const el = containerRef.current;
		if (!el) return;
		const obs = new ResizeObserver((entries) => {
			for (const entry of entries) {
				setIsNarrow(entry.contentRect.width < compactBreakpoint);
			}
		});
		obs.observe(el);
		return () => {
			obs.disconnect();
			setIsNarrow(false);
		};
	}, [responsive, compactBreakpoint]);
	const effectiveView: CalendarView = isNarrow ? "agenda" : view;

	const fire = useCallback(
		(interaction: string, extra: Record<string, unknown>) => {
			void trigger(component.actions, { interaction, ...extra });
		},
		[trigger, component.actions],
	);

	const onCreate = useCallback(
		(start: Date, end: Date, allDay: boolean) =>
			fire("create", { start: iso(start), end: iso(end), allDay }),
		[fire],
	);
	const onOpen = useCallback(
		(ev: CalendarEvent) =>
			fire("open", {
				id: ev.id,
				start: ev.start,
				end: ev.end,
				metadata: ev.metadata,
			}),
		[fire],
	);
	const onDelete = useCallback(
		(ev: CalendarEvent) => {
			setEvents((prev) => prev.filter((e) => e.id !== ev.id));
			fire("delete", { id: ev.id, metadata: ev.metadata });
		},
		[fire],
	);
	const onMoveOrResize = useCallback(
		(next: CalendarEvent, kind: "move" | "resize", prev: CalendarEvent) => {
			setEvents((list) => list.map((e) => (e.id === next.id ? next : e)));
			fire(kind, {
				id: next.id,
				start: next.start,
				end: next.end,
				oldStart: prev.start,
				oldEnd: prev.end,
				metadata: next.metadata,
			});
		},
		[fire],
	);

	// Cache Intl.DateTimeFormat instances — constructing them is expensive and
	// dtf() is called in tight render loops.
	const formattersRef = useRef<Map<string, Intl.DateTimeFormat>>(new Map());
	useEffect(() => {
		formattersRef.current.clear();
	}, [locale]);
	const dtf = useCallback(
		(opts: Intl.DateTimeFormatOptions) => {
			const key = JSON.stringify(opts);
			let formatter = formattersRef.current.get(key);
			if (!formatter) {
				formatter = new Intl.DateTimeFormat(locale, opts);
				formattersRef.current.set(key, formatter);
			}
			return formatter;
		},
		[locale],
	);

	const title = useMemo(() => {
		if (effectiveView === "day")
			return dtf({ weekday: "long", month: "long", day: "numeric" }).format(
				focusDate,
			);
		if (effectiveView === "week") {
			const days = getWeekDays(focusDate, firstDayOfWeek);
			return `${dtf({ month: "short", day: "numeric" }).format(days[0])} – ${dtf(
				{ month: "short", day: "numeric" },
			).format(days[6])}`;
		}
		return dtf({ month: "long", year: "numeric" }).format(focusDate);
	}, [effectiveView, focusDate, firstDayOfWeek, dtf]);

	const navigate = useCallback(
		(dir: -1 | 1) => {
			setFocusDate((d) => {
				if (effectiveView === "month") return addMonths(d, dir);
				if (effectiveView === "day") return addDays(d, dir);
				return addWeeks(d, dir);
			});
		},
		[effectiveView],
	);

	return (
		<div
			ref={containerRef}
			className={cn(
				"flex flex-col rounded-lg border border-border bg-card text-card-foreground overflow-hidden",
				resolveStyle(style),
			)}
			style={{ height: height ?? "600px", ...resolveInlineStyle(style) }}
		>
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<div className="flex items-center gap-1">
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						onClick={() => navigate(-1)}
						aria-label="Previous"
					>
						<ChevronLeftIcon className="h-4 w-4" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						onClick={() => navigate(1)}
						aria-label="Next"
					>
						<ChevronRightIcon className="h-4 w-4" />
					</Button>
					<Button
						variant="outline"
						size="sm"
						className="h-7 ml-1"
						onClick={() => setFocusDate(new Date())}
					>
						Today
					</Button>
					<h3 className="ml-2 text-sm font-semibold flex items-center gap-1.5">
						<CalendarDaysIcon className="h-4 w-4 text-muted-foreground" />
						{title}
					</h3>
					{isTriggering && (
						<Loader2Icon className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
					)}
				</div>
				{!isNarrow && (
					<div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
						{VIEWS.filter((v) => v !== "agenda").map((v) => (
							<button
								key={v}
								type="button"
								onClick={() => setView(v)}
								className={cn(
									"rounded px-2 py-1 text-xs capitalize transition-colors",
									view === v
										? "bg-primary text-primary-foreground"
										: "text-muted-foreground hover:bg-accent",
								)}
							>
								{v}
							</button>
						))}
					</div>
				)}
			</header>

			<div className="flex-1 overflow-auto">
				{effectiveView === "month" && (
					<MonthView
						focusDate={focusDate}
						events={events}
						weekStartsOn={firstDayOfWeek}
						showWeekends={showWeekends}
						editable={editable}
						selectable={selectable}
						dtf={dtf}
						onCreate={onCreate}
						onOpen={onOpen}
						onDelete={onDelete}
						onMove={(next, prev) => onMoveOrResize(next, "move", prev)}
					/>
				)}
				{(effectiveView === "week" || effectiveView === "day") && (
					<TimeGridView
						focusDate={focusDate}
						days={
							effectiveView === "day"
								? [focusDate]
								: getWeekDays(focusDate, firstDayOfWeek).filter(
										(d) =>
											showWeekends || (d.getDay() !== 0 && d.getDay() !== 6),
									)
						}
						events={events}
						minTime={minTime}
						maxTime={maxTime}
						slotDuration={slotDuration}
						showNowIndicator={showNowIndicator}
						editable={editable}
						selectable={selectable}
						dtf={dtf}
						onCreate={onCreate}
						onOpen={onOpen}
						onDelete={onDelete}
						onMoveOrResize={onMoveOrResize}
					/>
				)}
				{effectiveView === "agenda" && (
					<AgendaView
						focusDate={focusDate}
						events={events}
						selectable={selectable}
						dtf={dtf}
						onCreate={onCreate}
						onOpen={onOpen}
						onDelete={onDelete}
					/>
				)}
			</div>
		</div>
	);
}

type Dtf = (opts: Intl.DateTimeFormatOptions) => Intl.DateTimeFormat;

function eventColorStyle(ev: CalendarEvent): React.CSSProperties | undefined {
	if (!ev.color) return undefined;
	return { backgroundColor: ev.color, borderColor: ev.color };
}

function EventChip({
	ev,
	editable,
	onOpen,
	onDelete,
	compact,
	onPointerDown,
}: {
	ev: CalendarEvent;
	editable: boolean;
	onOpen: (ev: CalendarEvent) => void;
	onDelete: (ev: CalendarEvent) => void;
	compact?: boolean;
	onPointerDown?: (e: React.PointerEvent) => void;
}) {
	return (
		<button
			type="button"
			onPointerDown={onPointerDown}
			onClick={(e) => {
				e.stopPropagation();
				onOpen(ev);
			}}
			style={eventColorStyle(ev)}
			className={cn(
				"group/ev relative flex w-full items-center gap-1 truncate rounded px-1.5 text-left text-xs",
				"bg-primary/15 text-foreground border border-primary/30",
				compact ? "py-0.5" : "py-1",
				editable ? "cursor-grab active:cursor-grabbing" : "cursor-pointer",
			)}
		>
			<span className="truncate">{ev.title}</span>
			{editable && (
				<span
					role="button"
					tabIndex={-1}
					onClick={(e) => {
						e.stopPropagation();
						onDelete(ev);
					}}
					className="ml-auto hidden shrink-0 rounded p-0.5 hover:bg-background/40 group-hover/ev:block"
				>
					<XIcon className="h-3 w-3" />
				</span>
			)}
		</button>
	);
}

function MonthView({
	focusDate,
	events,
	weekStartsOn,
	showWeekends,
	editable,
	selectable,
	dtf,
	onCreate,
	onOpen,
	onDelete,
	onMove,
}: {
	focusDate: Date;
	events: CalendarEvent[];
	weekStartsOn: WeekStartsOn;
	showWeekends: boolean;
	editable: boolean;
	selectable: boolean;
	dtf: Dtf;
	onCreate: (start: Date, end: Date, allDay: boolean) => void;
	onOpen: (ev: CalendarEvent) => void;
	onDelete: (ev: CalendarEvent) => void;
	onMove: (next: CalendarEvent, prev: CalendarEvent) => void;
}) {
	const weeks = useMemo(
		() => getMonthWeeks(focusDate, weekStartsOn),
		[focusDate, weekStartsOn],
	);
	const weekdayLabels = weeks[0].map((d) =>
		dtf({ weekday: "short" }).format(d),
	);
	const dragRef = useRef<CalendarEvent | null>(null);

	const eventsForDay = useCallback(
		(day: Date) => events.filter((ev) => isSameDay(toDate(ev.start), day)),
		[events],
	);

	const handleDrop = useCallback(
		(e: React.PointerEvent) => {
			const dragged = dragRef.current;
			dragRef.current = null;
			if (!dragged) return;
			const target = document
				.elementFromPoint(e.clientX, e.clientY)
				?.closest("[data-day]") as HTMLElement | null;
			if (!target?.dataset.day) return;
			const targetDay = toDate(target.dataset.day);
			const orig = toDate(dragged.start);
			const newStart = set(targetDay, {
				hours: orig.getHours(),
				minutes: orig.getMinutes(),
			});
			if (isSameDay(orig, targetDay)) return;
			onMove(shiftEvent(dragged, newStart), dragged);
		},
		[onMove],
	);

	const visibleDays = (week: Date[]) =>
		week.filter((d) => showWeekends || (d.getDay() !== 0 && d.getDay() !== 6));

	return (
		<div
			className="flex min-h-full flex-col"
			onPointerUp={editable ? handleDrop : undefined}
		>
			<div
				className="grid border-b border-border text-xs font-medium text-muted-foreground"
				style={{
					gridTemplateColumns: `repeat(${showWeekends ? 7 : 5}, minmax(0,1fr))`,
				}}
			>
				{weekdayLabels
					.filter(
						(_, i) =>
							showWeekends ||
							(weeks[0][i].getDay() !== 0 && weeks[0][i].getDay() !== 6),
					)
					.map((label) => (
						<div key={label} className="px-2 py-1.5 text-center">
							{label}
						</div>
					))}
			</div>
			<div
				className="grid flex-1"
				style={{ gridTemplateRows: `repeat(${weeks.length},minmax(90px,1fr))` }}
			>
				{weeks.map((week) => (
					<div
						key={week[0].toISOString()}
						className="grid border-b border-border"
						style={{
							gridTemplateColumns: `repeat(${showWeekends ? 7 : 5}, minmax(0,1fr))`,
						}}
					>
						{visibleDays(week).map((day) => {
							const dayEvents = eventsForDay(day);
							const inMonth = isSameMonth(day, focusDate);
							return (
								<div
									key={day.toISOString()}
									data-day={iso(startOfDay(day))}
									onClick={() => {
										if (selectable)
											onCreate(
												startOfDay(day),
												addDays(startOfDay(day), 1),
												true,
											);
									}}
									className={cn(
										"flex flex-col gap-0.5 border-r border-border p-1 last:border-r-0",
										selectable && "cursor-pointer hover:bg-accent/40",
										!inMonth && "bg-muted/30 text-muted-foreground",
									)}
								>
									<span
										className={cn(
											"mb-0.5 inline-flex h-5 w-5 items-center justify-center self-end rounded-full text-xs",
											isToday(day) &&
												"bg-primary text-primary-foreground font-semibold",
										)}
									>
										{day.getDate()}
									</span>
									<div className="flex flex-col gap-0.5 overflow-hidden">
										{dayEvents.slice(0, 4).map((ev) => (
											<EventChip
												key={ev.id}
												ev={ev}
												editable={editable && ev.editable !== false}
												onOpen={onOpen}
												onDelete={onDelete}
												compact
												onPointerDown={
													editable && ev.editable !== false
														? () => {
																dragRef.current = ev;
															}
														: undefined
												}
											/>
										))}
										{dayEvents.length > 4 && (
											<span className="px-1 text-[10px] text-muted-foreground">
												+{dayEvents.length - 4} more
											</span>
										)}
									</div>
								</div>
							);
						})}
					</div>
				))}
			</div>
		</div>
	);
}

const HOUR_HEIGHT = 48;

function TimeGridView({
	days,
	events,
	minTime,
	maxTime,
	slotDuration,
	showNowIndicator,
	editable,
	selectable,
	dtf,
	onCreate,
	onOpen,
	onDelete,
	onMoveOrResize,
}: {
	focusDate: Date;
	days: Date[];
	events: CalendarEvent[];
	minTime: number;
	maxTime: number;
	slotDuration: number;
	showNowIndicator: boolean;
	editable: boolean;
	selectable: boolean;
	dtf: Dtf;
	onCreate: (start: Date, end: Date, allDay: boolean) => void;
	onOpen: (ev: CalendarEvent) => void;
	onDelete: (ev: CalendarEvent) => void;
	onMoveOrResize: (
		next: CalendarEvent,
		kind: "move" | "resize",
		prev: CalendarEvent,
	) => void;
}) {
	const totalMinutes = Math.max(60, maxTime - minTime);
	const pxPerMinute = HOUR_HEIGHT / 60;
	const gridHeight = totalMinutes * pxPerMinute;
	const hours = useMemo(() => {
		const list: number[] = [];
		for (let m = minTime; m <= maxTime; m += 60) list.push(m);
		return list;
	}, [minTime, maxTime]);

	const drag = useRef<{
		ev: CalendarEvent;
		kind: "move" | "resize";
		startY: number;
		originStartMin: number;
		durationMin: number;
		day: Date;
	} | null>(null);
	const [preview, setPreview] = useState<CalendarEvent | null>(null);

	const snap = useCallback(
		(minutes: number) => Math.round(minutes / slotDuration) * slotDuration,
		[slotDuration],
	);

	const onPointerMove = useCallback(
		(e: React.PointerEvent) => {
			const d = drag.current;
			if (!d) return;
			const deltaMin = (e.clientY - d.startY) / pxPerMinute;
			if (d.kind === "move") {
				const newStartMin = Math.max(
					minTime,
					Math.min(maxTime - d.durationMin, snap(d.originStartMin + deltaMin)),
				);
				const start = set(d.day, {
					hours: Math.floor(newStartMin / 60),
					minutes: newStartMin % 60,
					seconds: 0,
					milliseconds: 0,
				});
				setPreview(shiftEvent(d.ev, start));
			} else {
				const newDuration = Math.max(
					slotDuration,
					snap(d.durationMin + deltaMin),
				);
				const start = toDate(d.ev.start);
				setPreview({
					...d.ev,
					end: iso(new Date(start.getTime() + newDuration * 60000)),
				});
			}
		},
		[pxPerMinute, minTime, maxTime, snap, slotDuration],
	);

	const onPointerUp = useCallback(() => {
		const d = drag.current;
		drag.current = null;
		if (d && preview) onMoveOrResize(preview, d.kind, d.ev);
		setPreview(null);
	}, [preview, onMoveOrResize]);

	const displayEvents = useCallback(
		(day: Date) =>
			events.map((ev) =>
				preview && preview.id === ev.id && isSameDay(toDate(preview.start), day)
					? preview
					: ev,
			),
		[events, preview],
	);

	return (
		<div
			className="flex min-h-full"
			onPointerMove={editable ? onPointerMove : undefined}
			onPointerUp={editable ? onPointerUp : undefined}
			onPointerLeave={editable ? onPointerUp : undefined}
		>
			<div className="w-14 shrink-0 select-none border-r border-border">
				<div className="h-8" />
				{hours.map((m) => (
					<div
						key={m}
						className="relative text-right pr-1 text-[10px] text-muted-foreground"
						style={{ height: HOUR_HEIGHT }}
					>
						<span className="absolute -top-1.5 right-1">
							{String(Math.floor(m / 60)).padStart(2, "0")}:00
						</span>
					</div>
				))}
			</div>
			<div className="flex flex-1">
				{days.map((day) => (
					<div
						key={day.toISOString()}
						className="flex-1 border-r border-border last:border-r-0"
					>
						<div className="flex h-8 items-center justify-center border-b border-border text-xs">
							<span className="text-muted-foreground">
								{dtf({ weekday: "short" }).format(day)}
							</span>
							<span
								className={cn(
									"ml-1.5 inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1 font-medium",
									isToday(day) && "bg-primary text-primary-foreground",
								)}
							>
								{day.getDate()}
							</span>
						</div>
						<div
							className="relative"
							style={{ height: gridHeight }}
							onClick={(e) => {
								if (!selectable) return;
								const rect = e.currentTarget.getBoundingClientRect();
								const minute = snap(
									minTime + (e.clientY - rect.top) / pxPerMinute,
								);
								const start = set(day, {
									hours: Math.floor(minute / 60),
									minutes: minute % 60,
									seconds: 0,
									milliseconds: 0,
								});
								onCreate(
									start,
									new Date(start.getTime() + slotDuration * 60000),
									false,
								);
							}}
						>
							{hours.map((m) => (
								<div
									key={m}
									className="absolute inset-x-0 border-b border-border/60"
									style={{
										top: (m - minTime) * pxPerMinute,
										height: HOUR_HEIGHT,
									}}
								/>
							))}
							{showNowIndicator && isToday(day) && (
								<div
									className="absolute inset-x-0 z-10 border-t-2 border-red-500"
									style={{
										top: (minutesOf(new Date()) - minTime) * pxPerMinute,
									}}
								/>
							)}
							{displayEvents(day)
								.filter((ev) => !ev.allDay && isSameDay(toDate(ev.start), day))
								.map((ev) => {
									const startMin = minutesOf(toDate(ev.start));
									const durationMin = eventDurationMinutes(ev);
									const canEdit = editable && ev.editable !== false;
									return (
										<div
											key={ev.id}
											className={cn(
												"group/ev absolute inset-x-1 z-20 overflow-hidden rounded border border-primary/40 bg-primary/20 px-1.5 py-0.5 text-xs",
												canEdit
													? "cursor-grab active:cursor-grabbing"
													: "cursor-pointer",
											)}
											style={{
												top: (startMin - minTime) * pxPerMinute,
												height: Math.max(16, durationMin * pxPerMinute),
												...eventColorStyle(ev),
											}}
											onPointerDown={(e) => {
												if (!canEdit) return;
												e.stopPropagation();
												drag.current = {
													ev,
													kind: "move",
													startY: e.clientY,
													originStartMin: startMin,
													durationMin,
													day,
												};
											}}
											onClick={(e) => {
												e.stopPropagation();
												if (!drag.current) onOpen(ev);
											}}
										>
											<div className="flex items-start justify-between gap-1">
												<span className="truncate font-medium">{ev.title}</span>
												{canEdit && (
													<span
														role="button"
														tabIndex={-1}
														onClick={(e) => {
															e.stopPropagation();
															onDelete(ev);
														}}
														className="hidden shrink-0 rounded p-0.5 hover:bg-background/40 group-hover/ev:block"
													>
														<XIcon className="h-3 w-3" />
													</span>
												)}
											</div>
											{canEdit && (
												<div
													onPointerDown={(e) => {
														e.stopPropagation();
														drag.current = {
															ev,
															kind: "resize",
															startY: e.clientY,
															originStartMin: startMin,
															durationMin,
															day,
														};
													}}
													className="absolute inset-x-0 bottom-0 h-1.5 cursor-ns-resize"
												/>
											)}
										</div>
									);
								})}
						</div>
					</div>
				))}
			</div>
		</div>
	);
}

function AgendaView({
	focusDate,
	events,
	selectable,
	dtf,
	onCreate,
	onOpen,
	onDelete,
}: {
	focusDate: Date;
	events: CalendarEvent[];
	selectable: boolean;
	dtf: Dtf;
	onCreate: (start: Date, end: Date, allDay: boolean) => void;
	onOpen: (ev: CalendarEvent) => void;
	onDelete: (ev: CalendarEvent) => void;
}) {
	const days = useMemo(
		() =>
			Array.from({ length: 14 }, (_, i) => addDays(startOfDay(focusDate), i)),
		[focusDate],
	);
	return (
		<div className="divide-y divide-border">
			{days.map((day) => {
				const dayEvents = events
					.filter((ev) => isSameDay(toDate(ev.start), day))
					.sort(
						(a, b) => toDate(a.start).getTime() - toDate(b.start).getTime(),
					);
				return (
					<div key={day.toISOString()} className="flex gap-3 px-3 py-2">
						<div className="w-16 shrink-0 pt-1 text-center">
							<div className="text-xs text-muted-foreground">
								{dtf({ weekday: "short" }).format(day)}
							</div>
							<div
								className={cn(
									"mx-auto inline-flex h-7 w-7 items-center justify-center rounded-full text-sm",
									isToday(day) &&
										"bg-primary text-primary-foreground font-semibold",
								)}
							>
								{day.getDate()}
							</div>
						</div>
						<div className="flex flex-1 flex-col gap-1 py-0.5">
							{dayEvents.length === 0 && (
								<span className="py-1 text-xs text-muted-foreground/60">
									No events
								</span>
							)}
							{dayEvents.map((ev) => (
								<div
									key={ev.id}
									className="group/ev flex items-center gap-2 rounded-md border border-border px-2 py-1.5 hover:bg-accent/40"
								>
									<span
										className="h-2.5 w-2.5 shrink-0 rounded-full bg-primary"
										style={ev.color ? { backgroundColor: ev.color } : undefined}
									/>
									<button
										type="button"
										onClick={() => onOpen(ev)}
										className="flex-1 truncate text-left text-sm"
									>
										<span className="font-medium">{ev.title}</span>
										{!ev.allDay && (
											<span className="ml-2 text-xs text-muted-foreground">
												{dtf({ hour: "2-digit", minute: "2-digit" }).format(
													toDate(ev.start),
												)}
											</span>
										)}
									</button>
									<button
										type="button"
										onClick={() => onDelete(ev)}
										className="hidden rounded p-1 text-muted-foreground hover:bg-background group-hover/ev:block"
										aria-label="Delete event"
									>
										<XIcon className="h-3.5 w-3.5" />
									</button>
								</div>
							))}
							{selectable && (
								<button
									type="button"
									onClick={() =>
										onCreate(startOfDay(day), addDays(startOfDay(day), 1), true)
									}
									className="flex items-center gap-1 self-start rounded px-1 py-0.5 text-xs text-muted-foreground hover:text-foreground"
								>
									<PlusIcon className="h-3 w-3" /> Add
								</button>
							)}
						</div>
					</div>
				);
			})}
		</div>
	);
}
