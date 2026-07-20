"use client";

import type { GanttTask } from "@flow-like/flow-like-ui";
import {
	ganttRange,
	taskBarDays,
} from "@flow-like/flow-like-ui/components/a2ui/planning-utils";
import { getVoiceVisualizer } from "@flow-like/flow-like-ui/components/voice/visualizers";
import {
	Braces,
	ChevronLeft,
	ChevronRight,
	Columns2,
	FileDiff,
	FoldVertical,
	FormInput,
	GanttChart,
	Mic,
	Pilcrow,
	Plus,
	Rows3,
	Square,
	WrapText,
} from "lucide-react";
import {
	type ReactNode,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { ProductDemoFrame, cn } from "./ProductDemoFrame";

type PlanningSurface = "calendar" | "gantt";
type CalendarView = "month" | "week" | "day" | "agenda";
type GanttView = "day" | "week" | "month" | "quarter";

type CalendarEvent = {
	id: string;
	title: string;
	day: number;
	time?: string;
	allDay?: boolean;
	color: string;
};

const INITIAL_EVENTS: CalendarEvent[] = [
	{
		id: "e1",
		title: "Kickoff",
		day: 7,
		time: "10:00",
		color: "#3b82f6",
	},
	{
		id: "e2",
		title: "Design review",
		day: 9,
		time: "14:00",
		color: "#8b5cf6",
	},
	{
		id: "e3",
		title: "1:1 sync",
		day: 9,
		time: "14:30",
		color: "#10b981",
	},
	{
		id: "e4",
		title: "Release",
		day: 15,
		allDay: true,
		color: "#f59e0b",
	},
];

const MONTH_CELLS = [
	29, 30, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
	21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 1, 2,
];

const MONTH_LABELS = ["June 2026", "July 2026", "August 2026"];

function SmallButton({
	children,
	onClick,
	label,
	variant = "outline",
	className,
}: Readonly<{
	children: ReactNode;
	onClick?: () => void;
	label?: string;
	variant?: "outline" | "ghost";
	className?: string;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-label={label}
			className={cn(
				"inline-flex h-7 items-center justify-center rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				variant === "outline"
					? "border border-border bg-background px-2 hover:bg-accent"
					: "px-1.5 hover:bg-accent",
				className,
			)}
		>
			{children}
		</button>
	);
}

function PlanningViewSwitch<T extends string>({
	value,
	options,
	onChange,
	label,
}: Readonly<{
	value: T;
	options: readonly T[];
	onChange: (value: T) => void;
	label: string;
}>) {
	return (
		<fieldset
			className="flex items-center gap-0.5 rounded-md border border-border p-0.5"
			aria-label={label}
		>
			{options.map((option) => (
				<button
					key={option}
					type="button"
					onClick={() => onChange(option)}
					className={cn(
						"rounded px-2 py-1 text-xs capitalize transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
						value === option
							? "bg-primary text-primary-foreground"
							: "text-muted-foreground hover:bg-accent",
					)}
				>
					{option}
				</button>
			))}
		</fieldset>
	);
}

function CalendarMonth({ events }: Readonly<{ events: CalendarEvent[] }>) {
	const weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
	return (
		<div className="flex h-full select-none flex-col overflow-y-auto">
			<div className="sticky top-0 z-20 grid shrink-0 grid-cols-7 border-b border-border bg-card text-xs font-medium text-muted-foreground">
				{weekdays.map((day) => (
					<div key={day} className="px-2 py-1.5 text-center">
						{day}
					</div>
				))}
			</div>
			<div className="grid flex-1 grid-cols-7 grid-rows-5">
				{MONTH_CELLS.map((day, index) => {
					const inMonth = index >= 2 && index <= 32;
					const dayEvents = inMonth
						? events.filter((event) => event.day === day)
						: [];
					return (
						<div
							key={`${index}-${day}`}
							className={cn(
								"group relative flex min-h-16 min-w-0 flex-col gap-0.5 overflow-hidden border-b border-r border-border p-1 transition-colors hover:bg-accent/20 sm:min-h-20",
								!inMonth && "bg-muted/25 text-muted-foreground",
							)}
						>
							<Plus className="pointer-events-none absolute left-1 top-1 h-3 w-3 opacity-0 transition-opacity group-hover:opacity-50" />
							<span className="inline-flex h-5 w-5 items-center justify-center self-end rounded-full text-xs">
								{day}
							</span>
							<div className="flex min-h-0 flex-col gap-0.5 overflow-hidden">
								{dayEvents.slice(0, 2).map((event) => (
									<button
										type="button"
										key={event.id}
										className="relative flex w-full min-w-0 cursor-pointer items-center gap-1 rounded-md border-l-2 px-1.5 py-0.5 text-left text-[11px] text-foreground transition hover:brightness-105 hover:ring-1 hover:ring-ring/40"
										style={{
											borderLeftColor: event.color,
											backgroundColor: `${event.color}1f`,
										}}
									>
										{!event.allDay && (
											<span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
												{event.time}
											</span>
										)}
										<span className="truncate">{event.title}</span>
									</button>
								))}
							</div>
						</div>
					);
				})}
			</div>
		</div>
	);
}

function CalendarAgenda({ events }: Readonly<{ events: CalendarEvent[] }>) {
	return (
		<div className="h-full overflow-y-auto p-2">
			{[7, 9, 15].map((day) => (
				<div key={day} className="grid grid-cols-[4.5rem_1fr] border-b py-3">
					<div className="px-2 text-xs text-muted-foreground">
						<div className="font-semibold text-foreground">Jul {day}</div>
						2026
					</div>
					<div className="space-y-1">
						{events
							.filter((event) => event.day === day)
							.map((event) => (
								<button
									key={event.id}
									type="button"
									className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-accent"
								>
									<span
										className="h-2 w-2 rounded-full"
										style={{ backgroundColor: event.color }}
									/>
									<span className="w-12 tabular-nums text-muted-foreground">
										{event.allDay ? "All day" : event.time}
									</span>
									<span className="font-medium">{event.title}</span>
								</button>
							))}
					</div>
				</div>
			))}
		</div>
	);
}

function CalendarSurface() {
	const [view, setView] = useState<CalendarView>("month");
	const [month, setMonth] = useState(1);
	const [events, setEvents] = useState(INITIAL_EVENTS);
	const addEvent = () => {
		if (events.some((event) => event.id === "sample")) return;
		setEvents((current) => [
			...current,
			{
				id: "sample",
				title: "New event",
				day: 21,
				time: "09:00",
				color: "#06b6d4",
			},
		]);
	};
	return (
		<div className="flex h-[430px] flex-col overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-sm">
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<div className="flex min-w-0 items-center gap-1">
					<SmallButton
						variant="ghost"
						className="w-7 shrink-0 p-0"
						label="Previous month"
						onClick={() => setMonth((value) => Math.max(0, value - 1))}
					>
						<ChevronLeft className="h-4 w-4" />
					</SmallButton>
					<SmallButton
						variant="ghost"
						className="w-7 shrink-0 p-0"
						label="Next month"
						onClick={() => setMonth((value) => Math.min(2, value + 1))}
					>
						<ChevronRight className="h-4 w-4" />
					</SmallButton>
					<SmallButton
						className="ml-1 hidden shrink-0 sm:inline-flex"
						onClick={() => setMonth(1)}
					>
						Today
					</SmallButton>
					<h3 className="ml-2 truncate text-sm font-semibold">
						{MONTH_LABELS[month]}
					</h3>
				</div>
				<div className="hidden shrink-0 items-center gap-2 sm:flex">
					<SmallButton onClick={addEvent}>
						<Plus className="mr-1 h-3.5 w-3.5" /> Event
					</SmallButton>
					<PlanningViewSwitch
						value={view}
						onChange={setView}
						options={["month", "week", "day", "agenda"]}
						label="Calendar view"
					/>
				</div>
			</header>
			<div className="relative flex-1 overflow-hidden">
				{view === "month" ? (
					<CalendarMonth events={events} />
				) : (
					<CalendarAgenda events={events} />
				)}
			</div>
		</div>
	);
}

const GANTT_TASKS: GanttTask[] = [
	{
		id: "t1",
		name: "Research",
		start: "2026-06-29",
		end: "2026-07-17",
		progress: 100,
		color: "#3b82f6",
	},
	{
		id: "t2",
		name: "Design",
		start: "2026-07-20",
		end: "2026-07-31",
		progress: 60,
		color: "#8b5cf6",
		dependencies: ["t1"],
	},
	{
		id: "t3",
		name: "Build",
		start: "2026-08-03",
		end: "2026-08-14",
		progress: 10,
		color: "#10b981",
		dependencies: ["t2"],
	},
	{
		id: "t4",
		name: "Launch",
		start: "2026-08-17",
		end: "2026-08-17",
		color: "#f59e0b",
		milestone: true,
		dependencies: ["t3"],
	},
];

type EffectiveGanttView = GanttView | "compact";

const GANTT_TARGET_DAYS: Record<EffectiveGanttView, number> = {
	day: 21,
	week: 84,
	month: 210,
	quarter: 455,
	compact: 180,
};
const GANTT_FALLBACK_DAY_WIDTH: Record<EffectiveGanttView, number> = {
	day: 40,
	week: 20,
	month: 6,
	quarter: 3,
	compact: 4,
};
const GANTT_ROW_HEIGHT = 40;
const GANTT_HEADER_HEIGHT = 44;
const GANTT_LIST_WIDTH = 192;

function addDays(date: Date, days: number) {
	const next = new Date(date);
	next.setDate(next.getDate() + days);
	return next;
}

function daysInRange(start: Date, count: number) {
	return Array.from({ length: count }, (_, index) => addDays(start, index));
}

function GanttTimelineHeader({
	days,
	dayWidth,
}: Readonly<{ days: Date[]; dayWidth: number }>) {
	const months = useMemo(() => {
		const result: Array<{
			key: string;
			left: number;
			width: number;
			label: string;
		}> = [];
		let startIndex = 0;
		for (let index = 1; index <= days.length; index += 1) {
			const previous = days[index - 1];
			const current = days[index];
			if (
				index === days.length ||
				(current && current.getMonth() !== previous.getMonth())
			) {
				result.push({
					key: `${previous.getFullYear()}-${previous.getMonth()}`,
					left: startIndex * dayWidth,
					width: (index - startIndex) * dayWidth,
					label: previous.toLocaleDateString(undefined, {
						month: "long",
						year: "numeric",
					}),
				});
				startIndex = index;
			}
		}
		return result;
	}, [days, dayWidth]);

	return (
		<div
			className="sticky top-0 z-40 border-b border-border bg-card"
			style={{ height: GANTT_HEADER_HEIGHT }}
		>
			<div className="relative h-5 border-b border-border/60">
				{months.map((month) => (
					<div
						key={month.key}
						className="absolute inset-y-0 flex items-center truncate border-r border-border/60 px-1.5 text-[10px] font-medium text-muted-foreground"
						style={{ left: month.left, width: month.width }}
					>
						{month.width >= 72 ? month.label : month.label.slice(0, 3)}
					</div>
				))}
			</div>
			<div className="relative h-6">
				{days.map((day, index) => {
					const show = dayWidth >= 24 || day.getDay() === 1;
					if (!show) return null;
					return (
						<span
							key={day.toISOString()}
							className="absolute top-1/2 -translate-y-1/2 text-[9px] text-muted-foreground"
							style={{
								left: index * dayWidth + (dayWidth >= 24 ? 0 : 2),
								width: dayWidth >= 24 ? dayWidth : undefined,
								textAlign: dayWidth >= 24 ? "center" : undefined,
							}}
						>
							{day.getDate()}
						</span>
					);
				})}
			</div>
		</div>
	);
}

function GanttDependencies({
	tasks,
	range,
	dayWidth,
	hovered,
	markerId,
}: Readonly<{
	tasks: GanttTask[];
	range: ReturnType<typeof ganttRange>;
	dayWidth: number;
	hovered: string | null;
	markerId: string;
}>) {
	const taskMap = new Map(tasks.map((task) => [task.id, task]));
	const rowIndex = new Map(tasks.map((task, index) => [task.id, index]));
	const edges = tasks.flatMap((target) =>
		(target.dependencies ?? []).flatMap((sourceId) => {
			const source = taskMap.get(sourceId);
			if (!source) return [];
			return [{ source, target }];
		}),
	);

	return (
		<svg
			className="pointer-events-none absolute inset-0 z-20 size-full overflow-visible"
			aria-hidden="true"
		>
			{edges.map(({ source, target }) => {
				const sourceGeometry = taskBarDays(source, range);
				const targetGeometry = taskBarDays(target, range);
				const x1 =
					(source.milestone
						? sourceGeometry.offsetDays
						: sourceGeometry.offsetDays + sourceGeometry.spanDays) * dayWidth;
				const y1 =
					(rowIndex.get(source.id) ?? 0) * GANTT_ROW_HEIGHT +
					GANTT_ROW_HEIGHT / 2;
				const x2 = targetGeometry.offsetDays * dayWidth;
				const y2 =
					(rowIndex.get(target.id) ?? 0) * GANTT_ROW_HEIGHT +
					GANTT_ROW_HEIGHT / 2;
				const direction = x2 >= x1 ? 1 : -1;
				const handleReach = Math.min(20, Math.abs(x2 - x1) / 2);
				const active = hovered === source.id || hovered === target.id;
				return (
					<path
						key={`${source.id}-${target.id}`}
						d={`M ${x1} ${y1} C ${x1 + direction * handleReach} ${y1}, ${x2 - direction * handleReach} ${y2}, ${x2} ${y2}`}
						fill="none"
						className={
							active ? "stroke-primary/75" : "stroke-muted-foreground/45"
						}
						strokeWidth={active ? 2 : 1.5}
						markerEnd={`url(#${markerId})`}
					/>
				);
			})}
			<defs>
				<marker
					id={markerId}
					markerWidth="6"
					markerHeight="6"
					refX="5"
					refY="3"
					orient="auto"
				>
					<path d="M0,0 L6,3 L0,6 Z" className="fill-muted-foreground/55" />
				</marker>
			</defs>
		</svg>
	);
}

function GanttSurface() {
	const containerRef = useRef<HTMLDivElement>(null);
	const markerId = useId().replaceAll(":", "");
	const [containerWidth, setContainerWidth] = useState(0);
	const [view, setView] = useState<GanttView>("week");
	const [tasks, setTasks] = useState(GANTT_TASKS);
	const [selected, setSelected] = useState<string | null>(null);
	const [hovered, setHovered] = useState<string | null>(null);

	useEffect(() => {
		const container = containerRef.current;
		if (!container || typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(([entry]) => {
			if (entry) setContainerWidth(entry.contentRect.width);
		});
		observer.observe(container);
		return () => observer.disconnect();
	}, []);

	const isNarrow = containerWidth === 0 || containerWidth < 720;
	const effectiveView: EffectiveGanttView = isNarrow ? "compact" : view;
	const listVisible = !isNarrow;
	const taskRange = useMemo(() => ganttRange(tasks), [tasks]);
	const range = useMemo(() => {
		const totalDays = Math.max(
			taskRange.totalDays,
			GANTT_TARGET_DAYS[effectiveView],
		);
		return {
			start: taskRange.start,
			end: addDays(taskRange.start, totalDays - 1),
			totalDays,
		};
	}, [effectiveView, taskRange]);
	const availableWidth = Math.max(
		0,
		containerWidth - (listVisible ? GANTT_LIST_WIDTH : 0),
	);
	const dayWidth =
		availableWidth > 0
			? Math.min(
					120,
					Math.max(1.5, availableWidth / GANTT_TARGET_DAYS[effectiveView]),
				)
			: GANTT_FALLBACK_DAY_WIDTH[effectiveView];
	const totalWidth = range.totalDays * dayWidth;
	const days = useMemo(
		() => daysInRange(range.start, range.totalDays),
		[range.start, range.totalDays],
	);
	const addTask = () => {
		if (tasks.some((task) => task.id === "t5")) return;
		setTasks((current) => [
			...current,
			{
				id: "t5",
				name: "Follow-up",
				start: "2026-08-18",
				end: "2026-08-28",
				progress: 0,
				color: "#06b6d4",
				dependencies: ["t4"],
			},
		]);
	};

	return (
		<div
			ref={containerRef}
			className="flex h-[390px] min-w-0 max-w-full flex-col overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-sm"
		>
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<div className="flex min-w-0 items-center gap-2">
					<h3 className="flex min-w-0 items-center gap-1.5 text-sm font-semibold">
						<GanttChart className="size-4 shrink-0 text-muted-foreground" />
						<span className="truncate">Timeline</span>
					</h3>
					<span className="shrink-0 rounded-md bg-secondary px-2 py-0.5 text-[10px]">
						{tasks.length} tasks
					</span>
				</div>
				<div className="flex shrink-0 items-center gap-2">
					<SmallButton onClick={addTask} label="Add task">
						<Plus className="mr-1 size-3.5" /> Task
					</SmallButton>
					{!isNarrow ? (
						<PlanningViewSwitch
							value={view}
							onChange={setView}
							options={["day", "week", "month", "quarter"]}
							label="Gantt zoom"
						/>
					) : null}
				</div>
			</header>
			<div
				aria-busy={containerWidth === 0}
				className={cn(
					"flex min-h-0 min-w-0 flex-1 overflow-hidden",
					containerWidth === 0 && "hidden",
				)}
			>
				{listVisible ? (
					<aside
						className="flex shrink-0 flex-col border-r border-border"
						style={{ width: GANTT_LIST_WIDTH }}
					>
						<div
							className="flex shrink-0 items-center border-b border-border px-2 text-xs font-medium text-muted-foreground"
							style={{ height: GANTT_HEADER_HEIGHT }}
						>
							Task
						</div>
						{tasks.map((task, index) => (
							<button
								type="button"
								key={task.id}
								onClick={() => setSelected(task.id)}
								onMouseEnter={() => setHovered(task.id)}
								onMouseLeave={() => setHovered(null)}
								className={cn(
									"flex items-center gap-2 border-b border-border/30 px-2 text-left text-xs transition-colors hover:bg-accent/30",
									index % 2 === 1 && "bg-muted/20",
									selected === task.id && "bg-accent/40",
								)}
								style={{ height: GANTT_ROW_HEIGHT }}
							>
								<span
									className="size-2 shrink-0 rounded-full"
									style={{ backgroundColor: task.color }}
								/>
								<span className="truncate">{task.name}</span>
							</button>
						))}
					</aside>
				) : null}
				<div className="min-w-0 flex-1 overflow-x-auto overflow-y-hidden overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
					<div style={{ width: Math.max(totalWidth, 240) }}>
						<GanttTimelineHeader days={days} dayWidth={dayWidth} />
						<div
							className="relative select-none"
							style={{ height: Math.max(1, tasks.length) * GANTT_ROW_HEIGHT }}
						>
							{days.map((day, index) =>
								day.getDay() === 1 ? (
									<div
										key={day.toISOString()}
										className="pointer-events-none absolute inset-y-0 border-r border-border/40"
										style={{ left: index * dayWidth }}
									/>
								) : null,
							)}
							{tasks.map((task, index) => (
								<div
									key={`row-${task.id}`}
									className={cn(
										"absolute inset-x-0 border-b border-border/30",
										index % 2 === 1 && "bg-muted/20",
									)}
									style={{
										top: index * GANTT_ROW_HEIGHT,
										height: GANTT_ROW_HEIGHT,
									}}
								/>
							))}
							<GanttDependencies
								tasks={tasks}
								range={range}
								dayWidth={dayWidth}
								hovered={hovered}
								markerId={`gantt-arrow-${markerId}`}
							/>
							{tasks.map((task, index) => {
								const geometry = taskBarDays(task, range);
								const left = geometry.offsetDays * dayWidth;
								const width = Math.max(dayWidth, geometry.spanDays * dayWidth);
								const top = index * GANTT_ROW_HEIGHT;
								if (task.milestone) {
									return (
										<button
											type="button"
											key={task.id}
											onClick={() => setSelected(task.id)}
											onMouseEnter={() => setHovered(task.id)}
											onMouseLeave={() => setHovered(null)}
											className="group absolute z-30 flex items-center gap-1.5 outline-none"
											style={{ left: left - 6, top, height: GANTT_ROW_HEIGHT }}
										>
											<span
												className="size-3 rotate-45 rounded-[2px] transition-transform group-hover:scale-110"
												style={{ backgroundColor: task.color }}
											/>
											<span className="whitespace-nowrap text-[11px] text-muted-foreground">
												{task.name}
											</span>
										</button>
									);
								}
								return (
									<button
										type="button"
										key={task.id}
										onClick={() => setSelected(task.id)}
										onMouseEnter={() => setHovered(task.id)}
										onMouseLeave={() => setHovered(null)}
										className={cn(
											"absolute z-30 flex items-center overflow-hidden rounded-md border px-1.5 text-left outline-none transition hover:brightness-110",
											selected === task.id && "ring-2 ring-ring",
										)}
										style={{
											left,
											top: top + 5,
											width,
											height: GANTT_ROW_HEIGHT - 10,
											borderColor: task.color,
											backgroundColor: `${task.color}26`,
										}}
									>
										{task.progress !== undefined ? (
											<span
												className="absolute inset-y-0 left-0 rounded-md"
												style={{
													width: `${task.progress}%`,
													backgroundColor: `${task.color}55`,
												}}
											/>
										) : null}
										<span className="relative z-10 truncate text-[11px] font-medium">
											{task.name}
										</span>
									</button>
								);
							})}
						</div>
					</div>
				</div>
			</div>
		</div>
	);
}

export function PlanningDemo() {
	const [surface, setSurface] = useState<PlanningSurface>("calendar");
	return (
		<ProductDemoFrame source="packages/ui/components/a2ui/display/Calendar.tsx · GanttChart.tsx">
			<div className="mb-2 flex justify-end">
				<PlanningViewSwitch
					value={surface}
					onChange={setSurface}
					options={["calendar", "gantt"]}
					label="Planning component"
				/>
			</div>
			{surface === "calendar" ? <CalendarSurface /> : <GanttSurface />}
		</ProductDemoFrame>
	);
}

type DiffMode = "split" | "unified" | "inline";
type DiffRow = {
	id: string;
	oldNo?: number;
	newNo?: number;
	oldText?: string;
	newText?: string;
	type: "context" | "change" | "insert" | "delete";
};

const DIFF_ROWS: DiffRow[] = [
	{
		id: "1",
		oldNo: 1,
		newNo: 1,
		oldText: "interface ReleasePlan {",
		newText: "interface ReleasePlan {",
		type: "context",
	},
	{
		id: "2",
		oldNo: 2,
		newNo: 2,
		oldText: "  owner: string;",
		newText: "  owner: string;",
		type: "context",
	},
	{
		id: "3",
		oldNo: 3,
		newNo: 3,
		oldText: '  status: "draft" | "ready";',
		newText: '  status: "draft" | "ready" | "live";',
		type: "change",
	},
	{ id: "4", newNo: 4, newText: "  reviewers: string[];", type: "insert" },
	{ id: "5", oldNo: 4, newNo: 5, oldText: "}", newText: "}", type: "context" },
	{ id: "6", oldNo: 5, newNo: 6, oldText: "", newText: "", type: "context" },
	{
		id: "7",
		oldNo: 6,
		newNo: 7,
		oldText: "const plan = createPlan({",
		newText: "const plan = createPlan({",
		type: "context",
	},
	{
		id: "8",
		oldNo: 7,
		newNo: 8,
		oldText: '  status: "draft",',
		newText: '  status: "ready",',
		type: "change",
	},
	{
		id: "9",
		newNo: 9,
		newText: '  reviewers: ["Design", "Security"],',
		type: "insert",
	},
	{
		id: "10",
		oldNo: 8,
		newNo: 10,
		oldText: "});",
		newText: "});",
		type: "context",
	},
];

function DiffIconToggle({
	active,
	label,
	onClick,
	children,
}: Readonly<{
	active: boolean;
	label: string;
	onClick: () => void;
	children: ReactNode;
}>) {
	return (
		<button
			type="button"
			title={label}
			aria-label={label}
			aria-pressed={active}
			onClick={onClick}
			className={cn(
				"flex h-6 w-7 items-center justify-center rounded-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				active
					? "bg-primary text-primary-foreground"
					: "text-muted-foreground hover:text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function DiffCode({ text, wrap }: Readonly<{ text?: string; wrap: boolean }>) {
	return (
		<code
			className={cn(
				"px-3 py-px min-w-0",
				wrap ? "whitespace-pre-wrap break-words" : "whitespace-pre",
			)}
		>
			{text === "" ? "\u00a0" : (text ?? "")}
		</code>
	);
}

export function DiffDemo() {
	const [mode, setMode] = useState<DiffMode>("split");
	const [wordWrap, setWordWrap] = useState(false);
	const [collapse, setCollapse] = useState(false);
	const rows = collapse
		? [...DIFF_ROWS.slice(0, 5), ...DIFF_ROWS.slice(-3)]
		: DIFF_ROWS;
	const additions = DIFF_ROWS.filter(
		(row) => row.type === "insert" || row.type === "change",
	).length;
	const deletions = DIFF_ROWS.filter(
		(row) => row.type === "delete" || row.type === "change",
	).length;

	return (
		<ProductDemoFrame source="packages/ui/components/ui/diff-viewer/DiffViewer.tsx">
			<div className="flex w-full flex-col overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm">
				<div className="flex flex-wrap items-center justify-between gap-2 border-b bg-muted/40 px-3 py-2">
					<div className="flex min-w-0 items-center gap-2 text-sm">
						<FileDiff className="h-4 w-4 shrink-0 text-muted-foreground" />
						<span className="truncate font-medium">0.1.5</span>
						<span className="text-muted-foreground">→</span>
						<span className="truncate font-medium">0.1.6</span>
					</div>
					<div className="flex items-center gap-1.5">
						<div className="mr-1 flex items-center gap-2 font-mono text-xs">
							<span className="text-green-600 dark:text-green-400">
								+{additions}
							</span>
							<span className="text-red-600 dark:text-red-400">
								−{deletions}
							</span>
						</div>
						<div className="flex items-center rounded-md border bg-background p-0.5">
							<DiffIconToggle
								active={mode === "split"}
								label="Split"
								onClick={() => setMode("split")}
							>
								<Columns2 className="h-3.5 w-3.5" />
							</DiffIconToggle>
							<DiffIconToggle
								active={mode === "unified"}
								label="Unified"
								onClick={() => setMode("unified")}
							>
								<Rows3 className="h-3.5 w-3.5" />
							</DiffIconToggle>
							<DiffIconToggle
								active={mode === "inline"}
								label="Inline"
								onClick={() => setMode("inline")}
							>
								<Pilcrow className="h-3.5 w-3.5" />
							</DiffIconToggle>
						</div>
						<button
							type="button"
							className={cn(
								"flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent",
								wordWrap && "text-primary",
							)}
							title="Toggle word wrap"
							aria-label="Toggle word wrap"
							aria-pressed={wordWrap}
							onClick={() => setWordWrap((value) => !value)}
						>
							<WrapText className="h-3.5 w-3.5" />
						</button>
						<button
							type="button"
							className={cn(
								"flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent",
								collapse && "text-primary",
							)}
							title="Collapse unchanged"
							aria-label="Collapse unchanged"
							aria-pressed={collapse}
							onClick={() => setCollapse((value) => !value)}
						>
							<FoldVertical className="h-3.5 w-3.5" />
						</button>
					</div>
				</div>
				<div className="max-h-[420px] min-h-0 flex-1 overflow-auto bg-card">
					{mode === "inline" ? (
						<div className="overflow-auto px-4 py-3 font-mono text-[13px] leading-[1.7] whitespace-pre-wrap break-words">
							<span>{'status: "'}</span>
							<span className="rounded-[2px] bg-red-500/20 text-red-700 line-through dark:text-red-300">
								draft
							</span>
							<span className="rounded-[2px] bg-green-500/20 text-green-700 dark:text-green-300">
								ready
							</span>
							<span>{'";\nreviewers: ["Design", "Security"];'}</span>
						</div>
					) : mode === "split" ? (
						<div className="grid min-w-[680px] grid-cols-[2.75rem_max-content_2.75rem_max-content] font-mono text-[13px] leading-[1.7]">
							{rows.map((row) => {
								const leftBg =
									row.oldText == null
										? "bg-muted/20"
										: row.type === "change" || row.type === "delete"
											? "bg-red-500/10"
											: "";
								const rightBg =
									row.newText == null
										? "bg-muted/20"
										: row.type === "change" || row.type === "insert"
											? "bg-green-500/10"
											: "";
								return (
									<div key={row.id} className="contents">
										<div
											className={cn(
												"px-2 py-px text-right tabular-nums text-[11px] leading-[1.7] text-muted-foreground/60 select-none",
												leftBg,
											)}
										>
											{row.oldNo ?? ""}
										</div>
										<div className={leftBg}>
											<DiffCode text={row.oldText} wrap={wordWrap} />
										</div>
										<div
											className={cn(
												"border-l border-border/60 px-2 py-px text-right tabular-nums text-[11px] leading-[1.7] text-muted-foreground/60 select-none",
												rightBg,
											)}
										>
											{row.newNo ?? ""}
										</div>
										<div className={rightBg}>
											<DiffCode text={row.newText} wrap={wordWrap} />
										</div>
									</div>
								);
							})}
						</div>
					) : (
						<div className="grid min-w-[560px] grid-cols-[2.5rem_2.5rem_1.25rem_max-content] font-mono text-[13px] leading-[1.7]">
							{rows
								.flatMap((row) => {
									if (row.type === "context")
										return [
											{
												key: `${row.id}-c`,
												oldNo: row.oldNo,
												newNo: row.newNo,
												marker: " ",
												text: row.newText,
												bg: "",
											},
										];
									const values = [];
									if (row.oldText != null)
										values.push({
											key: `${row.id}-d`,
											oldNo: row.oldNo,
											newNo: undefined,
											marker: "−",
											text: row.oldText,
											bg: "bg-red-500/10",
										});
									if (row.newText != null)
										values.push({
											key: `${row.id}-a`,
											oldNo: undefined,
											newNo: row.newNo,
											marker: "+",
											text: row.newText,
											bg: "bg-green-500/10",
										});
									return values;
								})
								.map((row) => (
									<div key={row.key} className="contents">
										<div
											className={cn(
												"px-2 py-px text-right text-[11px] text-muted-foreground/60",
												row.bg,
											)}
										>
											{row.oldNo ?? ""}
										</div>
										<div
											className={cn(
												"px-2 py-px text-right text-[11px] text-muted-foreground/60",
												row.bg,
											)}
										>
											{row.newNo ?? ""}
										</div>
										<div
											className={cn(
												"px-1 text-center text-[11px] font-bold",
												row.bg,
												row.marker === "+"
													? "text-green-600"
													: row.marker === "−"
														? "text-red-600"
														: "text-transparent",
											)}
										>
											{row.marker}
										</div>
										<div className={row.bg}>
											<DiffCode text={row.text} wrap={wordWrap} />
										</div>
									</div>
								))}
						</div>
					)}
				</div>
			</div>
		</ProductDemoFrame>
	);
}

type SchemaValue = {
	title: string;
	priority: "low" | "normal" | "high";
	owner: { sub: string; displayName: string };
	notify: boolean;
};

const INITIAL_SCHEMA_VALUE: SchemaValue = {
	title: "Quarterly review",
	priority: "high",
	owner: { sub: "usr_01JFLX", displayName: "Felix Mohr" },
	notify: true,
};

function SchemaModeButton({
	active,
	onClick,
	children,
	label,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	children: ReactNode;
	label: string;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			title={label}
			className={cn(
				"inline-flex h-7 items-center justify-center gap-1 rounded-md border px-2 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				active
					? "border-transparent bg-secondary text-secondary-foreground"
					: "border-border bg-background hover:bg-accent",
			)}
		>
			{children}
		</button>
	);
}

function SchemaTextField({
	label,
	value,
	onChange,
	description,
}: Readonly<{
	label: string;
	value: string;
	onChange: (value: string) => void;
	description?: string;
}>) {
	return (
		<div className="space-y-1">
			<label className="text-xs font-medium">
				{label}
				<input
					value={value}
					onChange={(event) => onChange(event.target.value)}
					className="mt-1 flex h-8 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none transition-colors focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
				/>
			</label>
			{description && (
				<p className="text-xs text-muted-foreground">{description}</p>
			)}
		</div>
	);
}

export function SchemaInputsDemo() {
	const [useJsonMode, setUseJsonMode] = useState(false);
	const [value, setValue] = useState(INITIAL_SCHEMA_VALUE);
	const [jsonDraft, setJsonDraft] = useState(() =>
		JSON.stringify(INITIAL_SCHEMA_VALUE, null, 2),
	);
	const [jsonError, setJsonError] = useState<string | null>(null);
	const [jsonFocused, setJsonFocused] = useState(false);

	const showForm = () => {
		try {
			const parsed = JSON.parse(jsonDraft) as SchemaValue;
			setValue(parsed);
			setJsonError(null);
		} catch {
			// Preserve the last valid form value, matching StructVariable.
		}
		setUseJsonMode(false);
	};
	const showJson = () => {
		setJsonDraft(JSON.stringify(value, null, 2));
		setUseJsonMode(true);
	};

	return (
		<ProductDemoFrame source="packages/ui/components/flow/variables/struct-variable.tsx">
			<div className="grid w-full items-center gap-2 rounded-lg border bg-card p-4 text-card-foreground shadow-sm sm:p-5">
				<div className="flex items-center justify-end gap-2">
					<SchemaModeButton
						active={!useJsonMode}
						onClick={showForm}
						label="Edit using generated form"
					>
						<FormInput className="h-3 w-3" /> Form
					</SchemaModeButton>
					<SchemaModeButton
						active={useJsonMode}
						onClick={showJson}
						label="Edit raw JSON"
					>
						<Braces className="h-3 w-3" /> JSON
					</SchemaModeButton>
				</div>

				{useJsonMode ? (
					<div className="space-y-1">
						<div
							className={cn(
								"relative w-full rounded-md border border-input bg-transparent transition-all duration-200 dark:bg-input/30",
								jsonFocused && "border-ring ring-3 ring-ring/50",
								jsonError && "border-destructive",
							)}
						>
							<textarea
								value={jsonDraft}
								onChange={(event) => {
									const next = event.target.value;
									setJsonDraft(next);
									try {
										setValue(JSON.parse(next) as SchemaValue);
										setJsonError(null);
									} catch {
										setJsonError("Invalid JSON");
									}
								}}
								onFocus={() => setJsonFocused(true)}
								onBlur={() => setJsonFocused(false)}
								rows={12}
								spellCheck={false}
								className="w-full resize-none bg-transparent px-3 py-2 font-mono text-sm leading-[22px] outline-none placeholder:text-muted-foreground"
							/>
						</div>
						{jsonError && (
							<p className="text-xs text-destructive">{jsonError}</p>
						)}
					</div>
				) : (
					<div className="space-y-3 rounded-md border p-3">
						<p className="mb-2 text-xs text-muted-foreground">
							A typed review request generated from its JSON Schema.
						</p>
						<SchemaTextField
							label="title *"
							value={value.title}
							onChange={(title) =>
								setValue((current) => ({ ...current, title }))
							}
						/>
						<div className="space-y-1">
							<label className="text-xs font-medium">
								priority *
								<select
									value={value.priority}
									onChange={(event) =>
										setValue((current) => ({
											...current,
											priority: event.target.value as SchemaValue["priority"],
										}))
									}
									className="mt-1 flex h-8 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
								>
									<option value="low">low</option>
									<option value="normal">normal</option>
									<option value="high">high</option>
								</select>
							</label>
						</div>
						<div className="space-y-2">
							<div>
								<div className="text-xs font-medium">owner *</div>
								<p className="text-xs text-muted-foreground">
									Project user assigned to this review.
								</p>
							</div>
							<div className="space-y-3 rounded-md border border-border/70 p-3">
								<SchemaTextField
									label="sub *"
									value={value.owner.sub}
									onChange={(sub) =>
										setValue((current) => ({
											...current,
											owner: { ...current.owner, sub },
										}))
									}
									description="Project user ID"
								/>
								<SchemaTextField
									label="displayName"
									value={value.owner.displayName}
									onChange={(displayName) =>
										setValue((current) => ({
											...current,
											owner: { ...current.owner, displayName },
										}))
									}
								/>
							</div>
						</div>
						<label className="flex cursor-pointer items-center space-x-2 py-1 text-xs font-medium">
							<input
								type="checkbox"
								checked={value.notify}
								onChange={(event) =>
									setValue((current) => ({
										...current,
										notify: event.target.checked,
									}))
								}
								className="h-4 w-4 rounded border border-primary accent-primary"
							/>
							<span>notify</span>
							<span className="ml-2 text-muted-foreground">
								Notify the owner when the review changes.
							</span>
						</label>
					</div>
				)}
			</div>
		</ProductDemoFrame>
	);
}

type ProfileVariant = "avatar" | "chip" | "row" | "detailed" | "card";

const PROFILE_FIXTURE = {
	userId: "usr_01JFLX9Q4E",
	label: "Felix Mohr",
	subtitle: "@felix",
	email: "felix@example.com",
	description: "Building local-first workflow infrastructure at Flow-Like.",
	initials: "FM",
};

function ProfileAvatar({
	size = "md",
}: Readonly<{ size?: "sm" | "md" | "lg" | "xl" }>) {
	const sizes = {
		sm: "h-6 w-6 text-[10px]",
		md: "h-8 w-8 text-xs",
		lg: "h-10 w-10 text-sm",
		xl: "h-14 w-14 text-base",
	};
	return (
		<span
			className={cn(
				"flex shrink-0 items-center justify-center rounded-full bg-primary/12 font-semibold text-primary ring-1 ring-primary/15",
				sizes[size],
			)}
			aria-hidden="true"
		>
			{PROFILE_FIXTURE.initials}
		</span>
	);
}

function ProfileHoverCard() {
	return (
		<div className="absolute left-0 top-[calc(100%+0.5rem)] z-30 w-80 max-w-[calc(100vw-3rem)] overflow-hidden rounded-md border bg-popover text-popover-foreground shadow-md">
			<div className="border-b bg-muted/30 p-4">
				<div className="flex min-w-0 items-start gap-3">
					<ProfileAvatar size="lg" />
					<div className="min-w-0 flex-1">
						<div className="truncate font-semibold">
							{PROFILE_FIXTURE.label}
						</div>
						<div className="truncate text-xs text-muted-foreground">
							{PROFILE_FIXTURE.subtitle}
						</div>
						<span className="mt-2 inline-flex text-xs font-medium text-primary">
							View profile ↗
						</span>
					</div>
				</div>
			</div>
			<div className="grid gap-3 p-4 text-sm">
				<div className="grid min-w-0 grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-3">
					<span className="text-muted-foreground">Email</span>
					<span className="truncate text-right">{PROFILE_FIXTURE.email}</span>
				</div>
				<div className="grid min-w-0 grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-3">
					<span className="text-muted-foreground">User ID</span>
					<code className="truncate rounded bg-muted px-1.5 py-0.5 text-right font-mono text-xs">
						{PROFILE_FIXTURE.userId}
					</code>
				</div>
				<p className="rounded-md bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
					{PROFILE_FIXTURE.description}
				</p>
			</div>
		</div>
	);
}

function ProfileContent({ variant }: Readonly<{ variant: ProfileVariant }>) {
	if (variant === "avatar") return <ProfileAvatar />;
	if (variant === "chip") {
		return (
			<span className="inline-flex max-w-full items-center gap-1.5 rounded-full border bg-background px-1.5 py-1 text-xs text-foreground shadow-sm">
				<ProfileAvatar size="sm" />
				<span className="min-w-0 truncate">{PROFILE_FIXTURE.label}</span>
			</span>
		);
	}
	if (variant === "card") {
		return (
			<div className="w-full max-w-sm rounded-lg border bg-card p-4 text-card-foreground shadow-sm">
				<div className="flex min-w-0 items-start gap-4">
					<ProfileAvatar size="xl" />
					<div className="min-w-0 flex-1">
						<div className="truncate text-base font-semibold">
							{PROFILE_FIXTURE.label}
						</div>
						<div className="truncate text-sm text-muted-foreground">
							{PROFILE_FIXTURE.subtitle}
						</div>
					</div>
				</div>
				<p className="mt-4 line-clamp-3 text-sm leading-relaxed text-muted-foreground">
					{PROFILE_FIXTURE.description}
				</p>
				<div className="mt-4 grid min-w-0 gap-2 border-t pt-3 text-xs">
					<div className="flex min-w-0 items-center gap-2 text-muted-foreground">
						<span aria-hidden="true">@</span>
						<span className="truncate">{PROFILE_FIXTURE.email}</span>
					</div>
					<div className="flex min-w-0 items-center gap-2 text-muted-foreground">
						<span aria-hidden="true">#</span>
						<code className="min-w-0 truncate font-mono">
							{PROFILE_FIXTURE.userId}
						</code>
					</div>
					<span className="inline-flex min-w-0 items-center gap-1 font-medium text-primary">
						View profile ↗
					</span>
				</div>
			</div>
		);
	}
	const detailed = variant === "detailed";
	return (
		<div
			className={cn(
				"flex min-w-0 max-w-full items-center gap-3 rounded-lg",
				detailed ? "border bg-card p-3 shadow-sm" : "p-1",
				"text-foreground",
			)}
		>
			<ProfileAvatar size={detailed ? "lg" : "md"} />
			<div className="min-w-0 flex-1">
				<div className="truncate text-sm font-medium">
					{PROFILE_FIXTURE.label}
				</div>
				<div
					className={cn(
						"text-xs text-muted-foreground",
						detailed ? "line-clamp-2" : "truncate",
					)}
				>
					{detailed ? PROFILE_FIXTURE.description : PROFILE_FIXTURE.subtitle}
				</div>
			</div>
		</div>
	);
}

export function UserProfileDemo() {
	const [variant, setVariant] = useState<ProfileVariant>("detailed");
	const [hoverOpen, setHoverOpen] = useState(false);
	const hasHover = variant !== "card";
	return (
		<ProductDemoFrame source="packages/ui/components/a2ui/display/UserProfile.tsx">
			<div className="mb-2 flex items-center justify-end gap-2 text-xs text-muted-foreground">
				<label htmlFor="profile-variant">Variant</label>
				<select
					id="profile-variant"
					value={variant}
					onChange={(event) => {
						setVariant(event.target.value as ProfileVariant);
						setHoverOpen(false);
					}}
					className="h-8 rounded-md border border-input bg-background px-2 text-xs text-foreground"
				>
					<option value="avatar">avatar</option>
					<option value="chip">chip</option>
					<option value="row">row</option>
					<option value="detailed">detailed</option>
					<option value="card">card</option>
				</select>
			</div>
			<div className="flex min-h-56 items-center justify-center rounded-lg border bg-card p-6">
				<div className="relative w-full max-w-sm">
					{hasHover ? (
						<button
							type="button"
							className="w-full rounded-lg text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							onClick={() => setHoverOpen((open) => !open)}
							onMouseEnter={() => setHoverOpen(true)}
							onMouseLeave={() => setHoverOpen(false)}
							aria-expanded={hoverOpen}
							aria-label={`${PROFILE_FIXTURE.label} profile details`}
						>
							<ProfileContent variant={variant} />
						</button>
					) : (
						<ProfileContent variant={variant} />
					)}
					{hoverOpen && hasHover && <ProfileHoverCard />}
				</div>
			</div>
		</ProductDemoFrame>
	);
}

const FACE_SCENE = `
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 540">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#dff6ff"/><stop offset="1" stop-color="#ede9fe"/></linearGradient>
    <linearGradient id="shirt" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#0ea5e9"/><stop offset="1" stop-color="#4f46e5"/></linearGradient>
    <filter id="shadow"><feDropShadow dx="0" dy="12" stdDeviation="18" flood-opacity=".18"/></filter>
  </defs>
  <rect width="960" height="540" fill="url(#bg)"/>
  <circle cx="120" cy="82" r="120" fill="#fff" opacity=".38"/>
  <circle cx="870" cy="460" r="180" fill="#fff" opacity=".32"/>
  <g opacity=".42" stroke="#7dd3fc"><path d="M0 120h960M0 240h960M0 360h960M0 480h960"/><path d="M160 0v540M320 0v540M480 0v540M640 0v540M800 0v540"/></g>
  <g filter="url(#shadow)">
    <rect x="135" y="105" width="690" height="345" rx="34" fill="#fff" opacity=".83"/>
    <rect x="170" y="145" width="250" height="265" rx="24" fill="#e0f2fe"/>
    <rect x="455" y="145" width="330" height="95" rx="22" fill="#f5f3ff"/>
    <rect x="455" y="265" width="330" height="145" rx="22" fill="#f0fdfa"/>
  </g>
  <g transform="translate(208 156)">
    <path d="M25 237c8-66 49-98 101-98s95 34 102 98" fill="url(#shirt)"/>
    <ellipse cx="126" cy="91" rx="68" ry="76" fill="#f3c6a5"/>
    <path d="M61 88c0-54 25-83 67-83 45 0 70 32 66 92-14-14-23-31-26-50-26 30-58 43-103 40z" fill="#334155"/>
    <circle cx="101" cy="95" r="5" fill="#334155"/><circle cx="151" cy="95" r="5" fill="#334155"/>
    <path d="M107 128c12 10 27 10 39 0" fill="none" stroke="#a85555" stroke-width="4" stroke-linecap="round"/>
  </g>
  <g fill="#64748b"><rect x="495" y="178" width="210" height="12" rx="6"/><rect x="495" y="204" width="145" height="8" rx="4" opacity=".55"/></g>
  <g transform="translate(495 302)"><rect width="250" height="15" rx="7.5" fill="#99f6e4"/><rect y="36" width="195" height="12" rx="6" fill="#a5b4fc"/><rect y="69" width="220" height="12" rx="6" fill="#bae6fd"/></g>
</svg>`;

const FACE_SCENE_SRC = `data:image/svg+xml;charset=UTF-8,${encodeURIComponent(FACE_SCENE)}`;

const FACE_BOXES = [
	{
		id: "face-1",
		x: 0.272,
		y: 0.286,
		width: 0.145,
		height: 0.286,
		label: "face",
		confidence: 0.97,
		color: "#22c55e",
	},
];

export function FaceVisionDemo() {
	const [hovered, setHovered] = useState<string | null>(null);
	const [selected, setSelected] = useState<string | null>(null);
	return (
		<ProductDemoFrame source="packages/ui/components/a2ui/display/BoundingBoxOverlay.tsx">
			<div
				className="relative aspect-video overflow-hidden rounded-lg border bg-muted shadow-sm"
				data-card-action-stop
			>
				<img
					src={FACE_SCENE_SRC}
					alt="Illustrated office portrait with a detected face"
					className="h-full w-full object-contain"
				/>
				{FACE_BOXES.map((box) => {
					const active = hovered === box.id || selected === box.id;
					return (
						<button
							type="button"
							key={box.id}
							className="absolute appearance-none border bg-transparent p-0 text-left transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
							aria-label={`Bounding box: ${box.label}, ${Math.round(box.confidence * 100)} percent confidence`}
							aria-pressed={selected === box.id}
							style={{
								left: `${box.x * 100}%`,
								top: `${box.y * 100}%`,
								width: `${box.width * 100}%`,
								height: `${box.height * 100}%`,
								borderColor: box.color,
								borderWidth: active ? 3 : 2,
								backgroundColor: `${box.color}${active ? "30" : "15"}`,
							}}
							onClick={() =>
								setSelected((value) => (value === box.id ? null : box.id))
							}
							onMouseEnter={() => setHovered(box.id)}
							onMouseLeave={() => setHovered(null)}
						>
							<span
								className="absolute -top-6 left-0 whitespace-nowrap px-1.5 py-0.5 text-white"
								style={{
									backgroundColor: box.color,
									fontSize: 12,
									lineHeight: 1.2,
								}}
							>
								{box.label}
								<span className="ml-1 opacity-80">
									{Math.round(box.confidence * 100)}%
								</span>
							</span>
						</button>
					);
				})}
			</div>
		</ProductDemoFrame>
	);
}

type VoiceVariant =
	| "conservative"
	| "waveform"
	| "orb"
	| "vortex"
	| "shader"
	| "aurora"
	| "pulse";

const VOICE_COLOR = "#8b5cf6";
const RECORDING_COLOR = "#ef4444";

function VoiceVisualizer({
	variant,
	recording,
	hover,
}: Readonly<{
	variant: VoiceVariant;
	recording: boolean;
	hover: boolean;
}>) {
	if (typeof window !== "undefined") {
		const ProductVisualizer = getVoiceVisualizer(variant);
		return (
			<ProductVisualizer
				analyser={null}
				state={recording ? "recording" : "idle"}
				size="md"
				color={VOICE_COLOR}
				recordingColor={RECORDING_COLOR}
				hover={hover}
			/>
		);
	}

	// Client-only islands use the shipped visualizer above. This static fallback
	// keeps the copied VoiceInput surface safe if it is ever pre-rendered.
	const color = recording ? RECORDING_COLOR : VOICE_COLOR;
	if (variant === "conservative") {
		return (
			<div className="relative flex h-14 w-14 items-center justify-center">
				{(recording || hover) && (
					<span
						className="absolute inset-0 animate-ping rounded-full opacity-25"
						style={{ backgroundColor: color }}
					/>
				)}
				<span
					className={cn(
						"relative flex h-14 w-14 items-center justify-center rounded-full shadow-md transition-transform duration-200",
						hover && !recording && "scale-110",
					)}
					style={{ backgroundColor: color, boxShadow: `0 0 22px ${color}66` }}
				>
					{recording ? (
						<Square className="h-5 w-5 fill-white text-white" />
					) : (
						<Mic className="h-6 w-6 text-white" />
					)}
				</span>
			</div>
		);
	}
	if (variant === "waveform") {
		return (
			<svg
				viewBox="0 0 320 80"
				className="h-20 w-full max-w-80 rounded-lg"
				role="img"
				aria-label="Waveform visualizer"
			>
				<defs>
					<linearGradient id="voice-wave" x1="0" x2="1">
						<stop stopColor={color} stopOpacity=".85" />
						<stop offset=".5" stopColor={recording ? "#fda4af" : "#c4b5fd"} />
						<stop offset="1" stopColor={color} stopOpacity=".85" />
					</linearGradient>
				</defs>
				<path
					d={
						recording
							? "M0 40 C18 8 31 73 49 35 S82 7 102 43 S136 74 158 34 S195 8 218 45 S252 70 274 32 S304 13 320 40"
							: "M0 40 C35 34 55 47 82 39 S130 34 160 41 S207 47 239 38 S290 35 320 40"
					}
					fill="none"
					stroke="url(#voice-wave)"
					strokeWidth="3"
					strokeLinecap="round"
					className="transition-all duration-500"
				/>
				<path
					d={
						recording
							? "M0 40 C18 72 31 7 49 45 S82 73 102 37 S136 6 158 46 S195 72 218 35 S252 10 274 48 S304 67 320 40"
							: "M0 40 C35 46 55 33 82 41 S130 46 160 39 S207 33 239 42 S290 45 320 40"
					}
					fill="none"
					stroke="url(#voice-wave)"
					strokeWidth="2"
					strokeOpacity=".35"
				/>
			</svg>
		);
	}
	if (variant === "pulse") {
		return (
			<div className="relative h-52 w-52">
				{[0, 1, 2, 3].map((ring) => (
					<span
						key={ring}
						className={cn(
							"absolute rounded-full border",
							recording && "animate-ping",
						)}
						style={{
							inset: 24 + ring * 17,
							borderColor: `${color}${90 - ring * 15}`,
							animationDelay: `${ring * 130}ms`,
						}}
					/>
				))}
				<span
					className={cn(
						"absolute inset-[42%] rounded-full shadow-lg",
						recording && "animate-pulse",
					)}
					style={{ backgroundColor: color, boxShadow: `0 0 32px ${color}88` }}
				/>
			</div>
		);
	}
	if (variant === "aurora") {
		return (
			<div className="relative h-52 w-52 overflow-hidden rounded-full bg-slate-950/5 dark:bg-slate-950/40">
				{[22, 38, 54, 70].map((top, index) => (
					<span
						key={top}
						className={cn(
							"absolute left-[14%] h-7 w-[72%] rounded-[50%] blur-md",
							recording && "animate-pulse",
						)}
						style={{
							top: `${top}%`,
							background: `linear-gradient(90deg, transparent, ${index % 2 ? "#22d3ee" : color}, transparent)`,
							transform: `rotate(${index % 2 ? 7 : -7}deg)`,
						}}
					/>
				))}
			</div>
		);
	}
	const background =
		variant === "vortex"
			? `conic-gradient(from 30deg, ${color}, #22d3ee, #4338ca, ${color})`
			: variant === "shader"
				? `radial-gradient(circle at 35% 30%, #fff 0, ${color} 18%, #22d3ee 46%, #312e81 75%, transparent 76%)`
				: `radial-gradient(circle at 35% 30%, #fff 0, #c4b5fd 10%, ${color} 42%, #4c1d95 75%)`;
	return (
		<div className="relative flex h-52 w-52 items-center justify-center">
			<span
				className={cn(
					"absolute inset-8 rounded-full blur-2xl opacity-35",
					recording && "animate-pulse",
				)}
				style={{ backgroundColor: color }}
			/>
			<span
				className={cn(
					"relative h-24 w-24 rounded-[46%_54%_51%_49%] shadow-2xl transition-transform duration-300",
					recording ? "scale-125 animate-pulse" : hover && "scale-110",
				)}
				style={{ background, boxShadow: `0 0 42px ${color}66` }}
			/>
		</div>
	);
}

export function VoiceStudioDemo() {
	const [variant, setVariant] = useState<VoiceVariant>("waveform");
	const [recording, setRecording] = useState(false);
	const [hover, setHover] = useState(false);
	const seconds = recording ? 18 : 0;
	return (
		<ProductDemoFrame source="packages/ui/components/a2ui/interactive/VoiceInput.tsx · components/voice/visualizers">
			<div className="mb-2 flex items-center justify-end gap-2 text-xs text-muted-foreground">
				<label htmlFor="voice-variant">Visualizer</label>
				<select
					id="voice-variant"
					value={variant}
					onChange={(event) => setVariant(event.target.value as VoiceVariant)}
					className="h-8 rounded-md border border-input bg-background px-2 text-xs text-foreground"
				>
					<option value="conservative">Conservative</option>
					<option value="waveform">Waveform</option>
					<option value="orb">Orb</option>
					<option value="vortex">Vortex</option>
					<option value="shader">Shader</option>
					<option value="aurora">Aurora</option>
					<option value="pulse">Pulse</option>
				</select>
			</div>
			<div className="space-y-2">
				<div className="text-sm font-medium">Record</div>
				<div
					className={cn(
						"relative overflow-hidden rounded-xl border transition-all duration-300",
						recording
							? "border-primary/40 bg-linear-to-b from-primary/5 to-transparent"
							: "border-border bg-background hover:border-primary/30",
					)}
				>
					<div className="flex min-h-40 flex-col items-center justify-center p-6">
						<button
							type="button"
							className="group flex select-none flex-col items-center gap-3 rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							onMouseEnter={() => setHover(true)}
							onMouseLeave={() => setHover(false)}
							onClick={() => setRecording((value) => !value)}
							aria-label={
								recording ? "Stop recording preview" : "Start recording preview"
							}
							aria-pressed={recording}
						>
							<VoiceVisualizer
								variant={variant}
								recording={recording}
								hover={hover}
							/>
						</button>
						<p className="mt-4 text-sm text-muted-foreground">
							{recording
								? `0:${seconds.toString().padStart(2, "0")}`
								: "Tap to start recording"}
						</p>
						{recording && (
							<div className="mt-4 w-full">
								<div className="h-1 overflow-hidden rounded-full bg-muted/30">
									<div
										className="h-full w-[30%] rounded-full transition-all duration-1000"
										style={{ backgroundColor: RECORDING_COLOR }}
									/>
								</div>
							</div>
						)}
					</div>
				</div>
				<p className="text-xs text-muted-foreground">
					Interactive visual state only; this preview never requests microphone
					access.
				</p>
			</div>
		</ProductDemoFrame>
	);
}
