"use client";

import {
	addDays,
	differenceInCalendarDays,
	eachDayOfInterval,
	startOfDay,
} from "date-fns";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	GanttChartIcon,
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
	type GanttRange,
	ganttRange,
	normalizeGanttTasks,
	taskBarDays,
	toDate,
} from "../planning-utils";
import type {
	BoundValue,
	GanttComponent,
	GanttTask,
	GanttView,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const VIEWS: GanttView[] = ["day", "week", "month", "quarter"];
const DAY_WIDTH: Record<GanttView, number> = {
	day: 40,
	week: 20,
	month: 6,
	quarter: 3,
	compact: 4,
};
const DEFAULT_ROW_HEIGHT = 36;
const DEFAULT_COMPACT_BREAKPOINT = 720;
const LEFT_PANEL_WIDTH = 220;
const HEADER_HEIGHT = 44;

function iso(date: Date): string {
	return date.toISOString();
}

interface DragState {
	taskId: string;
	kind: "move" | "resize-start" | "resize-end" | "link";
	startX: number;
	startY: number;
	origStart: Date;
	origEnd: Date;
	pointerX: number;
	pointerY: number;
}

export function A2UIGantt({
	component,
	componentId,
	style,
}: ComponentProps<GanttComponent>) {
	const containerRef = useRef<HTMLDivElement>(null);
	const timelineRef = useRef<HTMLDivElement>(null);
	const trigger = useComponentActionTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);

	const rawTasks = useResolved<unknown>(component.tasks);
	const viewProp = (useResolved<string>(component.view) as GanttView) ?? "week";
	const editable = useResolved<boolean>(component.editable) ?? true;
	const draggable =
		(useResolved<boolean>(component.draggable) ?? true) && editable;
	const resizable =
		(useResolved<boolean>(component.resizable) ?? true) && editable;
	const showDependencies =
		useResolved<boolean>(component.showDependencies) ?? true;
	const showProgress = useResolved<boolean>(component.showProgress) ?? true;
	const showToday = useResolved<boolean>(component.showToday) ?? true;
	const rowHeight =
		useResolved<number>(component.rowHeight) ?? DEFAULT_ROW_HEIGHT;
	const extraColumns = useResolved<string[]>(component.columns) ?? [];
	const height = useResolved<string>(component.height);
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const compactBreakpoint =
		useResolved<number>(component.compactBreakpoint) ??
		DEFAULT_COMPACT_BREAKPOINT;

	const resolvedTasks = useMemo(
		() => normalizeGanttTasks(rawTasks),
		[rawTasks],
	);
	const [tasks, setTasks] = useState<GanttTask[]>(resolvedTasks);
	useEffect(() => setTasks(resolvedTasks), [resolvedTasks]);

	const [view, setView] = useState<GanttView>(viewProp);
	useEffect(() => setView(viewProp), [viewProp]);

	const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

	const [isNarrow, setIsNarrow] = useState(false);
	useEffect(() => {
		if (!responsive || typeof ResizeObserver === "undefined") return;
		const el = containerRef.current;
		if (!el) return;
		const obs = new ResizeObserver((entries) => {
			for (const entry of entries)
				setIsNarrow(entry.contentRect.width < compactBreakpoint);
		});
		obs.observe(el);
		return () => obs.disconnect();
	}, [responsive, compactBreakpoint]);
	const effectiveView: GanttView = isNarrow ? "compact" : view;
	const dayWidth = DAY_WIDTH[effectiveView];

	const fire = useCallback(
		(interaction: string, extra: Record<string, unknown>) => {
			void trigger(component.actions, { interaction, ...extra });
		},
		[trigger, component.actions],
	);

	// Visible (non-collapsed-descendant) tasks in stable order.
	const visibleTasks = useMemo(() => {
		const collapsedIds = collapsed;
		const isHidden = (task: GanttTask): boolean => {
			let parent = task.parent;
			const guard = new Set<string>();
			while (parent && !guard.has(parent)) {
				guard.add(parent);
				if (collapsedIds.has(parent)) return true;
				parent = tasks.find((t) => t.id === parent)?.parent;
			}
			return false;
		};
		return tasks.filter((t) => !isHidden(t));
	}, [tasks, collapsed]);

	const range = useMemo(() => ganttRange(tasks), [tasks]);
	const rowIndex = useMemo(() => {
		const map = new Map<string, number>();
		visibleTasks.forEach((t, i) => map.set(t.id, i));
		return map;
	}, [visibleTasks]);

	const totalWidth = range.totalDays * dayWidth;
	const gridHeight = visibleTasks.length * rowHeight;

	const [drag, setDrag] = useState<DragState | null>(null);
	const [preview, setPreview] = useState<GanttTask | null>(null);

	const applyDrag = useCallback(
		(d: DragState, clientX: number): GanttTask | null => {
			const task = tasks.find((t) => t.id === d.taskId);
			if (!task) return null;
			const deltaDays = Math.round((clientX - d.startX) / dayWidth);
			if (d.kind === "move") {
				return {
					...task,
					start: iso(addDays(d.origStart, deltaDays)),
					end: iso(addDays(d.origEnd, deltaDays)),
				};
			}
			if (d.kind === "resize-start") {
				const newStart = addDays(d.origStart, deltaDays);
				if (newStart >= d.origEnd) return task;
				return { ...task, start: iso(newStart) };
			}
			if (d.kind === "resize-end") {
				const newEnd = addDays(d.origEnd, deltaDays);
				if (newEnd <= d.origStart) return task;
				return { ...task, end: iso(newEnd) };
			}
			return task;
		},
		[tasks, dayWidth],
	);

	const onPointerMove = useCallback(
		(e: React.PointerEvent) => {
			if (!drag) return;
			setDrag({ ...drag, pointerX: e.clientX, pointerY: e.clientY });
			if (drag.kind !== "link") setPreview(applyDrag(drag, e.clientX));
		},
		[drag, applyDrag],
	);

	const onPointerUp = useCallback(
		(e: React.PointerEvent) => {
			const d = drag;
			setDrag(null);
			if (!d) return;
			if (d.kind === "link") {
				const target = document
					.elementFromPoint(e.clientX, e.clientY)
					?.closest("[data-task-id]") as HTMLElement | null;
				const toId = target?.dataset.taskId;
				if (toId && toId !== d.taskId) {
					setTasks((list) =>
						list.map((t) => {
							if (t.id !== toId) return t;
							const deps = t.dependencies ?? [];
							return deps.includes(d.taskId)
								? t
								: { ...t, dependencies: [...deps, d.taskId] };
						}),
					);
					fire("link", { fromId: d.taskId, toId });
				}
			} else if (preview) {
				const original = tasks.find((t) => t.id === d.taskId);
				setTasks((list) =>
					list.map((t) => (t.id === preview.id ? preview : t)),
				);
				fire(d.kind === "move" ? "move" : "resize", {
					id: preview.id,
					start: preview.start,
					end: preview.end,
					oldStart: original?.start,
					oldEnd: original?.end,
					metadata: preview.metadata,
				});
			}
			setPreview(null);
		},
		[drag, preview, tasks, fire],
	);

	const displayTask = useCallback(
		(task: GanttTask) => (preview && preview.id === task.id ? preview : task),
		[preview],
	);

	const onCreateAt = useCallback(
		(clientX: number) => {
			const rect = timelineRef.current?.getBoundingClientRect();
			if (!rect) return;
			const scrollLeft = timelineRef.current?.scrollLeft ?? 0;
			const dayOffset = Math.floor(
				(clientX - rect.left + scrollLeft) / dayWidth,
			);
			const start = addDays(range.start, dayOffset);
			fire("create", {
				start: iso(startOfDay(start)),
				end: iso(startOfDay(addDays(start, 1))),
			});
		},
		[dayWidth, range.start, fire],
	);

	const onDelete = useCallback(
		(task: GanttTask) => {
			setTasks((list) => list.filter((t) => t.id !== task.id));
			fire("delete", { id: task.id, metadata: task.metadata });
		},
		[fire],
	);

	const monthSegments = useMemo(
		() => buildMonthSegments(range, dayWidth),
		[range, dayWidth],
	);
	const todayOffset = differenceInCalendarDays(
		startOfDay(new Date()),
		range.start,
	);
	const hasChildren = useCallback(
		(id: string) => tasks.some((t) => t.parent === id),
		[tasks],
	);

	return (
		<div
			ref={containerRef}
			className={cn(
				"flex flex-col rounded-lg border border-border bg-card text-card-foreground overflow-hidden",
				resolveStyle(style),
			)}
			style={{ height: height ?? "560px", ...resolveInlineStyle(style) }}
		>
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<h3 className="flex items-center gap-1.5 text-sm font-semibold">
					<GanttChartIcon className="h-4 w-4 text-muted-foreground" />
					Timeline
					{isTriggering && (
						<Loader2Icon className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
					)}
				</h3>
				<div className="flex items-center gap-2">
					{editable && (
						<Button
							variant="outline"
							size="sm"
							className="h-7"
							onClick={() => {
								const start = startOfDay(new Date());
								fire("create", {
									start: iso(start),
									end: iso(addDays(start, 1)),
								});
							}}
						>
							<PlusIcon className="h-3.5 w-3.5 mr-1" /> Task
						</Button>
					)}
					{!isNarrow && (
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
			</header>

			<div className="flex flex-1 overflow-hidden">
				{/* Left task panel */}
				<div
					className="shrink-0 overflow-y-auto border-r border-border"
					style={{ width: LEFT_PANEL_WIDTH }}
				>
					<div
						className="sticky top-0 z-10 flex items-center gap-2 border-b border-border bg-card px-2 text-xs font-medium text-muted-foreground"
						style={{ height: HEADER_HEIGHT }}
					>
						<span className="flex-1">Task</span>
						{extraColumns.map((c) => (
							<span key={c} className="w-14 truncate text-right capitalize">
								{c}
							</span>
						))}
					</div>
					{visibleTasks.map((task) => {
						const depth = taskDepth(task, tasks);
						return (
							<div
								key={task.id}
								className="group/row flex items-center gap-1 border-b border-border/60 px-2 text-xs"
								style={{ height: rowHeight, paddingLeft: 8 + depth * 12 }}
							>
								{hasChildren(task.id) ? (
									<button
										type="button"
										onClick={() =>
											setCollapsed((prev) => {
												const next = new Set(prev);
												if (next.has(task.id)) {
													next.delete(task.id);
												} else {
													next.add(task.id);
												}
												return next;
											})
										}
										className="shrink-0 text-muted-foreground"
									>
										{collapsed.has(task.id) ? (
											<ChevronRightIcon className="h-3.5 w-3.5" />
										) : (
											<ChevronDownIcon className="h-3.5 w-3.5" />
										)}
									</button>
								) : (
									<span className="w-3.5 shrink-0" />
								)}
								<button
									type="button"
									onClick={() =>
										fire("open", { id: task.id, metadata: task.metadata })
									}
									className="flex-1 truncate text-left hover:text-primary"
								>
									{task.name}
								</button>
								{extraColumns.map((c) => (
									<span
										key={c}
										className="w-14 truncate text-right text-muted-foreground"
									>
										{formatColumn(task, c)}
									</span>
								))}
								{editable && (
									<button
										type="button"
										onClick={() => onDelete(task)}
										className="hidden shrink-0 rounded p-0.5 text-muted-foreground hover:bg-accent group-hover/row:block"
										aria-label="Delete task"
									>
										<XIcon className="h-3 w-3" />
									</button>
								)}
							</div>
						);
					})}
					{visibleTasks.length === 0 && (
						<div className="px-3 py-6 text-center text-xs text-muted-foreground/60">
							No tasks
						</div>
					)}
				</div>

				{/* Timeline */}
				<div
					ref={timelineRef}
					className="relative flex-1 overflow-auto"
					onPointerMove={drag ? onPointerMove : undefined}
					onPointerUp={drag ? onPointerUp : undefined}
					onPointerLeave={drag ? onPointerUp : undefined}
				>
					<div style={{ width: Math.max(totalWidth, 100) }}>
						{/* Axis header */}
						<div
							className="sticky top-0 z-10 border-b border-border bg-card"
							style={{ height: HEADER_HEIGHT }}
						>
							<div className="relative h-full">
								{monthSegments.map((seg) => (
									<div
										key={seg.key}
										className="absolute top-0 h-full border-r border-border px-1 text-[11px] font-medium text-muted-foreground"
										style={{ left: seg.left, width: seg.width }}
									>
										{seg.label}
									</div>
								))}
							</div>
						</div>

						{/* Grid + bars */}
						<div
							className="relative"
							style={{ height: Math.max(gridHeight, rowHeight) }}
							onClick={(e) => {
								if (editable && e.currentTarget === e.target)
									onCreateAt(e.clientX);
							}}
						>
							{/* week gridlines */}
							{buildWeekLines(range, dayWidth).map((x) => (
								<div
									key={x}
									className="absolute top-0 bottom-0 border-r border-border/40"
									style={{ left: x }}
								/>
							))}
							{/* row separators */}
							{visibleTasks.map((t, i) => (
								<div
									key={`sep-${t.id}`}
									className="absolute inset-x-0 border-b border-border/40"
									style={{ top: (i + 1) * rowHeight - 1 }}
								/>
							))}
							{/* today marker */}
							{showToday &&
								todayOffset >= 0 &&
								todayOffset <= range.totalDays && (
									<div
										className="absolute top-0 bottom-0 z-10 border-l-2 border-red-500/70"
										style={{ left: todayOffset * dayWidth }}
									/>
								)}

							{/* dependency arrows */}
							{showDependencies && (
								<svg
									className="pointer-events-none absolute inset-0 z-20 h-full w-full overflow-visible"
									role="img"
									aria-hidden="true"
								>
									<title>Task dependencies</title>
									{visibleTasks.flatMap((task) =>
										(task.dependencies ?? []).map((depId) => {
											const from = tasks.find((t) => t.id === depId);
											if (!from || rowIndex.get(depId) === undefined)
												return null;
											const fromGeom = taskBarDays(displayTask(from), range);
											const toGeom = taskBarDays(displayTask(task), range);
											const fromRow = rowIndex.get(depId) ?? 0;
											const toRow = rowIndex.get(task.id) ?? 0;
											const x1 =
												(fromGeom.offsetDays + fromGeom.spanDays) * dayWidth;
											const y1 = fromRow * rowHeight + rowHeight / 2;
											const x2 = toGeom.offsetDays * dayWidth;
											const y2 = toRow * rowHeight + rowHeight / 2;
											return (
												<path
													key={`${depId}-${task.id}`}
													d={`M ${x1} ${y1} C ${x1 + 16} ${y1}, ${x2 - 16} ${y2}, ${x2} ${y2}`}
													className="stroke-muted-foreground/60"
													strokeWidth={1.5}
													fill="none"
													markerEnd="url(#gantt-arrow)"
												/>
											);
										}),
									)}
									<defs>
										<marker
											id="gantt-arrow"
											markerWidth="6"
											markerHeight="6"
											refX="5"
											refY="3"
											orient="auto"
										>
											<path
												d="M0,0 L6,3 L0,6 Z"
												className="fill-muted-foreground/60"
											/>
										</marker>
									</defs>
								</svg>
							)}

							{/* task bars */}
							{visibleTasks.map((task) => {
								const dt = displayTask(task);
								const geom = taskBarDays(dt, range);
								const top = (rowIndex.get(task.id) ?? 0) * rowHeight;
								const left = geom.offsetDays * dayWidth;
								const width = geom.spanDays * dayWidth;
								const canDrag = draggable;
								const canResize = resizable;

								if (task.milestone) {
									return (
										<div
											key={task.id}
											data-task-id={task.id}
											className="absolute z-30 flex items-center justify-center"
											style={{
												top: top + rowHeight / 2 - 7,
												left: left - 7,
												height: 14,
												width: 14,
											}}
											onClick={() =>
												fire("open", { id: task.id, metadata: task.metadata })
											}
											title={task.name}
										>
											<div
												className="h-3 w-3 rotate-45 border border-primary bg-primary"
												style={
													task.color
														? {
																backgroundColor: task.color,
																borderColor: task.color,
															}
														: undefined
												}
											/>
										</div>
									);
								}

								return (
									<div
										key={task.id}
										data-task-id={task.id}
										className={cn(
											"group/bar absolute z-30 flex items-center overflow-hidden rounded border border-primary/50 bg-primary/25",
											canDrag
												? "cursor-grab active:cursor-grabbing"
												: "cursor-pointer",
										)}
										style={{
											top: top + 4,
											left,
											width: Math.max(dayWidth, width),
											height: rowHeight - 8,
											...(task.color
												? {
														backgroundColor: `${task.color}40`,
														borderColor: task.color,
													}
												: {}),
										}}
										onPointerDown={(e) => {
											if (!canDrag) return;
											e.stopPropagation();
											setDrag({
												taskId: task.id,
												kind: "move",
												startX: e.clientX,
												startY: e.clientY,
												origStart: toDate(task.start),
												origEnd: toDate(task.end),
												pointerX: e.clientX,
												pointerY: e.clientY,
											});
										}}
										onClick={(e) => {
											e.stopPropagation();
											if (!drag)
												fire("open", { id: task.id, metadata: task.metadata });
										}}
									>
										{showProgress && task.progress != null && (
											<div
												className="absolute inset-y-0 left-0 bg-primary/40"
												style={{
													width: `${Math.max(0, Math.min(100, task.progress))}%`,
												}}
											/>
										)}
										<span className="relative z-10 truncate px-1.5 text-[11px]">
											{task.name}
										</span>
										{canResize && (
											<>
												<div
													onPointerDown={(e) => {
														e.stopPropagation();
														setDrag({
															taskId: task.id,
															kind: "resize-start",
															startX: e.clientX,
															startY: e.clientY,
															origStart: toDate(task.start),
															origEnd: toDate(task.end),
															pointerX: e.clientX,
															pointerY: e.clientY,
														});
													}}
													className="absolute inset-y-0 left-0 z-20 w-1.5 cursor-ew-resize"
												/>
												<div
													onPointerDown={(e) => {
														e.stopPropagation();
														setDrag({
															taskId: task.id,
															kind: "resize-end",
															startX: e.clientX,
															startY: e.clientY,
															origStart: toDate(task.start),
															origEnd: toDate(task.end),
															pointerX: e.clientX,
															pointerY: e.clientY,
														});
													}}
													className="absolute inset-y-0 right-0 z-20 w-1.5 cursor-ew-resize"
												/>
											</>
										)}
										{editable && showDependencies && (
											<div
												onPointerDown={(e) => {
													e.stopPropagation();
													setDrag({
														taskId: task.id,
														kind: "link",
														startX: e.clientX,
														startY: e.clientY,
														origStart: toDate(task.start),
														origEnd: toDate(task.end),
														pointerX: e.clientX,
														pointerY: e.clientY,
													});
												}}
												className="absolute -right-1.5 top-1/2 z-30 hidden h-3 w-3 -translate-y-1/2 cursor-crosshair rounded-full border border-primary bg-background group-hover/bar:block"
												title="Drag to link a dependency"
											/>
										)}
									</div>
								);
							})}
						</div>
					</div>
				</div>
			</div>
		</div>
	);
}

interface MonthSegment {
	key: string;
	left: number;
	width: number;
	label: string;
}

function buildMonthSegments(
	range: GanttRange,
	dayWidth: number,
): MonthSegment[] {
	const days = eachDayOfInterval({ start: range.start, end: range.end });
	const segments: MonthSegment[] = [];
	let startIndex = 0;
	for (let i = 0; i <= days.length; i++) {
		const prev = days[i - 1];
		const cur = days[i];
		const boundary =
			i === days.length || (prev && cur && prev.getMonth() !== cur.getMonth());
		if (boundary && prev) {
			const count = i - startIndex;
			segments.push({
				key: `${prev.getFullYear()}-${prev.getMonth()}`,
				left: startIndex * dayWidth,
				width: count * dayWidth,
				label: prev.toLocaleDateString(undefined, {
					month: "short",
					year: "2-digit",
				}),
			});
			startIndex = i;
		}
	}
	return segments;
}

function buildWeekLines(range: GanttRange, dayWidth: number): number[] {
	const days = eachDayOfInterval({ start: range.start, end: range.end });
	const lines: number[] = [];
	days.forEach((d, i) => {
		if (d.getDay() === 1) lines.push(i * dayWidth);
	});
	return lines;
}

function taskDepth(task: GanttTask, tasks: GanttTask[]): number {
	let depth = 0;
	let parent = task.parent;
	const guard = new Set<string>();
	while (parent && !guard.has(parent)) {
		guard.add(parent);
		depth += 1;
		parent = tasks.find((t) => t.id === parent)?.parent;
	}
	return depth;
}

function formatColumn(task: GanttTask, column: string): string {
	if (column === "progress")
		return task.progress != null ? `${Math.round(task.progress)}%` : "";
	if (column === "assignee") return task.assignee ?? "";
	const value = (task as unknown as Record<string, unknown>)[column];
	return value == null ? "" : String(value);
}
