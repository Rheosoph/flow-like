"use client";

import {
	addDays,
	addMonths,
	addWeeks,
	differenceInCalendarDays,
	isSameDay,
	isSameMonth,
	isToday,
	set,
	startOfDay,
} from "date-fns";
import {
	ChevronLeftIcon,
	ChevronRightIcon,
	CopyIcon,
	EyeIcon,
	Loader2Icon,
	PencilIcon,
	PlusIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import {
	type CSSProperties,
	type KeyboardEvent as ReactKeyboardEvent,
	type ReactNode,
	type PointerEvent as ReactPointerEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../../lib/utils";
import {
	Button,
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "../../ui/index";
import {
	useComponentActionTrigger,
	useIsComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	EventDialog,
	type EventDialogState,
	PlanningContextMenu,
	type PlanningDialogMode,
	type PlanningMenuAction,
	planningTint,
} from "../planning-dialogs";
import {
	type TimedEventLayout,
	type WeekStartsOn,
	densityPreset,
	eventDurationMinutes,
	eventEnd,
	genId,
	getMonthWeeks,
	getWeekDays,
	layoutOverlappingEvents,
	normalizeCalendarEvents,
	parseTimeToMinutes,
	toDate,
	toDateInput,
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
const ALL_DAY_MAX_LANES = 3;
const DRAG_THRESHOLD_PX = 3;
const AGENDA_DAYS = 14;

type Dtf = (opts: Intl.DateTimeFormatOptions) => Intl.DateTimeFormat;
// Structural ref type so it works with both React 18 and 19 ref typings.
type MovedRef = { current: boolean };

interface EventHandlers {
	openEvent: (ev: CalendarEvent) => void;
	editEvent: (ev: CalendarEvent) => void;
	duplicateEvent: (ev: CalendarEvent) => void;
	deleteEvent: (ev: CalendarEvent) => void;
	openCreate: (start: Date, end: Date, allDay: boolean) => void;
	moveOrResize: (
		next: CalendarEvent,
		kind: "move" | "resize",
		prev: CalendarEvent,
	) => void;
}

function iso(date: Date): string {
	return date.toISOString();
}

function minutesOf(date: Date): number {
	return date.getHours() * 60 + date.getMinutes();
}

function atMinute(day: Date, minutes: number): Date {
	return set(day, {
		hours: Math.floor(minutes / 60),
		minutes: minutes % 60,
		seconds: 0,
		milliseconds: 0,
	});
}

/** Move an event's start (and its end by the same delta) to a new instant. */
function shiftEvent(ev: CalendarEvent, newStart: Date): CalendarEvent {
	if (ev.allDay) {
		// Calendar-day arithmetic: millisecond deltas shrink/grow all-day spans
		// across DST transitions.
		const spanDays = differenceInCalendarDays(
			startOfDay(eventEnd(ev)),
			startOfDay(toDate(ev.start)),
		);
		const start = startOfDay(newStart);
		return {
			...ev,
			start: toDateInput(start),
			end: toDateInput(addDays(start, spanDays)),
		};
	}
	const durationMs = eventEnd(ev).getTime() - toDate(ev.start).getTime();
	return {
		...ev,
		start: iso(newStart),
		end: iso(new Date(newStart.getTime() + durationMs)),
	};
}

function chipColorStyle(
	ev: CalendarEvent,
	tint: number,
): CSSProperties | undefined {
	if (!ev.color) return undefined;
	return {
		borderLeftColor: ev.color,
		backgroundColor: planningTint(ev.color, tint),
	};
}

function isMultiDay(ev: CalendarEvent): boolean {
	return (
		differenceInCalendarDays(
			startOfDay(eventEnd(ev)),
			startOfDay(toDate(ev.start)),
		) >= 1
	);
}

/** Whether `day` falls within the event's (inclusive) day span. */
function eventTouchesDay(ev: CalendarEvent, day: Date): boolean {
	const t = startOfDay(day).getTime();
	return (
		t >= startOfDay(toDate(ev.start)).getTime() &&
		t <= startOfDay(eventEnd(ev)).getTime()
	);
}

function formatHourLabel(minutes: number): string {
	return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:00`;
}

function sortDayEvents(a: CalendarEvent, b: CalendarEvent): number {
	if (!!a.allDay !== !!b.allDay) return a.allDay ? -1 : 1;
	return toDate(a.start).getTime() - toDate(b.start).getTime();
}

function keyActivate(action: () => void) {
	return (e: ReactKeyboardEvent) => {
		if (e.key === "Enter" || e.key === " ") {
			e.preventDefault();
			action();
		}
	};
}

function EventContextMenu({
	ev,
	editable,
	handlers,
	children,
}: {
	ev: CalendarEvent;
	editable: boolean;
	handlers: EventHandlers;
	children: ReactNode;
}) {
	const canEdit = editable && ev.editable !== false;
	const groups: PlanningMenuAction[][] = [
		[
			{
				label: "View details",
				icon: <EyeIcon className="h-3.5 w-3.5" />,
				onSelect: () => handlers.openEvent(ev),
			},
			...(canEdit
				? [
						{
							label: "Edit",
							icon: <PencilIcon className="h-3.5 w-3.5" />,
							onSelect: () => handlers.editEvent(ev),
						},
					]
				: []),
		],
		canEdit
			? [
					{
						label: "Duplicate",
						icon: <CopyIcon className="h-3.5 w-3.5" />,
						onSelect: () => handlers.duplicateEvent(ev),
					},
				]
			: [],
		canEdit
			? [
					{
						label: "Delete",
						icon: <Trash2Icon className="h-3.5 w-3.5" />,
						destructive: true,
						onSelect: () => handlers.deleteEvent(ev),
					},
				]
			: [],
	];
	return <PlanningContextMenu groups={groups}>{children}</PlanningContextMenu>;
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
	const rawView = useResolved<string>(component.view);
	// Unknown view values fall back to month instead of a blank body.
	const viewProp: CalendarView = VIEWS.includes(rawView as CalendarView)
		? (rawView as CalendarView)
		: "month";
	const dateProp = useResolved<string>(component.date);
	const titleProp = useResolved<string>(component.title) || undefined;
	const densityValue = useResolved<string>(component.density);
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
	const showAllDay = useResolved<boolean>(component.showAllDay) ?? true;
	const showViewSwitcher =
		useResolved<boolean>(component.showViewSwitcher) ?? true;
	const locale = useResolved<string>(component.locale) || undefined;
	const height = useResolved<string>(component.height);
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const compactBreakpoint =
		useResolved<number>(component.compactBreakpoint) ??
		DEFAULT_COMPACT_BREAKPOINT;

	const preset = densityPreset(densityValue);
	const compactDensity = densityValue === "compact";
	// Creating events is an edit: selection affordances require both flags.
	const canCreate = selectable && editable;

	// `useResolved` re-parses `literalJson` into a fresh array every render, so
	// key the memo/effect on the serialized content — otherwise the sync effect
	// below would fire on every render and wipe local edits/creates instantly.
	const eventsKey = JSON.stringify(rawEvents ?? null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: eventsKey is the stable identity of rawEvents
	const resolvedEvents = useMemo(
		() => normalizeCalendarEvents(rawEvents),
		[eventsKey],
	);
	// Local overlay lets edits/creates feel instant before the workflow round-trips.
	const [events, setEvents] = useState<CalendarEvent[]>(resolvedEvents);
	useEffect(() => setEvents(resolvedEvents), [resolvedEvents]);

	const [view, setView] = useState<CalendarView>(viewProp);
	useEffect(() => setView(viewProp), [viewProp]);

	// Without an explicit `date`, focus the month that actually holds events so
	// the preset/sample data is visible instead of an empty current month.
	const [focusDate, setFocusDate] = useState<Date>(() => {
		if (dateProp) return toDate(dateProp);
		if (resolvedEvents.length > 0) {
			return resolvedEvents.reduce((min, e) => {
				const d = toDate(e.start);
				return d < min ? d : min;
			}, toDate(resolvedEvents[0].start));
		}
		return new Date();
	});
	useEffect(() => {
		if (dateProp) setFocusDate(toDate(dateProp));
	}, [dateProp]);

	const [containerWidth, setContainerWidth] = useState(0);
	useEffect(() => {
		if (typeof ResizeObserver === "undefined") return;
		const el = containerRef.current;
		if (!el) return;
		const obs = new ResizeObserver((entries) => {
			for (const entry of entries) setContainerWidth(entry.contentRect.width);
		});
		obs.observe(el);
		return () => obs.disconnect();
	}, []);
	const isNarrow =
		responsive && containerWidth > 0 && containerWidth < compactBreakpoint;
	const effectiveView: CalendarView = isNarrow ? "agenda" : view;

	const fire = useCallback(
		(interaction: string, extra: Record<string, unknown>) => {
			void trigger(component.actions, { interaction, ...extra });
		},
		[trigger, component.actions],
	);

	const [dialogState, setDialogState] = useState<EventDialogState | null>(null);
	const closeDialog = useCallback(() => setDialogState(null), []);

	// Suppresses the click that follows a >3px drag (click fires after pointerup).
	const movedRef = useRef(false);

	const openEvent = useCallback(
		(ev: CalendarEvent) => {
			if (movedRef.current) {
				movedRef.current = false;
				return;
			}
			setDialogState({ event: ev, mode: "view" });
			fire("open", { id: ev.id, metadata: ev.metadata });
		},
		[fire],
	);

	const editEvent = useCallback((ev: CalendarEvent) => {
		setDialogState({ event: ev, mode: "edit" });
	}, []);

	const openCreate = useCallback((start: Date, end: Date, allDay: boolean) => {
		setDialogState({
			event: {
				id: genId("event"),
				title: "",
				start: allDay ? toDateInput(start) : iso(start),
				end: allDay ? toDateInput(end) : iso(end),
				allDay,
			},
			mode: "create",
		});
	}, []);

	const deleteEvent = useCallback(
		(ev: CalendarEvent) => {
			setEvents((list) => list.filter((e) => e.id !== ev.id));
			fire("delete", { id: ev.id, metadata: ev.metadata });
		},
		[fire],
	);

	const duplicateEvent = useCallback(
		(ev: CalendarEvent) => {
			const copy: CalendarEvent = { ...ev, id: genId("event") };
			setEvents((list) => [...list, copy]);
			fire("create", {
				id: copy.id,
				start: copy.start,
				end: copy.end,
				allDay: copy.allDay ?? false,
				event: copy,
				sourceId: ev.id,
			});
		},
		[fire],
	);

	const moveOrResize = useCallback(
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

	const handlers = useMemo<EventHandlers>(
		() => ({
			openEvent,
			editEvent,
			duplicateEvent,
			deleteEvent,
			openCreate,
			moveOrResize,
		}),
		[
			openEvent,
			editEvent,
			duplicateEvent,
			deleteEvent,
			openCreate,
			moveOrResize,
		],
	);

	const handleDialogSave = useCallback(
		(
			next: CalendarEvent,
			original: CalendarEvent,
			mode: PlanningDialogMode,
		) => {
			if (mode === "create") {
				setEvents((list) => [...list, next]);
				fire("create", {
					id: next.id,
					start: next.start,
					end: next.end,
					allDay: next.allDay ?? false,
					event: next,
				});
				// Jump to the new event's date so it is visible in every view
				// (the agenda window in particular starts at focusDate).
				setFocusDate(toDate(next.start));
			} else {
				setEvents((list) => list.map((e) => (e.id === next.id ? next : e)));
				fire("update", { id: next.id, event: next, previous: original });
			}
		},
		[fire],
	);

	// Anchor header-created events to the focused period, not absolute today —
	// today may lie outside the currently displayed month/week/agenda window.
	const onHeaderCreate = useCallback(() => {
		const base = startOfDay(focusDate);
		openCreate(set(base, { hours: 9 }), set(base, { hours: 10 }), false);
	}, [openCreate, focusDate]);

	// Cache Intl.DateTimeFormat instances — constructing them is expensive and
	// dtf() is called in tight render loops. Keyed by locale so a locale change
	// takes effect in the very render it arrives in.
	const formattersRef = useRef<Map<string, Intl.DateTimeFormat>>(new Map());
	const dtf = useCallback(
		(opts: Intl.DateTimeFormatOptions) => {
			const key = `${locale ?? ""}|${JSON.stringify(opts)}`;
			let formatter = formattersRef.current.get(key);
			if (!formatter) {
				formatter = new Intl.DateTimeFormat(locale, opts);
				formattersRef.current.set(key, formatter);
			}
			return formatter;
		},
		[locale],
	);

	const periodLabel = useMemo(() => {
		if (effectiveView === "day")
			return dtf({ weekday: "long", month: "long", day: "numeric" }).format(
				focusDate,
			);
		if (effectiveView === "week") {
			const days = getWeekDays(focusDate, firstDayOfWeek);
			const fmt = dtf({ month: "short", day: "numeric" });
			return `${fmt.format(days[0])} – ${fmt.format(days[6])}`;
		}
		if (effectiveView === "agenda") {
			const fmt = dtf({ month: "short", day: "numeric" });
			return `${fmt.format(focusDate)} – ${fmt.format(
				addDays(focusDate, AGENDA_DAYS - 1),
			)}`;
		}
		return dtf({ month: "long", year: "numeric" }).format(focusDate);
	}, [effectiveView, focusDate, firstDayOfWeek, dtf]);

	const navigate = useCallback(
		(dir: -1 | 1) => {
			setFocusDate((d) => {
				if (effectiveView === "month") return addMonths(d, dir);
				if (effectiveView === "day") return addDays(d, dir);
				if (effectiveView === "agenda") return addDays(d, dir * AGENDA_DAYS);
				return addWeeks(d, dir);
			});
		},
		[effectiveView],
	);

	const gridDays = useMemo(() => {
		if (effectiveView === "day") return [focusDate];
		return getWeekDays(focusDate, firstDayOfWeek).filter(
			(d) => showWeekends || (d.getDay() !== 0 && d.getDay() !== 6),
		);
	}, [effectiveView, focusDate, firstDayOfWeek, showWeekends]);

	return (
		<div
			ref={containerRef}
			className={cn(
				"flex flex-col overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-sm",
				resolveStyle(style),
			)}
			style={{ height: height ?? "600px", ...resolveInlineStyle(style) }}
		>
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<div className="flex min-w-0 items-center gap-1">
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7 shrink-0"
						onClick={() => navigate(-1)}
						aria-label="Previous"
					>
						<ChevronLeftIcon className="h-4 w-4" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7 shrink-0"
						onClick={() => navigate(1)}
						aria-label="Next"
					>
						<ChevronRightIcon className="h-4 w-4" />
					</Button>
					{!isNarrow && (
						<Button
							variant="outline"
							size="sm"
							className="ml-1 h-7 shrink-0"
							onClick={() => setFocusDate(new Date())}
						>
							Today
						</Button>
					)}
					<div className="ml-2 flex min-w-0 items-baseline gap-2">
						<h3 className="truncate text-sm font-semibold">
							{titleProp ?? periodLabel}
						</h3>
						{titleProp && (
							<span className="truncate text-xs text-muted-foreground">
								{periodLabel}
							</span>
						)}
					</div>
					{isTriggering && (
						<Loader2Icon className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
					)}
				</div>
				{!isNarrow && (
					<div className="flex shrink-0 items-center gap-2">
						{editable && (
							<Button
								variant="outline"
								size="sm"
								className="h-7"
								onClick={onHeaderCreate}
							>
								<PlusIcon className="mr-1 h-3.5 w-3.5" /> Event
							</Button>
						)}
						{showViewSwitcher && (
							<div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
								{VIEWS.map((v) => (
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
					</div>
				)}
			</header>

			<div className="relative flex-1 overflow-hidden">
				{effectiveView === "month" && (
					<MonthView
						focusDate={focusDate}
						events={events}
						weekStartsOn={firstDayOfWeek}
						showWeekends={showWeekends}
						editable={editable}
						selectable={canCreate}
						showTime={!compactDensity}
						maxVisible={compactDensity ? 2 : 3}
						rowMinHeight={preset.monthRowMinHeight}
						dtf={dtf}
						movedRef={movedRef}
						handlers={handlers}
					/>
				)}
				{(effectiveView === "week" || effectiveView === "day") && (
					<TimeGridView
						days={gridDays}
						events={events}
						minTime={minTime}
						maxTime={maxTime}
						slotDuration={slotDuration}
						hourHeight={preset.hourHeight}
						showWeekendShade={showWeekends}
						showNowIndicator={showNowIndicator}
						showAllDay={showAllDay}
						editable={editable}
						selectable={canCreate}
						dtf={dtf}
						movedRef={movedRef}
						handlers={handlers}
					/>
				)}
				{effectiveView === "agenda" && (
					<div className="h-full overflow-y-auto">
						<AgendaView
							focusDate={focusDate}
							events={events}
							editable={editable}
							dtf={dtf}
							handlers={handlers}
						/>
					</div>
				)}
			</div>

			<EventDialog
				state={dialogState}
				editable={editable}
				locale={locale}
				onClose={closeDialog}
				onSave={handleDialogSave}
				onDelete={deleteEvent}
			/>
		</div>
	);
}

// ── Month view ──────────────────────────────────────────────────────

function MonthView({
	focusDate,
	events,
	weekStartsOn,
	showWeekends,
	editable,
	selectable,
	showTime,
	maxVisible,
	rowMinHeight,
	dtf,
	movedRef,
	handlers,
}: {
	focusDate: Date;
	events: CalendarEvent[];
	weekStartsOn: WeekStartsOn;
	showWeekends: boolean;
	editable: boolean;
	selectable: boolean;
	showTime: boolean;
	maxVisible: number;
	rowMinHeight: number;
	dtf: Dtf;
	movedRef: MovedRef;
	handlers: EventHandlers;
}) {
	const weeks = useMemo(
		() => getMonthWeeks(focusDate, weekStartsOn),
		[focusDate, weekStartsOn],
	);
	const dragRef = useRef<{ ev: CalendarEvent; x: number; y: number } | null>(
		null,
	);
	// Drag-across-days on empty cells paints a multi-day range (Outlook-style).
	const [paint, setPaint] = useState<{ anchor: Date; current: Date } | null>(
		null,
	);
	const [dropDay, setDropDay] = useState<Date | null>(null);
	const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

	const visibleDays = useCallback(
		(week: Date[]) =>
			week.filter(
				(d) => showWeekends || (d.getDay() !== 0 && d.getDay() !== 6),
			),
		[showWeekends],
	);
	const headerDays = visibleDays(weeks[0]);

	const eventsForDay = useCallback(
		(day: Date) =>
			events.filter((ev) => eventTouchesDay(ev, day)).sort(sortDayEvents),
		[events],
	);

	const dayAt = useCallback((x: number, y: number): Date | null => {
		const target = document
			.elementFromPoint(x, y)
			?.closest("[data-day]") as HTMLElement | null;
		return target?.dataset.day ? toDate(target.dataset.day) : null;
	}, []);

	const onChipDragStart = useCallback(
		(ev: CalendarEvent, e: ReactPointerEvent) => {
			if (e.button !== 0) return;
			e.stopPropagation();
			movedRef.current = false;
			dragRef.current = { ev, x: e.clientX, y: e.clientY };
		},
		[movedRef],
	);

	const onCellPaintStart = useCallback(
		(day: Date, e: ReactPointerEvent) => {
			if (!selectable || e.button !== 0 || dragRef.current) return;
			movedRef.current = false;
			setPaint({ anchor: day, current: day });
		},
		[selectable, movedRef],
	);

	const onPointerMove = useCallback(
		(e: ReactPointerEvent) => {
			const d = dragRef.current;
			if (d) {
				if (Math.hypot(e.clientX - d.x, e.clientY - d.y) > DRAG_THRESHOLD_PX) {
					movedRef.current = true;
					setDropDay(dayAt(e.clientX, e.clientY));
					setCursor({ x: e.clientX, y: e.clientY });
				}
				return;
			}
			if (paint) {
				const day = dayAt(e.clientX, e.clientY);
				if (day && !isSameDay(day, paint.current)) {
					movedRef.current = true;
					setPaint({ ...paint, current: day });
				}
				if (movedRef.current) setCursor({ x: e.clientX, y: e.clientY });
			}
		},
		[movedRef, paint, dayAt],
	);

	const onPointerUp = useCallback(
		(e: ReactPointerEvent) => {
			const d = dragRef.current;
			dragRef.current = null;
			setDropDay(null);
			setCursor(null);
			if (d) {
				if (!movedRef.current) return;
				// Clear the suppression flag after the trailing click has fired.
				window.setTimeout(() => {
					movedRef.current = false;
				}, 0);
				const targetDay = dayAt(e.clientX, e.clientY);
				if (!targetDay) return;
				const orig = toDate(d.ev.start);
				if (isSameDay(orig, targetDay)) return;
				const newStart = set(targetDay, {
					hours: orig.getHours(),
					minutes: orig.getMinutes(),
				});
				handlers.moveOrResize(shiftEvent(d.ev, newStart), "move", d.ev);
				return;
			}
			if (paint) {
				setPaint(null);
				window.setTimeout(() => {
					movedRef.current = false;
				}, 0);
				const [a, b] = [paint.anchor, paint.current].sort(
					(x, y) => x.getTime() - y.getTime(),
				);
				handlers.openCreate(startOfDay(a), startOfDay(b), true);
			}
		},
		[movedRef, paint, dayAt, handlers],
	);

	const cancelPointer = useCallback(() => {
		dragRef.current = null;
		movedRef.current = false;
		setPaint(null);
		setDropDay(null);
		setCursor(null);
	}, [movedRef]);

	const monthDragLabel = useMemo(() => {
		const fmt = dtf({ weekday: "short", month: "short", day: "numeric" });
		if (dropDay) return fmt.format(dropDay);
		if (paint) {
			const [a, b] = [paint.anchor, paint.current].sort(
				(x, y) => x.getTime() - y.getTime(),
			);
			return isSameDay(a, b)
				? fmt.format(a)
				: `${fmt.format(a)} – ${fmt.format(b)}`;
		}
		return null;
	}, [dropDay, paint, dtf]);

	const inPaintRange = useCallback(
		(day: Date) => {
			if (!paint) return false;
			const t = startOfDay(day).getTime();
			const a = startOfDay(paint.anchor).getTime();
			const b = startOfDay(paint.current).getTime();
			return t >= Math.min(a, b) && t <= Math.max(a, b);
		},
		[paint],
	);

	const interactive = editable || selectable;
	return (
		<div
			className="flex h-full select-none flex-col overflow-y-auto"
			onPointerMove={interactive ? onPointerMove : undefined}
			onPointerUp={interactive ? onPointerUp : undefined}
			onPointerLeave={interactive ? cancelPointer : undefined}
		>
			<div
				className="sticky top-0 z-20 grid shrink-0 border-b border-border bg-card text-xs font-medium text-muted-foreground"
				style={{
					gridTemplateColumns: `repeat(${headerDays.length}, minmax(0,1fr))`,
				}}
			>
				{headerDays.map((d) => (
					<div key={d.toISOString()} className="px-2 py-1.5 text-center">
						{dtf({ weekday: "short" }).format(d)}
					</div>
				))}
			</div>
			<div
				className="grid flex-1"
				style={{
					gridTemplateRows: `repeat(${weeks.length}, minmax(${rowMinHeight}px, 1fr))`,
				}}
			>
				{weeks.map((week) => (
					<div
						key={week[0].toISOString()}
						className="grid border-b border-border last:border-b-0"
						style={{
							gridTemplateColumns: `repeat(${headerDays.length}, minmax(0,1fr))`,
						}}
					>
						{visibleDays(week).map((day) => (
							<MonthDayCell
								key={day.toISOString()}
								day={day}
								inMonth={isSameMonth(day, focusDate)}
								events={eventsForDay(day)}
								editable={editable}
								selectable={selectable}
								showTime={showTime}
								maxVisible={maxVisible}
								painted={inPaintRange(day)}
								dropTarget={!!dropDay && isSameDay(dropDay, day)}
								dtf={dtf}
								movedRef={movedRef}
								handlers={handlers}
								onChipDragStart={onChipDragStart}
								onPaintStart={onCellPaintStart}
							/>
						))}
					</div>
				))}
			</div>
			{cursor && monthDragLabel && (
				<div
					className="pointer-events-none fixed z-50 rounded-md border border-border bg-popover px-2 py-1 text-[10px] font-medium text-popover-foreground shadow-md"
					style={{ left: cursor.x + 12, top: cursor.y + 14 }}
				>
					{monthDragLabel}
				</div>
			)}
		</div>
	);
}

function MonthDayCell({
	day,
	inMonth,
	events,
	editable,
	selectable,
	showTime,
	maxVisible,
	painted,
	dropTarget,
	dtf,
	movedRef,
	handlers,
	onChipDragStart,
	onPaintStart,
}: {
	day: Date;
	inMonth: boolean;
	events: CalendarEvent[];
	editable: boolean;
	selectable: boolean;
	showTime: boolean;
	maxVisible: number;
	painted: boolean;
	dropTarget: boolean;
	dtf: Dtf;
	movedRef: MovedRef;
	handlers: EventHandlers;
	onChipDragStart: (ev: CalendarEvent, e: ReactPointerEvent) => void;
	onPaintStart: (day: Date, e: ReactPointerEvent) => void;
}) {
	const visible = events.slice(0, maxVisible);
	const hidden = events.slice(maxVisible);
	return (
		<div
			data-day={iso(startOfDay(day))}
			onPointerDown={(e) => onPaintStart(day, e)}
			onKeyDown={
				selectable
					? keyActivate(() =>
							handlers.openCreate(startOfDay(day), startOfDay(day), true),
						)
					: undefined
			}
			className={cn(
				"group relative flex min-w-0 flex-col gap-0.5 overflow-hidden border-r border-border p-1 transition-colors last:border-r-0",
				selectable && "cursor-pointer hover:bg-accent/20",
				!inMonth && "bg-muted/25 text-muted-foreground",
				painted && "bg-primary/10",
				dropTarget && "bg-accent/40 ring-1 ring-inset ring-ring/40",
			)}
		>
			{selectable && (
				<PlusIcon className="pointer-events-none absolute left-1 top-1 h-3 w-3 opacity-0 transition-opacity group-hover:opacity-50" />
			)}
			<span
				className={cn(
					"inline-flex h-5 w-5 items-center justify-center self-end rounded-full text-xs",
					isToday(day) && "bg-primary font-semibold text-primary-foreground",
				)}
			>
				{day.getDate()}
			</span>
			<div className="flex min-h-0 flex-col gap-0.5 overflow-hidden">
				{visible.map((ev) => (
					<MonthChip
						key={ev.id}
						ev={ev}
						editable={editable}
						showTime={showTime && isSameDay(toDate(ev.start), day)}
						dtf={dtf}
						handlers={handlers}
						onDragStart={onChipDragStart}
					/>
				))}
			</div>
			{hidden.length > 0 && (
				<Popover>
					<PopoverTrigger asChild>
						<button
							type="button"
							onPointerDown={(e) => e.stopPropagation()}
							onClick={(e) => e.stopPropagation()}
							className="self-start rounded px-1 text-[10px] text-muted-foreground transition-colors hover:text-foreground"
						>
							+{hidden.length} more
						</button>
					</PopoverTrigger>
					<PopoverContent
						align="start"
						className="w-60 p-2"
						onClick={(e) => e.stopPropagation()}
					>
						<div className="mb-1.5 px-0.5 text-xs font-medium">
							{dtf({ weekday: "long", month: "short", day: "numeric" }).format(
								day,
							)}
						</div>
						<div className="flex flex-col gap-1">
							{hidden.map((ev) => (
								<MonthChip
									key={ev.id}
									ev={ev}
									editable={editable}
									showTime={showTime}
									dtf={dtf}
									handlers={handlers}
								/>
							))}
						</div>
					</PopoverContent>
				</Popover>
			)}
		</div>
	);
}

function MonthChip({
	ev,
	editable,
	showTime,
	dtf,
	handlers,
	onDragStart,
}: {
	ev: CalendarEvent;
	editable: boolean;
	showTime: boolean;
	dtf: Dtf;
	handlers: EventHandlers;
	onDragStart?: (ev: CalendarEvent, e: ReactPointerEvent) => void;
}) {
	const canEdit = editable && ev.editable !== false;
	const allDay = !!ev.allDay;
	return (
		<EventContextMenu ev={ev} editable={editable} handlers={handlers}>
			<div
				onPointerDown={(e) => {
					e.stopPropagation();
					if (canEdit && onDragStart) onDragStart(ev, e);
				}}
				onClick={(e) => {
					e.stopPropagation();
					handlers.openEvent(ev);
				}}
				onKeyDown={keyActivate(() => handlers.openEvent(ev))}
				style={chipColorStyle(ev, allDay ? 26 : 16)}
				className={cn(
					"group/ev relative flex w-full min-w-0 items-center gap-1 rounded-md border-l-2 border-l-primary px-1.5 py-0.5 text-left text-[11px] text-foreground transition hover:brightness-105 hover:ring-1 hover:ring-ring/40 dark:hover:brightness-125",
					allDay ? "bg-primary/20" : "bg-primary/10",
					canEdit && onDragStart
						? "cursor-grab active:cursor-grabbing"
						: "cursor-pointer",
				)}
			>
				{showTime && !allDay && (
					<span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
						{dtf({ hour: "2-digit", minute: "2-digit" }).format(
							toDate(ev.start),
						)}
					</span>
				)}
				<span className="truncate">{ev.title}</span>
				{canEdit && (
					<button
						type="button"
						aria-label="Delete event"
						onPointerDown={(e) => e.stopPropagation()}
						onClick={(e) => {
							e.stopPropagation();
							handlers.deleteEvent(ev);
						}}
						className="ml-auto hidden shrink-0 rounded p-0.5 hover:bg-background/50 group-hover/ev:block"
					>
						<XIcon className="h-3 w-3" />
					</button>
				)}
			</div>
		</EventContextMenu>
	);
}

// ── Time grid (week / day) ──────────────────────────────────────────

type TimeDragKind = "move" | "resize-start" | "resize-end";

interface TimeDrag {
	ev: CalendarEvent;
	kind: TimeDragKind;
	startX: number;
	startY: number;
	originStartMin: number;
	durationMin: number;
	day: Date;
}

interface PaintState {
	day: Date;
	top: number;
	anchorMin: number;
}

function TimeGridView({
	days,
	events,
	minTime,
	maxTime,
	slotDuration,
	hourHeight,
	showWeekendShade,
	showNowIndicator,
	showAllDay,
	editable,
	selectable,
	dtf,
	movedRef,
	handlers,
}: {
	days: Date[];
	events: CalendarEvent[];
	minTime: number;
	maxTime: number;
	slotDuration: number;
	hourHeight: number;
	showWeekendShade: boolean;
	showNowIndicator: boolean;
	showAllDay: boolean;
	editable: boolean;
	selectable: boolean;
	dtf: Dtf;
	movedRef: MovedRef;
	handlers: EventHandlers;
}) {
	const pxPerMinute = hourHeight / 60;
	const gridHeight = Math.max(60, maxTime - minTime) * pxPerMinute;
	const halfHourLines = hourHeight >= 48;
	const timeFmt = dtf({ hour: "2-digit", minute: "2-digit" });

	const lines = useMemo(() => {
		const solid: number[] = [];
		const dashed: number[] = [];
		for (let m = Math.ceil(minTime / 30) * 30; m < maxTime; m += 30) {
			if (m <= minTime) continue;
			if (m % 60 === 0) solid.push(m);
			else dashed.push(m);
		}
		return { solid, dashed };
	}, [minTime, maxTime]);

	const dragRef = useRef<TimeDrag | null>(null);
	const paintRef = useRef<PaintState | null>(null);
	const [preview, setPreview] = useState<CalendarEvent | null>(null);
	const [ghost, setGhost] = useState<{
		day: Date;
		startMin: number;
		endMin: number;
	} | null>(null);
	const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

	const snapRound = useCallback(
		(m: number) => Math.round(m / slotDuration) * slotDuration,
		[slotDuration],
	);
	const snapFloor = useCallback(
		(m: number) => Math.floor(m / slotDuration) * slotDuration,
		[slotDuration],
	);

	const beginEventDrag = useCallback(
		(
			ev: CalendarEvent,
			kind: TimeDragKind,
			e: ReactPointerEvent,
			day: Date,
		) => {
			movedRef.current = false;
			dragRef.current = {
				ev,
				kind,
				startX: e.clientX,
				startY: e.clientY,
				originStartMin: minutesOf(toDate(ev.start)),
				durationMin: eventDurationMinutes(ev),
				day,
			};
		},
		[movedRef],
	);

	const beginPaint = useCallback(
		(day: Date, e: ReactPointerEvent) => {
			if (!selectable || e.button !== 0 || dragRef.current) return;
			movedRef.current = false;
			const rect = e.currentTarget.getBoundingClientRect();
			const raw = minTime + (e.clientY - rect.top) / pxPerMinute;
			const anchor = Math.min(
				maxTime - slotDuration,
				Math.max(minTime, snapFloor(raw)),
			);
			paintRef.current = { day, top: rect.top, anchorMin: anchor };
			setGhost({ day, startMin: anchor, endMin: anchor + slotDuration });
			setCursor({ x: e.clientX, y: e.clientY });
		},
		[
			selectable,
			movedRef,
			minTime,
			maxTime,
			slotDuration,
			pxPerMinute,
			snapFloor,
		],
	);

	// Day column under the pointer — lets a move drag cross into other days
	// (Outlook-style) instead of staying locked to the origin column.
	const dayColAt = useCallback((x: number, y: number): Date | null => {
		const col = document
			.elementFromPoint(x, y)
			?.closest("[data-day-col]") as HTMLElement | null;
		return col?.dataset.dayCol ? toDate(col.dataset.dayCol) : null;
	}, []);

	const onPointerMove = useCallback(
		(e: ReactPointerEvent) => {
			const d = dragRef.current;
			const p = paintRef.current;
			if (!d && !p) return;
			setCursor({ x: e.clientX, y: e.clientY });
			if (d) {
				if (
					Math.hypot(e.clientX - d.startX, e.clientY - d.startY) >
					DRAG_THRESHOLD_PX
				)
					movedRef.current = true;
				// Sub-threshold jitter must not build a preview: snapping would
				// shift non-slot-aligned events and commit a phantom move on click.
				if (!movedRef.current) return;
				const deltaMin = (e.clientY - d.startY) / pxPerMinute;
				if (d.kind === "move") {
					const newStartMin = Math.max(
						minTime,
						Math.min(
							maxTime - d.durationMin,
							snapRound(d.originStartMin + deltaMin),
						),
					);
					const targetDay = dayColAt(e.clientX, e.clientY) ?? d.day;
					setPreview(shiftEvent(d.ev, atMinute(targetDay, newStartMin)));
				} else if (d.kind === "resize-start") {
					const endMin = d.originStartMin + d.durationMin;
					const newStartMin = Math.max(
						minTime,
						Math.min(
							endMin - slotDuration,
							snapRound(d.originStartMin + deltaMin),
						),
					);
					setPreview({
						...d.ev,
						start: iso(atMinute(d.day, newStartMin)),
						end: iso(atMinute(d.day, endMin)),
					});
				} else {
					const newDuration = Math.min(
						maxTime - d.originStartMin,
						Math.max(slotDuration, snapRound(d.durationMin + deltaMin)),
					);
					const start = toDate(d.ev.start);
					setPreview({
						...d.ev,
						end: iso(new Date(start.getTime() + newDuration * 60000)),
					});
				}
			} else if (p) {
				const raw = minTime + (e.clientY - p.top) / pxPerMinute;
				const cur = Math.max(minTime, Math.min(maxTime, snapRound(raw)));
				setGhost({
					day: p.day,
					startMin: Math.min(p.anchorMin, cur),
					endMin: Math.max(p.anchorMin + slotDuration, cur),
				});
			}
		},
		[
			movedRef,
			pxPerMinute,
			minTime,
			maxTime,
			slotDuration,
			snapRound,
			dayColAt,
		],
	);

	const onPointerUp = useCallback(() => {
		const d = dragRef.current;
		const p = paintRef.current;
		dragRef.current = null;
		paintRef.current = null;
		setCursor(null);
		if (d) {
			// Compare instants, not strings — re-encoding alone (ISO vs date-only
			// vs timezone-less input) must never count as a change.
			const changed =
				preview &&
				(toDate(preview.start).getTime() !== toDate(d.ev.start).getTime() ||
					eventEnd(preview).getTime() !== eventEnd(d.ev).getTime());
			if (movedRef.current && changed && preview) {
				handlers.moveOrResize(
					preview,
					d.kind === "move" ? "move" : "resize",
					d.ev,
				);
			}
			// Clear the suppression flag after the trailing click has fired.
			window.setTimeout(() => {
				movedRef.current = false;
			}, 0);
		}
		setPreview(null);
		if (p && ghost) {
			handlers.openCreate(
				atMinute(ghost.day, ghost.startMin),
				atMinute(ghost.day, ghost.endMin),
				false,
			);
		}
		setGhost(null);
	}, [preview, ghost, handlers, movedRef]);

	const cancelPointer = useCallback(() => {
		dragRef.current = null;
		paintRef.current = null;
		movedRef.current = false;
		setPreview(null);
		setGhost(null);
		setCursor(null);
	}, [movedRef]);

	// Preview participates in overlap layout with its live times.
	const columnData = useCallback(
		(day: Date) => {
			const timed = events
				.map((ev) => (preview && preview.id === ev.id ? preview : ev))
				.filter(
					(ev) =>
						!ev.allDay &&
						(!showAllDay || !isMultiDay(ev)) &&
						isSameDay(toDate(ev.start), day),
				);
			return { timed, layout: layoutOverlappingEvents(timed) };
		},
		[events, preview, showAllDay],
	);

	const dragLabel = useMemo(() => {
		if (preview)
			return `${timeFmt.format(toDate(preview.start))} – ${timeFmt.format(
				eventEnd(preview),
			)}`;
		if (ghost)
			return `${timeFmt.format(atMinute(ghost.day, ghost.startMin))} – ${timeFmt.format(
				atMinute(ghost.day, ghost.endMin),
			)}`;
		return null;
	}, [preview, ghost, timeFmt]);

	const now = new Date();
	const nowMin = minutesOf(now);
	const nowVisible = showNowIndicator && nowMin >= minTime && nowMin <= maxTime;
	const hasToday = days.some((d) => isToday(d));

	return (
		<div className="flex h-full flex-col">
			{/* scrollbar-gutter on the fixed rows mirrors the grid's scrollbar so
			    columns stay aligned on platforms with classic scrollbars. */}
			<div className="flex shrink-0 overflow-y-hidden border-b border-border [scrollbar-gutter:stable]">
				<div className="w-14 shrink-0 border-r border-border" />
				{days.map((day, i) => (
					<div
						key={day.toISOString()}
						className={cn(
							"flex flex-1 items-center justify-center gap-1.5 py-1.5 text-xs",
							i > 0 && "border-l border-border/60",
						)}
					>
						<span className="text-muted-foreground">
							{dtf({ weekday: "short" }).format(day)}
						</span>
						<span
							className={cn(
								"inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1 font-medium",
								isToday(day) && "bg-primary text-primary-foreground",
							)}
						>
							{day.getDate()}
						</span>
					</div>
				))}
			</div>

			{showAllDay && (
				<AllDayRow
					days={days}
					events={events}
					editable={editable}
					selectable={selectable}
					movedRef={movedRef}
					handlers={handlers}
				/>
			)}

			<div
				className="relative flex-1 select-none overflow-y-auto [scrollbar-gutter:stable]"
				onPointerMove={onPointerMove}
				onPointerUp={onPointerUp}
				onPointerLeave={cancelPointer}
			>
				<div className="flex" style={{ height: gridHeight }}>
					<div className="relative w-14 shrink-0 border-r border-border">
						{lines.solid.map((m) => (
							<span
								key={m}
								className="absolute right-1.5 -translate-y-1/2 text-[10px] tabular-nums text-muted-foreground"
								style={{ top: (m - minTime) * pxPerMinute }}
							>
								{formatHourLabel(m)}
							</span>
						))}
						{hasToday && nowVisible && (
							<span
								className="absolute right-0.5 z-30 -translate-y-1/2 rounded bg-card px-0.5 text-[9px] font-medium tabular-nums text-red-500"
								style={{ top: (nowMin - minTime) * pxPerMinute }}
							>
								{timeFmt.format(now)}
							</span>
						)}
					</div>
					{days.map((day, i) => {
						const { timed, layout } = columnData(day);
						const weekend = day.getDay() === 0 || day.getDay() === 6;
						return (
							<div
								key={day.toISOString()}
								data-day-col={toDateInput(day)}
								onPointerDown={(e) => beginPaint(day, e)}
								className={cn(
									"relative flex-1",
									i > 0 && "border-l border-border/60",
									showWeekendShade && weekend && "bg-muted/20",
								)}
							>
								{lines.solid.map((m) => (
									<div
										key={m}
										className="pointer-events-none absolute inset-x-0 border-t border-border/50"
										style={{ top: (m - minTime) * pxPerMinute }}
									/>
								))}
								{halfHourLines &&
									lines.dashed.map((m) => (
										<div
											key={m}
											className="pointer-events-none absolute inset-x-0 border-t border-dashed border-border/25"
											style={{ top: (m - minTime) * pxPerMinute }}
										/>
									))}
								{ghost && isSameDay(ghost.day, day) && (
									<div
										className="pointer-events-none absolute inset-x-1 z-10 rounded-md border border-dashed border-primary/60 bg-primary/10"
										style={{
											top: (ghost.startMin - minTime) * pxPerMinute,
											height: (ghost.endMin - ghost.startMin) * pxPerMinute,
										}}
									/>
								)}
								{nowVisible && isToday(day) && (
									<div
										className="pointer-events-none absolute inset-x-0 z-30 h-0.5 -translate-y-1/2 bg-red-500"
										style={{ top: (nowMin - minTime) * pxPerMinute }}
									>
										<span className="absolute -left-0.75 top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-red-500" />
									</div>
								)}
								{timed.map((ev) => (
									<TimeGridChip
										key={ev.id}
										ev={ev}
										layout={layout.get(ev.id)}
										minTime={minTime}
										pxPerMinute={pxPerMinute}
										editable={editable}
										timeFmt={timeFmt}
										handlers={handlers}
										onDragStart={(evd, kind, e) =>
											beginEventDrag(evd, kind, e, day)
										}
									/>
								))}
							</div>
						);
					})}
				</div>
				{cursor && dragLabel && (
					<div
						className="pointer-events-none fixed z-50 rounded-md border border-border bg-popover px-2 py-1 text-[10px] font-medium tabular-nums text-popover-foreground shadow-md"
						style={{ left: cursor.x + 12, top: cursor.y + 14 }}
					>
						{dragLabel}
					</div>
				)}
			</div>
		</div>
	);
}

function TimeGridChip({
	ev,
	layout,
	minTime,
	pxPerMinute,
	editable,
	timeFmt,
	handlers,
	onDragStart,
}: {
	ev: CalendarEvent;
	layout: TimedEventLayout | undefined;
	minTime: number;
	pxPerMinute: number;
	editable: boolean;
	timeFmt: Intl.DateTimeFormat;
	handlers: EventHandlers;
	onDragStart: (
		ev: CalendarEvent,
		kind: TimeDragKind,
		e: ReactPointerEvent,
	) => void;
}) {
	const canEdit = editable && ev.editable !== false;
	const start = toDate(ev.start);
	const height = Math.max(18, eventDurationMinutes(ev) * pxPerMinute - 2);
	const columns = layout?.columns ?? 1;
	const column = layout?.column ?? 0;
	const position: CSSProperties =
		columns > 1
			? {
					left: `calc(${(column / columns) * 100}% + 2px)`,
					width: `calc(${100 / columns}% - 4px)`,
				}
			: {};
	return (
		<EventContextMenu ev={ev} editable={editable} handlers={handlers}>
			<div
				onPointerDown={(e) => {
					if (e.button !== 0) return;
					e.stopPropagation();
					if (canEdit) onDragStart(ev, "move", e);
				}}
				onClick={(e) => {
					e.stopPropagation();
					handlers.openEvent(ev);
				}}
				onKeyDown={keyActivate(() => handlers.openEvent(ev))}
				style={{
					top: (minutesOf(start) - minTime) * pxPerMinute,
					height,
					...position,
					...chipColorStyle(ev, 16),
				}}
				className={cn(
					"group/ev absolute z-20 flex flex-col overflow-hidden rounded-md border-l-2 border-l-primary bg-primary/10 px-1.5 py-0.5 text-left text-xs text-foreground shadow-xs transition-shadow hover:z-30 hover:ring-1 hover:ring-ring/50",
					columns === 1 && "inset-x-1",
					canEdit ? "cursor-grab active:cursor-grabbing" : "cursor-pointer",
				)}
			>
				<div className="flex items-start justify-between gap-1">
					<span className="truncate text-[11px] font-medium leading-4">
						{ev.title}
					</span>
					{canEdit && (
						<button
							type="button"
							aria-label="Delete event"
							onPointerDown={(e) => e.stopPropagation()}
							onClick={(e) => {
								e.stopPropagation();
								handlers.deleteEvent(ev);
							}}
							className="hidden shrink-0 rounded p-0.5 hover:bg-background/50 group-hover/ev:block"
						>
							<XIcon className="h-3 w-3" />
						</button>
					)}
				</div>
				{height >= 34 && (
					<span className="truncate text-[10px] tabular-nums text-muted-foreground">
						{timeFmt.format(start)} – {timeFmt.format(eventEnd(ev))}
					</span>
				)}
				{canEdit && (
					<>
						<div
							onPointerDown={(e) => {
								if (e.button !== 0) return;
								e.stopPropagation();
								onDragStart(ev, "resize-start", e);
							}}
							className="group/handle absolute inset-x-0 top-0 flex h-1.5 cursor-ns-resize items-center justify-center"
						>
							<span className="h-0.5 w-6 rounded-full bg-foreground/30 opacity-0 transition-opacity group-hover/ev:opacity-100" />
						</div>
						<div
							onPointerDown={(e) => {
								if (e.button !== 0) return;
								e.stopPropagation();
								onDragStart(ev, "resize-end", e);
							}}
							className="absolute inset-x-0 bottom-0 flex h-1.5 cursor-ns-resize items-center justify-center"
						>
							<span className="h-0.5 w-6 rounded-full bg-foreground/30 opacity-0 transition-opacity group-hover/ev:opacity-100" />
						</div>
					</>
				)}
			</div>
		</EventContextMenu>
	);
}

// ── All-day lane ────────────────────────────────────────────────────

function AllDayRow({
	days,
	events,
	editable,
	selectable,
	movedRef,
	handlers,
}: {
	days: Date[];
	events: CalendarEvent[];
	editable: boolean;
	selectable: boolean;
	movedRef: MovedRef;
	handlers: EventHandlers;
}) {
	const laneRef = useRef<HTMLDivElement>(null);
	const dragRef = useRef<{ ev: CalendarEvent; x: number; y: number } | null>(
		null,
	);
	const [dropCol, setDropCol] = useState<number | null>(null);

	const colAt = useCallback(
		(x: number): number | null => {
			const rect = laneRef.current?.getBoundingClientRect();
			if (!rect || rect.width === 0) return null;
			const idx = Math.floor(((x - rect.left) / rect.width) * days.length);
			return Math.max(0, Math.min(days.length - 1, idx));
		},
		[days.length],
	);

	const onLanePointerMove = useCallback(
		(e: ReactPointerEvent) => {
			const d = dragRef.current;
			if (!d) return;
			if (Math.hypot(e.clientX - d.x, e.clientY - d.y) > DRAG_THRESHOLD_PX) {
				movedRef.current = true;
				setDropCol(colAt(e.clientX));
			}
		},
		[movedRef, colAt],
	);

	const onLanePointerUp = useCallback(
		(e: ReactPointerEvent) => {
			const d = dragRef.current;
			dragRef.current = null;
			setDropCol(null);
			if (!d || !movedRef.current) return;
			// Clear the suppression flag after the trailing click has fired.
			window.setTimeout(() => {
				movedRef.current = false;
			}, 0);
			const col = colAt(e.clientX);
			if (col == null) return;
			const targetDay = days[col];
			const orig = toDate(d.ev.start);
			if (isSameDay(orig, targetDay)) return;
			const newStart = d.ev.allDay
				? startOfDay(targetDay)
				: set(targetDay, {
						hours: orig.getHours(),
						minutes: orig.getMinutes(),
					});
			handlers.moveOrResize(shiftEvent(d.ev, newStart), "move", d.ev);
		},
		[movedRef, colAt, days, handlers],
	);

	const cancelLanePointer = useCallback(() => {
		dragRef.current = null;
		movedRef.current = false;
		setDropCol(null);
	}, [movedRef]);

	const { items, overflow } = useMemo(() => {
		const candidates: {
			ev: CalendarEvent;
			startCol: number;
			endCol: number;
		}[] = [];
		for (const ev of events) {
			if (!ev.allDay && !isMultiDay(ev)) continue;
			const s = startOfDay(toDate(ev.start)).getTime();
			const e = startOfDay(eventEnd(ev)).getTime();
			let startCol = -1;
			let endCol = -1;
			days.forEach((d, i) => {
				const t = d.getTime();
				if (t >= s && t <= e) {
					if (startCol < 0) startCol = i;
					endCol = i;
				}
			});
			if (startCol >= 0) candidates.push({ ev, startCol, endCol });
		}
		candidates.sort(
			(a, b) =>
				a.startCol - b.startCol ||
				b.endCol - b.startCol - (a.endCol - a.startCol),
		);
		const laneEnds: number[] = [];
		const placed: {
			ev: CalendarEvent;
			startCol: number;
			endCol: number;
			lane: number;
		}[] = [];
		const overflowByCol = new Map<number, CalendarEvent[]>();
		for (const c of candidates) {
			let lane = laneEnds.findIndex((end) => end < c.startCol);
			if (lane === -1) {
				lane = laneEnds.length;
				laneEnds.push(c.endCol);
			} else {
				laneEnds[lane] = c.endCol;
			}
			if (lane < ALL_DAY_MAX_LANES) {
				placed.push({ ...c, lane });
			} else {
				for (let col = c.startCol; col <= c.endCol; col++) {
					const list = overflowByCol.get(col) ?? [];
					list.push(c.ev);
					overflowByCol.set(col, list);
				}
			}
		}
		return { items: placed, overflow: overflowByCol };
	}, [events, days]);

	const cols = `repeat(${days.length}, minmax(0,1fr))`;

	return (
		<div
			className="flex shrink-0 select-none overflow-y-hidden border-b border-border [scrollbar-gutter:stable]"
			onPointerMove={editable ? onLanePointerMove : undefined}
			onPointerUp={editable ? onLanePointerUp : undefined}
			onPointerLeave={editable ? cancelLanePointer : undefined}
		>
			<div className="flex w-14 shrink-0 items-start justify-end border-r border-border pr-1.5 pt-1 text-[9px] text-muted-foreground">
				all-day
			</div>
			<div ref={laneRef} className="relative min-h-7 flex-1">
				<div
					className="absolute inset-0 grid"
					style={{ gridTemplateColumns: cols }}
				>
					{days.map((day, i) => (
						<div
							key={day.toISOString()}
							onClick={
								selectable
									? () => {
											if (movedRef.current) return;
											handlers.openCreate(
												startOfDay(day),
												startOfDay(day),
												true,
											);
										}
									: undefined
							}
							onKeyDown={
								selectable
									? keyActivate(() =>
											handlers.openCreate(
												startOfDay(day),
												startOfDay(day),
												true,
											),
										)
									: undefined
							}
							className={cn(
								i > 0 && "border-l border-border/60",
								selectable &&
									"cursor-pointer transition-colors hover:bg-accent/20",
								dropCol === i && "bg-accent/40",
							)}
						/>
					))}
				</div>
				<div
					className="pointer-events-none relative grid px-0.5 py-1"
					style={{ gridTemplateColumns: cols, gridAutoRows: "18px", rowGap: 2 }}
				>
					{items.map(({ ev, startCol, endCol, lane }) => {
						const canEdit = editable && ev.editable !== false;
						return (
							<EventContextMenu
								key={ev.id}
								ev={ev}
								editable={editable}
								handlers={handlers}
							>
								<button
									type="button"
									onPointerDown={(e) => {
										if (e.button !== 0) return;
										e.stopPropagation();
										if (canEdit) {
											movedRef.current = false;
											dragRef.current = { ev, x: e.clientX, y: e.clientY };
										}
									}}
									onClick={(e) => {
										e.stopPropagation();
										handlers.openEvent(ev);
									}}
									style={{
										gridColumn: `${startCol + 1} / ${endCol + 2}`,
										gridRow: lane + 1,
										...chipColorStyle(ev, 26),
									}}
									className={cn(
										"pointer-events-auto mx-0.5 flex min-w-0 items-center overflow-hidden rounded-md border-l-2 border-l-primary bg-primary/20 px-1.5 text-left text-[10px] font-medium text-foreground hover:ring-1 hover:ring-ring/40",
										canEdit && "cursor-grab active:cursor-grabbing",
									)}
								>
									<span className="truncate">{ev.title}</span>
								</button>
							</EventContextMenu>
						);
					})}
					{[...overflow.entries()].map(([col, hiddenEvents]) => (
						<Popover key={col}>
							<PopoverTrigger asChild>
								<button
									type="button"
									onPointerDown={(e) => e.stopPropagation()}
									onClick={(e) => e.stopPropagation()}
									className="pointer-events-auto self-start rounded px-1.5 text-left text-[9px] leading-4.5 text-muted-foreground transition-colors hover:text-foreground"
									style={{
										gridColumn: `${col + 1} / ${col + 2}`,
										gridRow: ALL_DAY_MAX_LANES + 1,
									}}
								>
									+{hiddenEvents.length}
								</button>
							</PopoverTrigger>
							<PopoverContent align="start" className="w-56 p-2">
								<div className="flex flex-col gap-1">
									{hiddenEvents.map((ev) => (
										<button
											key={ev.id}
											type="button"
											onClick={() => handlers.openEvent(ev)}
											style={chipColorStyle(ev, 26)}
											className="flex min-w-0 items-center overflow-hidden rounded-md border-l-2 border-l-primary bg-primary/20 px-1.5 py-0.5 text-left text-[10px] font-medium text-foreground hover:ring-1 hover:ring-ring/40"
										>
											<span className="truncate">{ev.title}</span>
										</button>
									))}
								</div>
							</PopoverContent>
						</Popover>
					))}
				</div>
			</div>
		</div>
	);
}

// ── Agenda view ─────────────────────────────────────────────────────

function AgendaView({
	focusDate,
	events,
	editable,
	dtf,
	handlers,
}: {
	focusDate: Date;
	events: CalendarEvent[];
	editable: boolean;
	dtf: Dtf;
	handlers: EventHandlers;
}) {
	const timeFmt = dtf({ hour: "2-digit", minute: "2-digit" });
	const days = useMemo(
		() =>
			Array.from({ length: AGENDA_DAYS }, (_, i) =>
				addDays(startOfDay(focusDate), i),
			),
		[focusDate],
	);
	const byDay = useCallback(
		(day: Date) =>
			events.filter((ev) => eventTouchesDay(ev, day)).sort(sortDayEvents),
		[events],
	);
	const visibleDays = useMemo(
		() => days.filter((day) => byDay(day).length > 0 || isToday(day)),
		[days, byDay],
	);

	if (visibleDays.length === 0) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-2 py-10">
				<span className="text-sm text-muted-foreground/70">
					No upcoming events
				</span>
				{editable && (
					<Button
						variant="outline"
						size="sm"
						className="h-7"
						onClick={() =>
							handlers.openCreate(
								startOfDay(focusDate),
								startOfDay(focusDate),
								true,
							)
						}
					>
						<PlusIcon className="mr-1 h-3.5 w-3.5" /> Add event
					</Button>
				)}
			</div>
		);
	}

	return (
		<div className="divide-y divide-border">
			{visibleDays.map((day) => {
				const dayEvents = byDay(day);
				return (
					<div key={day.toISOString()} className="flex gap-3 px-3 py-2">
						<div className="w-16 shrink-0 pt-1 text-center">
							<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
								{dtf({ weekday: "short" }).format(day)}
							</div>
							<div
								className={cn(
									"mx-auto mt-0.5 inline-flex h-7 w-7 items-center justify-center rounded-full text-sm",
									isToday(day) &&
										"bg-primary font-semibold text-primary-foreground",
								)}
							>
								{day.getDate()}
							</div>
						</div>
						<div className="flex min-w-0 flex-1 flex-col gap-0.5 py-0.5">
							{dayEvents.length === 0 && (
								<span className="px-2 py-1.5 text-xs text-muted-foreground/60">
									No events
								</span>
							)}
							{dayEvents.map((ev) => {
								const canEdit = editable && ev.editable !== false;
								return (
									<EventContextMenu
										key={ev.id}
										ev={ev}
										editable={editable}
										handlers={handlers}
									>
										<div
											onClick={() => handlers.openEvent(ev)}
											onKeyDown={keyActivate(() => handlers.openEvent(ev))}
											className="group/ev flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-accent/30"
										>
											<span
												className="h-2.5 w-2.5 shrink-0 rounded-full bg-primary"
												style={
													ev.color ? { backgroundColor: ev.color } : undefined
												}
											/>
											<span className="w-14 shrink-0 text-xs tabular-nums text-muted-foreground">
												{ev.allDay
													? "all-day"
													: timeFmt.format(toDate(ev.start))}
											</span>
											<span className="truncate text-sm font-medium">
												{ev.title}
											</span>
											{ev.location && (
												<span className="min-w-0 truncate text-xs text-muted-foreground">
													{ev.location}
												</span>
											)}
											{canEdit && (
												<button
													type="button"
													onClick={(e) => {
														e.stopPropagation();
														handlers.deleteEvent(ev);
													}}
													aria-label="Delete event"
													className="ml-auto hidden shrink-0 rounded p-1 text-muted-foreground hover:bg-background group-hover/ev:block"
												>
													<XIcon className="h-3.5 w-3.5" />
												</button>
											)}
										</div>
									</EventContextMenu>
								);
							})}
							{editable && (
								<Button
									variant="ghost"
									size="sm"
									onClick={() => handlers.openCreate(day, day, true)}
									className="h-6 self-start px-1.5 text-xs text-muted-foreground hover:text-foreground"
								>
									<PlusIcon className="mr-1 h-3 w-3" /> Add
								</Button>
							)}
						</div>
					</div>
				);
			})}
		</div>
	);
}
