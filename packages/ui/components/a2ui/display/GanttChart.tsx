"use client";

import { useTranslation } from "@flow-like/locales";
import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
	addDays,
	differenceInCalendarDays,
	eachDayOfInterval,
	startOfDay,
} from "date-fns";
import {
	CheckCircle2Icon,
	ChevronDownIcon,
	ChevronRightIcon,
	CopyIcon,
	EyeIcon,
	GanttChartIcon,
	GripVerticalIcon,
	Loader2Icon,
	PencilIcon,
	PlusIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { Badge, Button, Input } from "../../ui/index";
import {
	useComponentEventTrigger,
	useIsComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	AssigneeDisplay,
	PlanningContextMenu,
	type PlanningMenuAction,
	TaskDialog,
	type TaskDialogState,
	planningTint,
} from "../planning-dialogs";
import {
	densityPreset,
	ganttRange,
	genId,
	normalizeGanttTasks,
	taskBarDays,
	toDate,
	toDateInput,
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
// Fallback zoom widths for the first render, before the container is measured.
const DAY_WIDTH: Record<GanttView, number> = {
	day: 40,
	week: 20,
	month: 6,
	quarter: 3,
	compact: 4,
};
// Zoom = how many days fit into the visible timeline width. Day width follows
// the container, so every level fills the viewport yet scales distinctly.
const VIEW_TARGET_DAYS: Record<GanttView, number> = {
	day: 21,
	week: 84,
	month: 210,
	quarter: 455,
	compact: 180,
};
const DEFAULT_COMPACT_BREAKPOINT = 720;
const DEFAULT_TASK_LIST_WIDTH = 240;
const HEADER_TIER1 = 20;
const HEADER_TIER2 = 24;
const HEADER_HEIGHT = HEADER_TIER1 + HEADER_TIER2;
const DRAG_THRESHOLD = 3;

function fmtShortDay(d: Date): string {
	return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function chipLabel(start: Date, end: Date): string {
	const a = fmtShortDay(start);
	const b = fmtShortDay(end);
	return a === b ? a : `${a} → ${b}`;
}

type DragState =
	| {
			type: "bar";
			kind: "move" | "resize-start" | "resize-end";
			taskId: string;
			startX: number;
			origStart: Date;
			origEnd: Date;
			pointerX: number;
			pointerY: number;
	  }
	| {
			type: "link";
			taskId: string;
			startX: number;
			gridX: number;
			gridY: number;
			targetId: string | null;
	  }
	| {
			type: "create";
			anchorDay: number;
			currentDay: number;
			row: number;
			startX: number;
			pointerX: number;
			pointerY: number;
	  };

export function A2UIGantt({
	component,
	componentId,
	style,
}: ComponentProps<GanttComponent>) {
	const { t } = useTranslation("common");
	const containerRef = useRef<HTMLDivElement>(null);
	const timelineRef = useRef<HTMLDivElement>(null);
	const listBodyRef = useRef<HTMLDivElement>(null);
	const gridRef = useRef<HTMLDivElement>(null);
	// Suppresses the click that follows a >3px drag.
	const movedRef = useRef(false);
	const triggerEvent = useComponentEventTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);

	const rawTasks = useResolved<unknown>(component.tasks);
	const rawView = useResolved<string>(component.view);
	// Unknown view values fall back to week instead of a zero-width timeline.
	const viewProp: GanttView =
		rawView && rawView in DAY_WIDTH ? (rawView as GanttView) : "week";
	const title = useResolved<string>(component.title) ?? "Timeline";
	const density = useResolved<string>(component.density);
	const editable = useResolved<boolean>(component.editable) ?? true;
	const draggable =
		(useResolved<boolean>(component.draggable) ?? true) && editable;
	const resizable =
		(useResolved<boolean>(component.resizable) ?? true) && editable;
	const showDependencies =
		useResolved<boolean>(component.showDependencies) ?? true;
	const showProgress = useResolved<boolean>(component.showProgress) ?? true;
	const showToday = useResolved<boolean>(component.showToday) ?? true;
	const showViewSwitcher =
		useResolved<boolean>(component.showViewSwitcher) ?? true;
	const showTaskList = useResolved<boolean>(component.showTaskList) ?? true;
	const taskListWidth =
		useResolved<number>(component.taskListWidth) ?? DEFAULT_TASK_LIST_WIDTH;
	const shadeWeekends = useResolved<boolean>(component.shadeWeekends) ?? true;
	const rowHeightProp = useResolved<number>(component.rowHeight);
	const rowHeight = rowHeightProp ?? densityPreset(density).rowHeight;
	const rawColumns = useResolved<unknown>(component.columns);
	const extraColumns = useMemo(() => {
		if (Array.isArray(rawColumns)) return rawColumns.map(String);
		if (typeof rawColumns === "string" && rawColumns.trim()) {
			return rawColumns
				.split(",")
				.map((c) => c.trim())
				.filter(Boolean);
		}
		return [];
	}, [rawColumns]);
	const height = useResolved<string>(component.height);
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const compactBreakpoint =
		useResolved<number>(component.compactBreakpoint) ??
		DEFAULT_COMPACT_BREAKPOINT;

	// `useResolved` re-parses `literalJson` into a fresh array every render, so
	// key the memo/effect on the serialized content — otherwise the sync effect
	// below would fire on every render and wipe local edits/creates instantly.
	const tasksKey = JSON.stringify(rawTasks ?? null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: tasksKey is the stable identity of rawTasks
	const resolvedTasks = useMemo(
		() => normalizeGanttTasks(rawTasks),
		[tasksKey],
	);
	const [tasks, setTasks] = useState<GanttTask[]>(resolvedTasks);
	useEffect(() => setTasks(resolvedTasks), [resolvedTasks]);

	const [view, setView] = useState<GanttView>(viewProp);
	useEffect(() => setView(viewProp), [viewProp]);

	const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
	const [hoveredId, setHoveredId] = useState<string | null>(null);
	const [dialogState, setDialogState] = useState<TaskDialogState | null>(null);
	const [drag, setDrag] = useState<DragState | null>(null);
	const [preview, setPreview] = useState<GanttTask | null>(null);
	const [rename, setRename] = useState<{ id: string; value: string } | null>(
		null,
	);
	// HTML5 drag events don't fire inside Tauri's webview — the list reorder
	// uses dnd-kit's pointer sensor instead.
	const listSensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
	);

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
	const effectiveView: GanttView = isNarrow ? "compact" : view;
	const listVisible = showTaskList && !isNarrow;

	const fire = useCallback(
		(interaction: string, extra: Record<string, unknown>) => {
			void triggerEvent(interaction, component, { interaction, ...extra });
		},
		[triggerEvent, component],
	);

	// Visible (non-collapsed-descendant) tasks in stable order.
	const visibleTasks = useMemo(() => {
		const taskMap = new Map(tasks.map((t) => [t.id, t]));
		const isHidden = (task: GanttTask): boolean => {
			let parent = task.parent;
			const guard = new Set<string>();
			while (parent && !guard.has(parent)) {
				guard.add(parent);
				if (collapsed.has(parent)) return true;
				parent = taskMap.get(parent)?.parent;
			}
			return false;
		};
		return tasks.filter((t) => !isHidden(t));
	}, [tasks, collapsed]);

	const taskRange = useMemo(() => ganttRange(tasks), [tasks]);
	// Extend short task ranges to the view's day target so the grid fills the
	// viewport at every zoom level instead of leaving an empty band.
	const range = useMemo(() => {
		const totalDays = Math.max(
			taskRange.totalDays,
			VIEW_TARGET_DAYS[effectiveView],
		);
		return {
			start: taskRange.start,
			end: addDays(taskRange.start, totalDays - 1),
			totalDays,
		};
	}, [taskRange, effectiveView]);
	const rangeDays = useMemo(
		() => eachDayOfInterval({ start: range.start, end: range.end }),
		[range],
	);
	const rowIndex = useMemo(() => {
		const map = new Map<string, number>();
		visibleTasks.forEach((t, i) => map.set(t.id, i));
		return map;
	}, [visibleTasks]);

	const availableWidth = Math.max(
		0,
		containerWidth - (listVisible ? taskListWidth : 0),
	);
	const dayWidth = useMemo(() => {
		if (availableWidth <= 0) return DAY_WIDTH[effectiveView];
		return Math.min(
			120,
			Math.max(1.5, availableWidth / VIEW_TARGET_DAYS[effectiveView]),
		);
	}, [availableWidth, effectiveView]);

	const totalWidth = range.totalDays * dayWidth;
	const gridHeight = visibleTasks.length * rowHeight;

	// The timeline's horizontal scrollbar steals client height the task list
	// doesn't lose; pad the list bottom by the measured difference so both
	// panes share the same vertical scroll range (0 with overlay scrollbars).
	const [hScrollbarHeight, setHScrollbarHeight] = useState(0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: re-measure whenever the layout-driving values change
	useEffect(() => {
		const tl = timelineRef.current;
		if (!tl) return;
		setHScrollbarHeight(Math.max(0, tl.offsetHeight - tl.clientHeight));
	}, [totalWidth, containerWidth]);

	const monthSegments = useMemo(
		() => buildMonthSegments(rangeDays, dayWidth),
		[rangeDays, dayWidth],
	);
	const weekendBands = useMemo(
		() =>
			shadeWeekends && dayWidth >= 6
				? buildWeekendBands(rangeDays, dayWidth)
				: [],
		[shadeWeekends, rangeDays, dayWidth],
	);
	const todayOffset = differenceInCalendarDays(
		startOfDay(new Date()),
		range.start,
	);

	const hasChildren = useCallback(
		(id: string) => tasks.some((t) => t.parent === id),
		[tasks],
	);
	const displayTask = useCallback(
		(task: GanttTask) => (preview && preview.id === task.id ? preview : task),
		[preview],
	);
	const clampDay = useCallback(
		(day: number) =>
			Math.min(Math.max(0, day), Math.max(0, range.totalDays - 1)),
		[range.totalDays],
	);

	// ── Mutations (optimistic local state + workflow action) ─────────

	const openTask = useCallback(
		(task: GanttTask, mode: "view" | "edit") => {
			if (mode === "view" && dialogState?.task.id !== task.id)
				fire("open", { id: task.id, metadata: task.metadata });
			setDialogState({ task, mode });
		},
		[dialogState, fire],
	);

	const openCreateDialog = useCallback((start: Date, end: Date) => {
		setDialogState({
			task: {
				id: genId("task"),
				name: "",
				start: toDateInput(start),
				end: toDateInput(end),
			},
			mode: "create",
		});
	}, []);

	const openCreateDraft = useCallback(() => {
		const lastEnd = tasks.reduce<Date | null>((acc, t) => {
			const e = toDate(t.end);
			return !acc || e > acc ? e : acc;
		}, null);
		const start = startOfDay(lastEnd ?? new Date());
		openCreateDialog(start, addDays(start, 2));
	}, [tasks, openCreateDialog]);

	const deleteTask = useCallback(
		(task: GanttTask) => {
			setTasks((list) => list.filter((t) => t.id !== task.id));
			fire("delete", { id: task.id, metadata: task.metadata });
		},
		[fire],
	);

	const duplicateTask = useCallback(
		(task: GanttTask) => {
			const copy: GanttTask = {
				...task,
				id: genId("task"),
				name: t('nameCopy', '{{name}} (copy)', { name: task.name }),
			};
			setTasks((list) => {
				const i = list.findIndex((t) => t.id === task.id);
				const next = [...list];
				next.splice(i < 0 ? next.length : i + 1, 0, copy);
				return next;
			});
			fire("create", {
				id: copy.id,
				start: copy.start,
				end: copy.end,
				task: copy,
				sourceId: task.id,
			});
		},
		[fire],
	);

	const completeTask = useCallback(
		(task: GanttTask) => {
			const next: GanttTask = { ...task, progress: 100 };
			setTasks((list) => list.map((t) => (t.id === next.id ? next : t)));
			fire("update", { id: next.id, task: next, previous: task });
		},
		[fire],
	);

	const onDialogSave = useCallback(
		(
			next: GanttTask,
			original: GanttTask,
			mode: "view" | "edit" | "create",
		) => {
			if (mode === "create") {
				setTasks((list) => [...list, next]);
				fire("create", {
					id: next.id,
					start: next.start,
					end: next.end,
					task: next,
				});
				return;
			}
			setTasks((list) => list.map((t) => (t.id === next.id ? next : t)));
			fire("update", { id: next.id, task: next, previous: original });
		},
		[fire],
	);

	const commitRename = useCallback(() => {
		if (!rename) return;
		const task = tasks.find((t) => t.id === rename.id);
		const nextName = rename.value.trim();
		setRename(null);
		if (!task || !nextName || nextName === task.name) return;
		const next: GanttTask = { ...task, name: nextName };
		setTasks((list) => list.map((t) => (t.id === next.id ? next : t)));
		fire("update", { id: next.id, task: next, previous: task });
	}, [rename, tasks, fire]);

	// ── List drag-and-drop reorder (dnd-kit) ─────────────────────────

	// Moves the dragged task together with all its descendants so hierarchy
	// blocks stay contiguous; dropping onto one's own descendant is a no-op.
	const onListDragEnd = useCallback(
		(e: DragEndEvent) => {
			const fromId = String(e.active.id);
			const toId = e.over ? String(e.over.id) : null;
			if (!toId || fromId === toId) return;
			const fromIndex = tasks.findIndex((t) => t.id === fromId);
			const toIndex = tasks.findIndex((t) => t.id === toId);
			if (fromIndex < 0 || toIndex < 0) return;

			const blockIds = new Set([fromId]);
			let grew = true;
			while (grew) {
				grew = false;
				for (const t of tasks) {
					if (t.parent && blockIds.has(t.parent) && !blockIds.has(t.id)) {
						blockIds.add(t.id);
						grew = true;
					}
				}
			}
			if (blockIds.has(toId)) return;

			const block = tasks.filter((t) => blockIds.has(t.id));
			const rest = tasks.filter((t) => !blockIds.has(t.id));
			const targetIndex = rest.findIndex((t) => t.id === toId);
			const insertAt = fromIndex < toIndex ? targetIndex + 1 : targetIndex;
			const next = [...rest];
			next.splice(insertAt, 0, ...block);

			setTasks(next);
			fire("reorder", {
				id: fromId,
				fromIndex,
				toIndex,
				order: next.map((t) => t.id),
			});
		},
		[tasks, fire],
	);

	const visibleIds = useMemo(
		() => visibleTasks.map((t) => t.id),
		[visibleTasks],
	);

	// ── Timeline pointer drags (move / resize / link / create) ───────

	// Bar drags are day-granular, so emit local date-only strings — same format
	// the task dialog saves — instead of UTC ISO (which shifts a day east of
	// UTC and breaks format consistency for workflow consumers).
	const applyDrag = useCallback(
		(
			d: Extract<DragState, { type: "bar" }>,
			clientX: number,
		): GanttTask | null => {
			const task = tasks.find((t) => t.id === d.taskId);
			if (!task) return null;
			const deltaDays = Math.round((clientX - d.startX) / dayWidth);
			if (d.kind === "move") {
				return {
					...task,
					start: toDateInput(addDays(d.origStart, deltaDays)),
					end: toDateInput(addDays(d.origEnd, deltaDays)),
				};
			}
			if (d.kind === "resize-start") {
				const newStart = addDays(d.origStart, deltaDays);
				if (newStart >= d.origEnd) return task;
				return { ...task, start: toDateInput(newStart) };
			}
			const newEnd = addDays(d.origEnd, deltaDays);
			if (newEnd <= d.origStart) return task;
			return { ...task, end: toDateInput(newEnd) };
		},
		[tasks, dayWidth],
	);

	const startBarDrag = useCallback(
		(
			task: GanttTask,
			kind: "move" | "resize-start" | "resize-end",
			e: React.PointerEvent,
		) => {
			if (e.button !== 0) return;
			e.stopPropagation();
			movedRef.current = false;
			setDrag({
				type: "bar",
				kind,
				taskId: task.id,
				startX: e.clientX,
				origStart: toDate(task.start),
				origEnd: toDate(task.end),
				pointerX: e.clientX,
				pointerY: e.clientY,
			});
		},
		[],
	);

	const startLinkDrag = useCallback(
		(task: GanttTask, e: React.PointerEvent) => {
			if (e.button !== 0) return;
			e.stopPropagation();
			movedRef.current = false;
			const rect = gridRef.current?.getBoundingClientRect();
			setDrag({
				type: "link",
				taskId: task.id,
				startX: e.clientX,
				gridX: rect ? e.clientX - rect.left : 0,
				gridY: rect ? e.clientY - rect.top : 0,
				targetId: null,
			});
		},
		[],
	);

	const onGridPointerDown = useCallback(
		(e: React.PointerEvent) => {
			if (!editable || e.button !== 0 || drag) return;
			if ((e.target as HTMLElement).closest("[data-task-id]")) return;
			const rect = gridRef.current?.getBoundingClientRect();
			if (!rect) return;
			const day = clampDay(Math.floor((e.clientX - rect.left) / dayWidth));
			const row =
				visibleTasks.length === 0
					? 0
					: Math.min(
							Math.max(0, Math.floor((e.clientY - rect.top) / rowHeight)),
							visibleTasks.length - 1,
						);
			movedRef.current = false;
			setDrag({
				type: "create",
				anchorDay: day,
				currentDay: day,
				row,
				startX: e.clientX,
				pointerX: e.clientX,
				pointerY: e.clientY,
			});
		},
		[editable, drag, dayWidth, rowHeight, visibleTasks.length, clampDay],
	);

	const onPointerMove = useCallback(
		(e: React.PointerEvent) => {
			if (!drag) return;
			if (Math.abs(e.clientX - drag.startX) > DRAG_THRESHOLD)
				movedRef.current = true;
			if (drag.type === "bar") {
				setDrag({ ...drag, pointerX: e.clientX, pointerY: e.clientY });
				setPreview(applyDrag(drag, e.clientX));
			} else if (drag.type === "create") {
				const rect = gridRef.current?.getBoundingClientRect();
				const day = rect
					? clampDay(Math.floor((e.clientX - rect.left) / dayWidth))
					: drag.currentDay;
				setDrag({
					...drag,
					currentDay: day,
					pointerX: e.clientX,
					pointerY: e.clientY,
				});
			} else if (drag.type === "link") {
				const rect = gridRef.current?.getBoundingClientRect();
				const target = document
					.elementFromPoint(e.clientX, e.clientY)
					?.closest("[data-task-id]") as HTMLElement | null;
				const targetId =
					target?.dataset.taskId && target.dataset.taskId !== drag.taskId
						? target.dataset.taskId
						: null;
				setDrag({
					...drag,
					gridX: rect ? e.clientX - rect.left : drag.gridX,
					gridY: rect ? e.clientY - rect.top : drag.gridY,
					targetId,
				});
			}
		},
		[drag, applyDrag, dayWidth, clampDay],
	);

	const finishDrag = useCallback(
		(e: React.PointerEvent, cancelCreate: boolean) => {
			const d = drag;
			const p = preview;
			setDrag(null);
			setPreview(null);
			// Clear the suppression flag after any trailing click has fired.
			window.setTimeout(() => {
				movedRef.current = false;
			}, 0);
			if (!d) return;
			if (d.type === "link") {
				const target = document
					.elementFromPoint(e.clientX, e.clientY)
					?.closest("[data-task-id]") as HTMLElement | null;
				const toId = d.targetId ?? target?.dataset.taskId;
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
				return;
			}
			if (d.type === "bar") {
				if (!p || !movedRef.current) return;
				const original = tasks.find((t) => t.id === d.taskId);
				// Compare instants — re-encoding the same dates must not commit.
				if (
					!original ||
					(toDate(p.start).getTime() === toDate(original.start).getTime() &&
						toDate(p.end).getTime() === toDate(original.end).getTime())
				)
					return;
				setTasks((list) => list.map((t) => (t.id === p.id ? p : t)));
				fire(d.kind === "move" ? "move" : "resize", {
					id: p.id,
					start: p.start,
					end: p.end,
					oldStart: original.start,
					oldEnd: original.end,
					metadata: p.metadata,
				});
				return;
			}
			if (cancelCreate) return;
			const a = movedRef.current
				? Math.min(d.anchorDay, d.currentDay)
				: d.anchorDay;
			const b = movedRef.current
				? Math.max(d.anchorDay, d.currentDay)
				: d.anchorDay;
			openCreateDialog(addDays(range.start, a), addDays(range.start, b));
		},
		[drag, preview, tasks, fire, openCreateDialog, range.start],
	);

	const handleBarClick = useCallback(
		(task: GanttTask) => {
			if (movedRef.current) {
				movedRef.current = false;
				return;
			}
			openTask(task, "view");
		},
		[openTask],
	);

	const handleBarDoubleClick = useCallback(
		(task: GanttTask) => {
			if (movedRef.current) {
				movedRef.current = false;
				return;
			}
			openTask(task, editable ? "edit" : "view");
		},
		[openTask, editable],
	);

	const menuGroupsFor = useCallback(
		(task: GanttTask): PlanningMenuAction[][] => {
			const viewItem: PlanningMenuAction = {
				label: t('viewDetails', 'View details'),
				icon: <EyeIcon className="h-3.5 w-3.5" />,
				onSelect: () => openTask(task, "view"),
			};
			if (!editable) return [[viewItem]];
			const secondary: PlanningMenuAction[] = [
				{
					label: t('duplicate', 'Duplicate'),
					icon: <CopyIcon className="h-3.5 w-3.5" />,
					onSelect: () => duplicateTask(task),
				},
			];
			if (!task.milestone) {
				secondary.push({
					label: t('complete', 'Complete'),
					icon: <CheckCircle2Icon className="h-3.5 w-3.5" />,
					onSelect: () => completeTask(task),
				});
			}
			return [
				[
					viewItem,
					{
						label: t('edit', 'Edit'),
						icon: <PencilIcon className="h-3.5 w-3.5" />,
						onSelect: () => setDialogState({ task, mode: "edit" }),
					},
				],
				secondary,
				[
					{
						label: t('delete', 'Delete'),
						icon: <Trash2Icon className="h-3.5 w-3.5" />,
						destructive: true,
						onSelect: () => deleteTask(task),
					},
				],
			];
		},
		[editable, openTask, duplicateTask, completeTask, deleteTask],
	);

	// ── Scroll sync between task list and timeline ───────────────────

	const syncFromTimeline = useCallback(() => {
		const tl = timelineRef.current;
		const list = listBodyRef.current;
		if (tl && list && list.scrollTop !== tl.scrollTop)
			list.scrollTop = tl.scrollTop;
	}, []);
	const syncFromList = useCallback(() => {
		const tl = timelineRef.current;
		const list = listBodyRef.current;
		if (tl && list && tl.scrollTop !== list.scrollTop)
			tl.scrollTop = list.scrollTop;
	}, []);

	const chip = useMemo(() => {
		if (!drag) return null;
		if (drag.type === "bar" && preview) {
			return {
				x: drag.pointerX,
				y: drag.pointerY,
				label: chipLabel(toDate(preview.start), toDate(preview.end)),
			};
		}
		if (drag.type === "create") {
			const a = Math.min(drag.anchorDay, drag.currentDay);
			const b = Math.max(drag.anchorDay, drag.currentDay);
			return {
				x: drag.pointerX,
				y: drag.pointerY,
				label: chipLabel(addDays(range.start, a), addDays(range.start, b)),
			};
		}
		return null;
	}, [drag, preview, range.start]);

	const ghost = useMemo(() => {
		if (drag?.type !== "create") return null;
		const a = Math.min(drag.anchorDay, drag.currentDay);
		const b = Math.max(drag.anchorDay, drag.currentDay);
		return {
			top: drag.row * rowHeight + 5,
			left: a * dayWidth,
			width: (b - a + 1) * dayWidth,
			height: rowHeight - 10,
		};
	}, [drag, rowHeight, dayWidth]);

	// Rubber-band feedback while dragging a dependency link.
	const linkLine = useMemo(() => {
		if (drag?.type !== "link") return null;
		const row = rowIndex.get(drag.taskId);
		const task = tasks.find((t) => t.id === drag.taskId);
		if (row === undefined || !task) return null;
		const geom = taskBarDays(task, range);
		return {
			x1: (geom.offsetDays + geom.spanDays) * dayWidth,
			y1: row * rowHeight + rowHeight / 2,
			x2: drag.gridX,
			y2: drag.gridY,
		};
	}, [drag, rowIndex, tasks, range, dayWidth, rowHeight]);

	return (
		<div
			ref={containerRef}
			className={cn(
				"flex flex-col rounded-xl border border-border bg-card text-card-foreground shadow-sm overflow-hidden",
				resolveStyle(style),
			)}
			style={{ height: height ?? "560px", ...resolveInlineStyle(style) }}
		>
			<header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
				<div className="flex min-w-0 items-center gap-2">
					<h3 className="flex min-w-0 items-center gap-1.5 text-sm font-semibold">
						<GanttChartIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
						<span className="truncate">{title}</span>
					</h3>
					<Badge variant="secondary" className="shrink-0 text-[10px]">
						{tasks.length} {tasks.length === 1 ? "task" : "tasks"}
					</Badge>
					{isTriggering && (
						<Loader2Icon className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
					)}
				</div>
				<div className="flex shrink-0 items-center gap-2">
					{editable && (
						<Button
							variant="outline"
							size="sm"
							className="h-7"
							onClick={openCreateDraft}
						>
							<PlusIcon className="h-3.5 w-3.5 mr-1" /> {t('task', 'Task')}
						</Button>
					)}
					{showViewSwitcher && !isNarrow && (
						<ViewSwitcher view={view} onChange={setView} />
					)}
				</div>
			</header>

			{tasks.length === 0 ? (
				<GanttEmptyState editable={editable} onAdd={openCreateDraft} />
			) : (
				<div className="flex flex-1 overflow-hidden">
					{listVisible && (
						<aside
							className="flex shrink-0 flex-col border-r border-border"
							style={{ width: taskListWidth }}
						>
							<div
								className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-2 text-xs font-medium text-muted-foreground"
								style={{ height: HEADER_HEIGHT }}
							>
								<span className="flex-1">{t('task', 'Task')}</span>
								{extraColumns.map((c) => (
									<span key={c} className="w-14 truncate text-right capitalize">
										{c}
									</span>
								))}
							</div>
							<div
								ref={listBodyRef}
								onScroll={syncFromList}
								className="flex-1 overflow-y-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
								style={{ paddingBottom: hScrollbarHeight }}
							>
								<DndContext
									sensors={listSensors}
									collisionDetection={closestCenter}
									onDragEnd={onListDragEnd}
								>
									<SortableContext
										items={visibleIds}
										strategy={verticalListSortingStrategy}
									>
										{visibleTasks.map((task, i) => (
											<TaskListRow
												key={task.id}
												task={task}
												index={i}
												depth={taskDepth(task, tasks)}
												rowHeight={rowHeight}
												editable={editable}
												extraColumns={extraColumns}
												hovered={hoveredId === task.id}
												parent={hasChildren(task.id)}
												isCollapsed={collapsed.has(task.id)}
												renameValue={
													rename?.id === task.id ? rename.value : null
												}
												menuGroups={menuGroupsFor(task)}
												onToggleCollapse={() =>
													setCollapsed((prev) => {
														const next = new Set(prev);
														if (next.has(task.id)) next.delete(task.id);
														else next.add(task.id);
														return next;
													})
												}
												onOpen={() => openTask(task, "view")}
												onRenameStart={() =>
													setRename({ id: task.id, value: task.name })
												}
												onRenameChange={(value) =>
													setRename({ id: task.id, value })
												}
												onRenameCommit={commitRename}
												onRenameCancel={() => setRename(null)}
												onDelete={() => deleteTask(task)}
												onHover={(over) => setHoveredId(over ? task.id : null)}
											/>
										))}
									</SortableContext>
								</DndContext>
							</div>
						</aside>
					)}

					<div
						ref={timelineRef}
						className={cn(
							"relative flex-1 overflow-auto",
							drag?.type === "link" && "cursor-crosshair",
						)}
						onScroll={syncFromTimeline}
						onPointerMove={drag ? onPointerMove : undefined}
						onPointerUp={drag ? (e) => finishDrag(e, false) : undefined}
						onPointerLeave={
							drag ? (e) => finishDrag(e, drag.type === "create") : undefined
						}
					>
						<div style={{ width: Math.max(totalWidth, 240) }}>
							<TimelineHeader
								segments={monthSegments}
								days={rangeDays}
								dayWidth={dayWidth}
							/>

							<div
								ref={gridRef}
								className="relative select-none"
								style={{ height: Math.max(gridHeight, rowHeight) }}
								onPointerDown={onGridPointerDown}
							>
								{weekendBands.map((band) => (
									<div
										key={`wk-${band.left}`}
										className="pointer-events-none absolute top-0 bottom-0 bg-muted/40"
										style={{ left: band.left, width: band.width }}
									/>
								))}

								{visibleTasks.map((t, i) => (
									<div
										key={`row-${t.id}`}
										className={cn(
											"absolute inset-x-0 border-b border-border/30",
											i % 2 === 1 && "bg-muted/20",
											hoveredId === t.id && "bg-accent/30",
										)}
										style={{ top: i * rowHeight, height: rowHeight }}
										onMouseEnter={() => setHoveredId(t.id)}
										onMouseLeave={() => setHoveredId(null)}
									/>
								))}

								{effectiveView === "day" &&
									rangeDays.map((d, i) => (
										<div
											key={`day-${d.toISOString()}`}
											className="pointer-events-none absolute top-0 bottom-0 border-r border-border/25"
											style={{ left: (i + 1) * dayWidth }}
										/>
									))}
								{rangeDays.map((d, i) =>
									d.getDay() === 1 ? (
										<div
											key={`week-${d.toISOString()}`}
											className="pointer-events-none absolute top-0 bottom-0 border-r border-border/40"
											style={{ left: i * dayWidth }}
										/>
									) : null,
								)}

								{showToday &&
									todayOffset >= 0 &&
									todayOffset <= range.totalDays && (
										<div
											className="pointer-events-none absolute top-0 bottom-0 z-10"
											style={{ left: todayOffset * dayWidth }}
										>
											<div className="h-full w-0.5 bg-red-500/70" />
											<div className="absolute -left-0.75 top-0 h-2 w-2 rounded-full bg-red-500/70" />
										</div>
									)}

								{ghost && (
									<div
										className="pointer-events-none absolute z-20 rounded-md border border-dashed border-primary/70 bg-primary/10"
										style={ghost}
									/>
								)}

								{showDependencies && (
									<DependencyArrows
										tasks={tasks}
										visibleTasks={visibleTasks}
										rowIndex={rowIndex}
										range={range}
										dayWidth={dayWidth}
										rowHeight={rowHeight}
										hoveredId={hoveredId}
										displayTask={displayTask}
									/>
								)}

								{linkLine && (
									<svg
										className="pointer-events-none absolute inset-0 z-40 h-full w-full overflow-visible"
										role="img"
										aria-hidden="true"
									>
										<title>{t('newDependency', 'New dependency')}</title>
										<path
											d={`M ${linkLine.x1} ${linkLine.y1} C ${linkLine.x1 + 24} ${linkLine.y1}, ${linkLine.x2 - 24} ${linkLine.y2}, ${linkLine.x2} ${linkLine.y2}`}
											className="stroke-primary"
											strokeWidth={1.5}
											strokeDasharray="4 3"
											fill="none"
										/>
										<circle
											cx={linkLine.x2}
											cy={linkLine.y2}
											r={3}
											className="fill-primary"
										/>
									</svg>
								)}

								{visibleTasks.map((task, i) => (
									<GanttTaskBar
										key={task.id}
										task={task}
										shown={displayTask(task)}
										row={i}
										rowHeight={rowHeight}
										dayWidth={dayWidth}
										range={range}
										draggable={draggable}
										resizable={resizable}
										linkable={editable && showDependencies}
										linkTarget={
											drag?.type === "link" && drag.targetId === task.id
										}
										showProgress={showProgress}
										menuGroups={menuGroupsFor(task)}
										onStartDrag={startBarDrag}
										onStartLink={startLinkDrag}
										onClick={handleBarClick}
										onDoubleClick={handleBarDoubleClick}
										onHover={(over) => setHoveredId(over ? task.id : null)}
									/>
								))}
							</div>
						</div>
					</div>
				</div>
			)}

			{chip && (
				<div
					className="pointer-events-none fixed z-50 rounded-md border border-border bg-popover px-2 py-1 text-[10px] font-medium text-popover-foreground shadow-md"
					style={{ left: chip.x + 12, top: chip.y - 30 }}
				>
					{chip.label}
				</div>
			)}

			<TaskDialog
				state={dialogState}
				tasks={tasks}
				editable={editable}
				onClose={() => setDialogState(null)}
				onSave={onDialogSave}
				onDelete={deleteTask}
			/>
		</div>
	);
}

// ── View switcher ───────────────────────────────────────────────────

function ViewSwitcher({
	view,
	onChange,
}: {
	view: GanttView;
	onChange: (view: GanttView) => void;
}) {
	return (
		<div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
			{VIEWS.map((v) => (
				<button
					key={v}
					type="button"
					onClick={() => onChange(v)}
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
	);
}

// ── Timeline header (month tier + day/week ticks) ───────────────────

interface MonthSegment {
	key: string;
	left: number;
	width: number;
	labelLong: string;
	labelShort: string;
}

function TimelineHeader({
	segments,
	days,
	dayWidth,
}: {
	segments: MonthSegment[];
	days: Date[];
	dayWidth: number;
}) {
	return (
		<div
			className="sticky top-0 z-40 border-b border-border bg-card"
			style={{ height: HEADER_HEIGHT }}
		>
			<div
				className="relative border-b border-border/60"
				style={{ height: HEADER_TIER1 }}
			>
				{segments.map((seg) => (
					<div
						key={seg.key}
						className="absolute top-0 flex h-full items-center truncate border-r border-border/60 px-1.5 text-[10px] font-medium text-muted-foreground"
						style={{ left: seg.left, width: seg.width }}
					>
						{seg.width >= 90 ? seg.labelLong : seg.labelShort}
					</div>
				))}
			</div>
			<div className="relative" style={{ height: HEADER_TIER2 }}>
				{dayWidth >= 24 &&
					days.map((d, i) => (
						<div
							key={d.toISOString()}
							className="absolute top-0 flex h-full flex-col items-center justify-center gap-px leading-none"
							style={{ left: i * dayWidth, width: dayWidth }}
						>
							<span className="text-[9px] text-muted-foreground/70">
								{d.toLocaleDateString(undefined, { weekday: "narrow" })}
							</span>
							<span className="text-[10px] text-muted-foreground">
								{d.getDate()}
							</span>
						</div>
					))}
				{dayWidth < 24 &&
					dayWidth >= 3 &&
					days.map((d, i) =>
						d.getDay() === 1 ? (
							<span
								key={d.toISOString()}
								className="absolute top-1/2 -translate-y-1/2 text-[9px] text-muted-foreground"
								style={{ left: i * dayWidth + 2 }}
							>
								{d.getDate()}
							</span>
						) : null,
					)}
			</div>
		</div>
	);
}

// ── Task list row ───────────────────────────────────────────────────

function TaskListRow({
	task,
	index,
	depth,
	rowHeight,
	editable,
	extraColumns,
	hovered,
	parent,
	isCollapsed,
	renameValue,
	menuGroups,
	onToggleCollapse,
	onOpen,
	onRenameStart,
	onRenameChange,
	onRenameCommit,
	onRenameCancel,
	onDelete,
	onHover,
}: {
	task: GanttTask;
	index: number;
	depth: number;
	rowHeight: number;
	editable: boolean;
	extraColumns: string[];
	hovered: boolean;
	parent: boolean;
	isCollapsed: boolean;
	renameValue: string | null;
	menuGroups: PlanningMenuAction[][];
	onToggleCollapse: () => void;
	onOpen: () => void;
	onRenameStart: () => void;
	onRenameChange: (value: string) => void;
	onRenameCommit: () => void;
	onRenameCancel: () => void;
	onDelete: () => void;
	onHover: (over: boolean) => void;
}) {
	const { t } = useTranslation("common");
	const clickTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	useEffect(
		() => () => {
			if (clickTimer.current) clearTimeout(clickTimer.current);
		},
		[],
	);

	// Delay single-click open so a double-click can win and start a rename.
	const handleNameClick = useCallback(() => {
		if (!editable) {
			onOpen();
			return;
		}
		if (clickTimer.current) clearTimeout(clickTimer.current);
		clickTimer.current = setTimeout(() => {
			clickTimer.current = null;
			onOpen();
		}, 200);
	}, [editable, onOpen]);

	const handleNameDoubleClick = useCallback(() => {
		if (!editable) return;
		if (clickTimer.current) {
			clearTimeout(clickTimer.current);
			clickTimer.current = null;
		}
		onRenameStart();
	}, [editable, onRenameStart]);

	const renaming = renameValue !== null;
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({ id: task.id, disabled: !editable || renaming });

	return (
		<PlanningContextMenu groups={menuGroups}>
			<div
				ref={setNodeRef}
				className={cn(
					"group/row relative flex items-center gap-1 border-b border-border/30 pr-1 text-xs",
					index % 2 === 1 && "bg-muted/20",
					hovered && "bg-accent/30",
					isDragging && "z-10 bg-accent/50 opacity-70 shadow-sm",
				)}
				style={{
					height: rowHeight,
					paddingLeft: 8 + depth * 12,
					transform: transform ? CSS.Transform.toString(transform) : undefined,
					transition,
				}}
				onMouseEnter={() => onHover(true)}
				onMouseLeave={() => onHover(false)}
			>
				{editable && (
					<button
						type="button"
						aria-label={t('dragToReorder', 'Drag to reorder')}
						className="shrink-0 cursor-grab touch-none opacity-0 transition-opacity active:cursor-grabbing group-hover/row:opacity-100"
						{...attributes}
						{...listeners}
						onClick={(e) => e.stopPropagation()}
					>
						<GripVerticalIcon className="h-3.5 w-3.5 text-muted-foreground/70" />
					</button>
				)}
				{parent ? (
					<button
						type="button"
						onClick={onToggleCollapse}
						className="shrink-0 text-muted-foreground"
						aria-label={isCollapsed ? "Expand" : "Collapse"}
					>
						{isCollapsed ? (
							<ChevronRightIcon className="h-3.5 w-3.5" />
						) : (
							<ChevronDownIcon className="h-3.5 w-3.5" />
						)}
					</button>
				) : (
					<span className="w-3.5 shrink-0" />
				)}
				<span
					className="h-2 w-2 shrink-0 rounded-full bg-primary"
					style={task.color ? { backgroundColor: task.color } : undefined}
				/>
				{renaming ? (
					<Input
						value={renameValue}
						autoFocus
						onChange={(e) => onRenameChange(e.target.value)}
						onBlur={onRenameCommit}
						onKeyDown={(e) => {
							if (e.key === "Enter") onRenameCommit();
							else if (e.key === "Escape") onRenameCancel();
						}}
						onClick={(e) => e.stopPropagation()}
						className="h-5 min-w-0 flex-1 rounded-none border-0 bg-transparent px-0 text-xs shadow-none focus-visible:ring-0"
					/>
				) : (
					<button
						type="button"
						onClick={handleNameClick}
						onDoubleClick={handleNameDoubleClick}
						className="min-w-0 flex-1 truncate text-left hover:text-primary"
					>
						{task.name}
					</button>
				)}
				{extraColumns.map((c) =>
					c === "assignee" && task.assignee ? (
						<span
							key={c}
							className="flex w-14 justify-end text-muted-foreground"
						>
							<AssigneeDisplay value={task.assignee} className="text-[11px]" />
						</span>
					) : (
						<span
							key={c}
							className="w-14 truncate text-right text-muted-foreground"
						>
							{formatColumn(task, c)}
						</span>
					),
				)}
				{editable && (
					<button
						type="button"
						onClick={onDelete}
						className="hidden shrink-0 rounded p-0.5 text-muted-foreground hover:bg-accent group-hover/row:block"
						aria-label={t('deleteTask', 'Delete task')}
					>
						<XIcon className="h-3 w-3" />
					</button>
				)}
			</div>
		</PlanningContextMenu>
	);
}

// ── Bars & milestones ───────────────────────────────────────────────

interface GanttRangeLike {
	start: Date;
	end: Date;
	totalDays: number;
}

function GanttTaskBar({
	task,
	shown,
	row,
	rowHeight,
	dayWidth,
	range,
	draggable,
	resizable,
	linkable,
	linkTarget,
	showProgress,
	menuGroups,
	onStartDrag,
	onStartLink,
	onClick,
	onDoubleClick,
	onHover,
}: {
	task: GanttTask;
	shown: GanttTask;
	row: number;
	rowHeight: number;
	dayWidth: number;
	range: GanttRangeLike;
	draggable: boolean;
	resizable: boolean;
	linkable: boolean;
	linkTarget: boolean;
	showProgress: boolean;
	menuGroups: PlanningMenuAction[][];
	onStartDrag: (
		task: GanttTask,
		kind: "move" | "resize-start" | "resize-end",
		e: React.PointerEvent,
	) => void;
	onStartLink: (task: GanttTask, e: React.PointerEvent) => void;
	onClick: (task: GanttTask) => void;
	onDoubleClick: (task: GanttTask) => void;
	onHover: (over: boolean) => void;
}) {
	const { t } = useTranslation("common");
	const [hovered, setHovered] = useState(false);
	const geom = taskBarDays(shown, range);
	const top = row * rowHeight;
	const left = geom.offsetDays * dayWidth;
	const width = Math.max(dayWidth, geom.spanDays * dayWidth);
	const color = task.color;

	const enter = useCallback(() => {
		setHovered(true);
		onHover(true);
	}, [onHover]);
	const leave = useCallback(() => {
		setHovered(false);
		onHover(false);
	}, [onHover]);

	const keyOpen = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter" || e.key === " ") {
				e.preventDefault();
				onClick(task);
			}
		},
		[onClick, task],
	);

	if (task.milestone) {
		return (
			<PlanningContextMenu groups={menuGroups}>
				<div
					data-task-id={task.id}
					className={cn(
						"group/ms absolute z-30 flex items-center gap-1.5 outline-none",
						draggable ? "cursor-grab active:cursor-grabbing" : "cursor-pointer",
					)}
					style={{ top, left: left - 6, height: rowHeight }}
					onPointerEnter={enter}
					onPointerLeave={leave}
					onPointerDown={
						draggable ? (e) => onStartDrag(task, "move", e) : undefined
					}
					onClick={(e) => {
						e.stopPropagation();
						onClick(task);
					}}
					onDoubleClick={(e) => {
						e.stopPropagation();
						onDoubleClick(task);
					}}
					onKeyDown={keyOpen}
				>
					<div
						className={cn(
							"h-3 w-3 rotate-45 rounded-[2px] bg-primary transition-transform group-hover/ms:scale-110",
							linkTarget && "scale-125 ring-2 ring-primary ring-offset-1",
						)}
						style={color ? { backgroundColor: color } : undefined}
					/>
					<span className="whitespace-nowrap text-[11px] text-muted-foreground">
						{shown.name}
					</span>
				</div>
			</PlanningContextMenu>
		);
	}

	const barHeight = rowHeight - 10;
	const labelInside = width >= 56;
	const progress =
		showProgress && shown.progress != null
			? Math.max(0, Math.min(100, shown.progress))
			: null;
	const showPercent = progress != null && width >= 110;

	return (
		<>
			<PlanningContextMenu groups={menuGroups}>
				<div
					data-task-id={task.id}
					className={cn(
						"group/bar absolute z-30 flex items-center rounded-md px-1.5 outline-none transition-[filter,box-shadow] hover:brightness-110 focus-visible:ring-1 focus-visible:ring-ring",
						!color &&
							"border border-primary/50 bg-primary/15 hover:ring-1 hover:ring-primary/50",
						linkTarget && "ring-2 ring-primary",
						draggable ? "cursor-grab active:cursor-grabbing" : "cursor-pointer",
					)}
					style={{
						top: top + 5,
						left,
						width,
						height: barHeight,
						...(color
							? {
									border: `1px solid ${color}`,
									backgroundColor: planningTint(color),
								}
							: {}),
						...(color && hovered ? { boxShadow: `0 0 0 1px ${color}` } : {}),
					}}
					onPointerEnter={enter}
					onPointerLeave={leave}
					onPointerDown={
						draggable ? (e) => onStartDrag(task, "move", e) : undefined
					}
					onClick={(e) => {
						e.stopPropagation();
						onClick(task);
					}}
					onDoubleClick={(e) => {
						e.stopPropagation();
						onDoubleClick(task);
					}}
					onKeyDown={keyOpen}
				>
					{progress != null && (
						<div
							className={cn(
								"absolute inset-y-0 left-0 rounded-md",
								!color && "bg-primary/40",
							)}
							style={{
								width: `${progress}%`,
								...(color ? { backgroundColor: planningTint(color, 55) } : {}),
							}}
						/>
					)}
					{labelInside && (
						<span className="relative z-10 min-w-0 flex-1 truncate text-[11px] font-medium">
							{shown.name}
						</span>
					)}
					{showPercent && (
						<span className="relative z-10 ml-auto shrink-0 pl-1 text-[10px] tabular-nums text-muted-foreground">
							{Math.round(progress)}%
						</span>
					)}
					{resizable && (
						<>
							<div
								onPointerDown={(e) => onStartDrag(task, "resize-start", e)}
								className="absolute inset-y-0 left-0 z-20 flex w-1.5 cursor-ew-resize items-center justify-center"
							>
								<div
									className={cn(
										"h-3 w-0.75 rounded-full opacity-0 transition-opacity group-hover/bar:opacity-100",
										!color && "bg-primary/60",
									)}
									style={
										color
											? { backgroundColor: planningTint(color, 60) }
											: undefined
									}
								/>
							</div>
							<div
								onPointerDown={(e) => onStartDrag(task, "resize-end", e)}
								className="absolute inset-y-0 right-0 z-20 flex w-1.5 cursor-ew-resize items-center justify-center"
							>
								<div
									className={cn(
										"h-3 w-0.75 rounded-full opacity-0 transition-opacity group-hover/bar:opacity-100",
										!color && "bg-primary/60",
									)}
									style={
										color
											? { backgroundColor: planningTint(color, 60) }
											: undefined
									}
								/>
							</div>
						</>
					)}
					{linkable && (
						<div
							onPointerDown={(e) => onStartLink(task, e)}
							className="absolute -right-1.5 top-1/2 z-30 h-3 w-3 -translate-y-1/2 cursor-crosshair rounded-full border border-primary bg-background opacity-0 transition-opacity group-hover/bar:opacity-100"
							title={t('dragToLinkADependency', 'Drag to link a dependency')}
						/>
					)}
				</div>
			</PlanningContextMenu>
			{!labelInside && (
				<span
					className="pointer-events-none absolute z-30 flex items-center whitespace-nowrap text-[11px] text-muted-foreground"
					style={{ top: top + 5, left: left + width + 6, height: barHeight }}
				>
					{shown.name}
				</span>
			)}
		</>
	);
}

// ── Dependency arrows ───────────────────────────────────────────────

function DependencyArrows({
	tasks,
	visibleTasks,
	rowIndex,
	range,
	dayWidth,
	rowHeight,
	hoveredId,
	displayTask,
}: {
	tasks: GanttTask[];
	visibleTasks: GanttTask[];
	rowIndex: Map<string, number>;
	range: GanttRangeLike;
	dayWidth: number;
	rowHeight: number;
	hoveredId: string | null;
	displayTask: (task: GanttTask) => GanttTask;
}) {
	const { t } = useTranslation("common");
	const taskMap = new Map(tasks.map((t) => [t.id, t]));
	const edges: { from: GanttTask; to: GanttTask; active: boolean }[] = [];
	for (const task of visibleTasks) {
		for (const depId of task.dependencies ?? []) {
			const from = taskMap.get(depId);
			if (!from || !rowIndex.has(depId) || !rowIndex.has(task.id)) continue;
			edges.push({
				from,
				to: task,
				active: hoveredId === depId || hoveredId === task.id,
			});
		}
	}
	// Highlighted paths render last so they sit on top.
	edges.sort((a, b) => Number(a.active) - Number(b.active));

	return (
		<svg
			className="pointer-events-none absolute inset-0 z-20 h-full w-full overflow-visible"
			role="img"
			aria-hidden="true"
		>
			<title>{t('taskDependencies', 'Task dependencies')}</title>
			{edges.map(({ from, to, active }) => {
				const fromGeom = taskBarDays(displayTask(from), range);
				const toGeom = taskBarDays(displayTask(to), range);
				const x1 =
					(from.milestone
						? fromGeom.offsetDays
						: fromGeom.offsetDays + fromGeom.spanDays) * dayWidth;
				const y1 = (rowIndex.get(from.id) ?? 0) * rowHeight + rowHeight / 2;
				const x2 = toGeom.offsetDays * dayWidth;
				const y2 = (rowIndex.get(to.id) ?? 0) * rowHeight + rowHeight / 2;
				return (
					<path
						key={`${from.id}-${to.id}`}
						d={`M ${x1} ${y1} C ${x1 + 20} ${y1}, ${x2 - 20} ${y2}, ${x2} ${y2}`}
						className={
							active ? "stroke-primary/70" : "stroke-muted-foreground/40"
						}
						strokeWidth={active ? 2 : 1.5}
						fill="none"
						markerEnd={
							active ? "url(#gantt-arrow-active)" : "url(#gantt-arrow)"
						}
					/>
				);
			})}
			<defs>
				<marker
					id="gantt-arrow"
					markerWidth="6"
					markerHeight="6"
					refX="5"
					refY="3"
					orient="auto"
				>
					<path d="M0,0 L6,3 L0,6 Z" className="fill-muted-foreground/40" />
				</marker>
				<marker
					id="gantt-arrow-active"
					markerWidth="6"
					markerHeight="6"
					refX="5"
					refY="3"
					orient="auto"
				>
					<path d="M0,0 L6,3 L0,6 Z" className="fill-primary/70" />
				</marker>
			</defs>
		</svg>
	);
}

// ── Empty state ─────────────────────────────────────────────────────

function GanttEmptyState({
	editable,
	onAdd,
}: {
	editable: boolean;
	onAdd: () => void;
}) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-2 py-10">
			<GanttChartIcon className="h-8 w-8 text-muted-foreground/40" />
			<p className="text-sm text-muted-foreground">{t('noTasksYet', 'No tasks yet')}</p>
			{editable && (
				<Button variant="outline" size="sm" onClick={onAdd}>
					<PlusIcon className="h-3.5 w-3.5 mr-1" /> {t('addTask', 'Add task')}
				</Button>
			)}
		</div>
	);
}

// ── Pure helpers ────────────────────────────────────────────────────

function buildMonthSegments(days: Date[], dayWidth: number): MonthSegment[] {
	const segments: MonthSegment[] = [];
	let startIndex = 0;
	// Start at 1 so `prev` is always valid; guard `cur` against the tail index
	// to avoid an out-of-bounds read at i === days.length.
	for (let i = 1; i <= days.length; i++) {
		const prev = days[i - 1];
		const cur = i < days.length ? days[i] : undefined;
		const boundary =
			i === days.length || (cur && prev.getMonth() !== cur.getMonth());
		if (boundary) {
			segments.push({
				key: `${prev.getFullYear()}-${prev.getMonth()}`,
				left: startIndex * dayWidth,
				width: (i - startIndex) * dayWidth,
				labelLong: prev.toLocaleDateString(undefined, {
					month: "long",
					year: "numeric",
				}),
				labelShort: prev.toLocaleDateString(undefined, { month: "short" }),
			});
			startIndex = i;
		}
	}
	return segments;
}

function buildWeekendBands(
	days: Date[],
	dayWidth: number,
): { left: number; width: number }[] {
	const bands: { left: number; width: number }[] = [];
	let i = 0;
	while (i < days.length) {
		const dow = days[i].getDay();
		if (dow === 6) {
			const span = i + 1 < days.length && days[i + 1].getDay() === 0 ? 2 : 1;
			bands.push({ left: i * dayWidth, width: span * dayWidth });
			i += span;
		} else if (dow === 0) {
			bands.push({ left: i * dayWidth, width: dayWidth });
			i += 1;
		} else {
			i += 1;
		}
	}
	return bands;
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
