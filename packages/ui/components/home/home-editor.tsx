"use client";

import {
	DndContext,
	type DragEndEvent,
	type DragMoveEvent,
	DragOverlay,
	type DragStartEvent,
	type KeyboardCoordinateGetter,
	KeyboardSensor,
	MeasuringStrategy,
	type Modifier,
	PointerSensor,
	closestCenter,
	pointerWithin,
	useDraggable,
	useDroppable,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	rectSortingStrategy,
	useSortable,
} from "@dnd-kit/sortable";
import {
	ArrowDown,
	ArrowLeft,
	ArrowUp,
	Check,
	Copy,
	GripVertical,
	Loader2,
	Maximize2,
	Monitor,
	MoreHorizontal,
	Pencil,
	Plus,
	Redo2,
	RotateCcw,
	Search,
	Settings2,
	Smartphone,
	Tablet,
	Trash2,
	Undo2,
	X,
} from "lucide-react";
import {
	type CSSProperties,
	Component,
	type ReactNode,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";
import { cn } from "../../lib/utils";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../ui/sheet";
import { Textarea } from "../ui/textarea";
import {
	HOME_WIDGET_PRESETS,
	createHomeWidget,
	getHomeWidgetPreset,
} from "./catalog";
import { HomeDataWidget } from "./data-widget";
import { HomeDataWidgetSettings } from "./data-widget-settings";
import {
	type HomeDragPoint,
	homeInsertionIndex,
	insertHomeWidget,
} from "./home-drag";
import {
	HOME_GRID_GAP,
	HOME_GRID_TRACK,
	HOME_ROW_HEIGHT,
	MAX_HOME_WIDGETS,
	homeGridRowSpan,
	homeWidgetAutoHeight,
	homeWidgetHeight,
	homeWidgetSpan,
	minimumHomeWidgetRows,
	moveHomeWidget,
	responsiveHomeColumns,
} from "./home-layout";
import { HomeWidgetContent } from "./home-widget-content";
import { HomeWidgetIcon } from "./home-widget-icon";
import { HomeWidgetSettings } from "./home-widget-settings";
import type {
	HomeWidgetCategory,
	HomeWidgetPreset,
	IHomeLayout,
	IHomeWidget,
} from "./types";

const ACCENTS: Record<string, string> = {
	neutral: "var(--foreground)",
	violet: "#a78bfa",
	blue: "#60a5fa",
	emerald: "#34d399",
	orange: "#fb713f",
	amber: "#fbbf24",
	rose: "#fb7185",
};
const CATEGORIES: { id: HomeWidgetCategory | "all"; name: string }[] = [
	{ id: "all", name: "All widgets" },
	{ id: "apps", name: "Apps" },
	{ id: "data", name: "Data" },
	{ id: "content", name: "Content" },
	{ id: "activity", name: "Activity" },
	{ id: "assistant", name: "FlowPilot" },
];

// Drafts stay in this browser session when a profile or route changes.
const homeDrafts = new Map<string, { layout: IHomeLayout; reset: boolean }>();

export interface HomeEditorProps {
	draftKey?: string;
	layout: IHomeLayout;
	onSave: (layout: IHomeLayout) => Promise<void>;
	onReset: () => Promise<void>;
	defaultLayout: IHomeLayout;
	sourceLabel?: string;
	admin?: boolean;
	disabled?: boolean;
	toolbar?: ReactNode;
	onEditingChange?: (editing: boolean) => void;
}

export function HomeEditor({
	draftKey,
	layout,
	onSave,
	onReset,
	defaultLayout,
	sourceLabel = "Default layout",
	admin = false,
	disabled = false,
	toolbar,
	onEditingChange,
}: HomeEditorProps) {
	const [editing, setEditing] = useState(false);
	const [hasDraft, setHasDraft] = useState(() =>
		Boolean(draftKey && homeDrafts.has(draftKey)),
	);
	const [draft, setDraft] = useState(layout);
	const [runtimeLayout, setRuntimeLayout] = useState(layout);
	const [runtimeSaving, setRuntimeSaving] = useState(false);
	const runtimeSavingRef = useRef(false);
	const [past, setPast] = useState<{ layout: IHomeLayout; reset: boolean }[]>(
		[],
	);
	const [future, setFuture] = useState<
		{ layout: IHomeLayout; reset: boolean }[]
	>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [panel, setPanel] = useState<"catalog" | "settings" | null>(null);
	const [saving, setSaving] = useState(false);
	const [confirm, setConfirm] = useState<"reset" | "discard" | null>(null);
	const [preview, setPreview] = useState<"desktop" | "tablet" | "phone">(
		"desktop",
	);
	const [resetPending, setResetPending] = useState(false);
	const [dirty, setDirty] = useState(false);
	const [announcement, setAnnouncement] = useState("");
	const editorRef = useRef<HTMLDivElement>(null);
	const pointer = useRef<HomeDragPoint | null>(null);
	const lastPlacement = useRef<{ x: number; y: number; scroll: number } | null>(
		null,
	);
	const [drag, setDrag] = useState<{
		widget: IHomeWidget;
		preset: boolean;
		widgets: IHomeWidget[];
		inside: boolean;
		snapshot: HTMLElement | null;
		width: number;
		height: number;
	} | null>(null);
	const dragRef = useRef(drag);
	const setDragState = useCallback((value: typeof drag) => {
		dragRef.current = value;
		setDrag(value);
	}, []);
	useLayoutEffect(() => {
		if (!drag?.preset || !drag.inside) return;
		const element = [
			...(editorRef.current?.querySelectorAll<HTMLElement>(
				"[data-home-widget]",
			) ?? []),
		].find((node) => node.dataset.homeWidget === drag.widget.id);
		if (!element) return;
		const refresh = () => {
			const currentDrag = dragRef.current;
			if (!currentDrag || currentDrag.widget.id !== drag.widget.id) return;
			const box = element.getBoundingClientRect();
			if (
				currentDrag.snapshot &&
				Math.abs(currentDrag.width - box.width) < 1 &&
				Math.abs(currentDrag.height - box.height) < 1
			)
				return;
			setDragState({
				...currentDrag,
				snapshot: cloneWidgetPreview(element),
				width: box.width,
				height: box.height,
			});
		};
		const observer = new ResizeObserver(refresh);
		observer.observe(element);
		refresh();
		return () => observer.disconnect();
	}, [drag?.widget.id, drag?.preset, drag?.inside, setDragState]);
	const editingChangeRef = useRef(onEditingChange);
	const current = editing ? draft : runtimeLayout;
	const selected = current.widgets.find((widget) => widget.id === selectedId);
	const keyboardCoordinates: KeyboardCoordinateGetter = (
		event,
		{ currentCoordinates },
	) => {
		if (
			!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.code)
		)
			return;
		const currentDrag = dragRef.current;
		if (!currentDrag) return;
		event.preventDefault();
		const direction =
			event.code === "ArrowLeft" || event.code === "ArrowUp" ? -1 : 1;
		const currentIndex = currentDrag.widgets.findIndex(
			(item) => item.id === currentDrag.widget.id,
		);
		const index = Math.max(
			0,
			Math.min(
				currentDrag.widgets.length - (currentIndex >= 0 ? 1 : 0),
				currentIndex < 0 ? 0 : currentIndex + direction,
			),
		);
		const widgets = insertHomeWidget(
			currentDrag.widgets,
			currentDrag.widget,
			index,
		);
		setDragState({ ...currentDrag, widgets, inside: true });
		setAnnouncement(
			`${currentDrag.widget.title ?? "Widget"}, position ${index + 1} of ${widgets.length}`,
		);
		requestAnimationFrame(() =>
			editorRef.current
				?.querySelector("[data-home-placeholder]")
				?.scrollIntoView({ block: "nearest", inline: "nearest" }),
		);
		return {
			x: currentCoordinates.x,
			y: currentCoordinates.y + direction * 24,
		};
	};
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
		useSensor(KeyboardSensor, {
			coordinateGetter: keyboardCoordinates,
		}),
	);

	useEffect(() => {
		if (!editing) setHasDraft(Boolean(draftKey && homeDrafts.has(draftKey)));
	}, [draftKey, editing]);
	useEffect(() => {
		if (!editing) setDraft(layout);
		if (!editing && !runtimeSaving) setRuntimeLayout(layout);
	}, [layout, editing, runtimeSaving]);
	useEffect(() => {
		editingChangeRef.current = onEditingChange;
	}, [onEditingChange]);
	useEffect(() => {
		editingChangeRef.current?.(editing);
	}, [editing]);
	useEffect(() => {
		if (!editing || !dirty) return;
		const protect = (event: BeforeUnloadEvent) => {
			event.preventDefault();
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", protect);
		return () => window.removeEventListener("beforeunload", protect);
	}, [editing, dirty]);

	useEffect(() => {
		if (!draftKey || !editing || !dirty) return;
		homeDrafts.set(draftKey, { layout: draft, reset: resetPending });
		setHasDraft(true);
	}, [draftKey, editing, dirty, draft, resetPending]);

	const change = useCallback(
		(next: IHomeLayout | ((value: IHomeLayout) => IHomeLayout)) => {
			if (saving) return;
			const value = typeof next === "function" ? next(draft) : next;
			if (value === draft) return;
			setPast((history) => [
				...history.slice(-49),
				{ layout: draft, reset: resetPending },
			]);
			setFuture([]);
			setDirty(true);
			setResetPending(false);
			setDraft(value);
		},
		[draft, resetPending, saving],
	);
	const update = useCallback(
		(id: string, values: Partial<IHomeWidget>) =>
			change((page) => ({
				...page,
				widgets: page.widgets.map((widget) => {
					if (widget.id !== id) return widget;
					const updated = { ...widget, ...values };
					return {
						...updated,
						size: {
							...updated.size,
							rows: Math.max(updated.size.rows, minimumHomeWidgetRows(updated)),
						},
					};
				}),
			})),
		[change],
	);
	const undo = useCallback(() => {
		if (
			!past.length ||
			saving ||
			dragRef.current ||
			editorRef.current?.querySelector("[data-home-resizing]")
		)
			return;
		const previous = past[past.length - 1];
		setFuture((history) => [
			{ layout: draft, reset: resetPending },
			...history,
		]);
		setDraft(previous.layout);
		setResetPending(previous.reset);
		setPast((history) => history.slice(0, -1));
		setDirty(true);
	}, [past, draft, resetPending, saving]);
	const redo = useCallback(() => {
		if (
			!future.length ||
			saving ||
			dragRef.current ||
			editorRef.current?.querySelector("[data-home-resizing]")
		)
			return;
		const next = future[0];
		setPast((history) => [...history, { layout: draft, reset: resetPending }]);
		setDraft(next.layout);
		setResetPending(next.reset);
		setFuture((history) => history.slice(1));
		setDirty(true);
	}, [future, draft, resetPending, saving]);
	const finishEditing = useCallback(() => {
		if (draftKey) homeDrafts.delete(draftKey);
		setHasDraft(false);
		setEditing(false);
		setDrag(null);
		dragRef.current = null;
		pointer.current = null;
		setPanel(null);
		setSelectedId(null);
		setPast([]);
		setFuture([]);
		setDirty(false);
		setResetPending(false);
		setPreview("desktop");
	}, [draftKey]);
	const save = useCallback(async () => {
		if (
			dragRef.current ||
			editorRef.current?.querySelector("[data-home-resizing]")
		)
			return;
		if (saving) return;
		if (new Blob([JSON.stringify(draft)]).size > 128 * 1024) {
			toast.error(
				"This layout is too large. Shorten its content before saving.",
			);
			return;
		}
		setSaving(true);
		try {
			if (resetPending) await onReset();
			else await onSave(draft);
			finishEditing();
			toast.success(
				admin
					? "Default home published"
					: resetPending
						? "Following the latest default"
						: "Your home is saved",
			);
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: "Could not save your home. Your changes are still here.",
			);
		} finally {
			setSaving(false);
		}
	}, [saving, draft, resetPending, onReset, onSave, finishEditing, admin]);
	useEffect(() => {
		if (!editing) return;
		const keyboard = (event: KeyboardEvent) => {
			if (saving) return;
			const target = event.target as HTMLElement;
			const input = target.closest("input, textarea, [contenteditable=true]");
			if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
				event.preventDefault();
				void save();
			}
			if (
				!input &&
				(event.metaKey || event.ctrlKey) &&
				event.key.toLowerCase() === "z"
			) {
				event.preventDefault();
				event.shiftKey ? redo() : undo();
			}
			if (event.key === "Escape" && panel && !dragRef.current) setPanel(null);
		};
		window.addEventListener("keydown", keyboard);
		return () => window.removeEventListener("keydown", keyboard);
	}, [editing, saving, undo, redo, save, panel]);
	const saveRuntimeConfig = async (
		widget: IHomeWidget,
		config: Record<string, unknown>,
	) => {
		if (admin || runtimeSavingRef.current) return;
		runtimeSavingRef.current = true;
		const previous = runtimeLayout;
		const next = {
			...previous,
			widgets: previous.widgets.map((item) =>
				item.id === widget.id ? { ...item, config } : item,
			),
		};
		setRuntimeLayout(next);
		setRuntimeSaving(true);
		try {
			await onSave(next);
		} catch {
			setRuntimeLayout(previous);
			toast.error("Could not save this change. Try again.");
		} finally {
			runtimeSavingRef.current = false;
			setRuntimeSaving(false);
		}
	};
	const begin = () => {
		const restored = draftKey ? homeDrafts.get(draftKey) : undefined;
		setDraft(structuredClone(restored?.layout ?? layout));
		setResetPending(restored?.reset ?? false);
		setDirty(Boolean(restored));
		if (restored) toast.info("Your unsaved draft is restored.");
		setEditing(true);
		setPanel("catalog");
		setPast([]);
		setFuture([]);
	};
	const add = (presetId: string, beforeId?: string) => {
		if (draft.widgets.length >= MAX_HOME_WIDGETS) {
			toast.error(`A home can contain up to ${MAX_HOME_WIDGETS} widgets.`);
			return;
		}
		const widget = createHomeWidget(presetId);
		change((page) => {
			const widgets = [...page.widgets];
			const index = beforeId
				? widgets.findIndex((item) => item.id === beforeId)
				: -1;
			widgets.splice(index < 0 ? widgets.length : index, 0, widget);
			return { ...page, widgets };
		});
		setSelectedId(widget.id);
		setPanel("settings");
		setAnnouncement(`${widget.title} added`);
	};
	const dragStart = ({ active }: DragStartEvent) => {
		lastPlacement.current = null;
		const preset = String(active.id).startsWith("preset:");
		if (preset && draft.widgets.length >= MAX_HOME_WIDGETS) {
			toast.error(`A home can contain up to ${MAX_HOME_WIDGETS} widgets.`);
			return;
		}
		const widget = preset
			? createHomeWidget(String(active.id).slice(7))
			: draft.widgets.find((item) => item.id === active.id);
		if (!widget) return;
		const element = preset
			? null
			: [
					...(editorRef.current?.querySelectorAll<HTMLElement>(
						"[data-home-widget]",
					) ?? []),
				].find((item) => item.dataset.homeWidget === widget.id);
		const box = element?.getBoundingClientRect();
		const snapshot = element ? cloneWidgetPreview(element) : null;
		setDragState({
			widget,
			preset,
			widgets: draft.widgets,
			inside: !preset,
			snapshot: snapshot ?? null,
			width: box?.width ?? 280,
			height: box?.height ?? 100,
		});
		setAnnouncement(
			`Moving ${widget.title ?? "widget"}. Use arrow keys to choose a position, Space to place, or Escape to cancel.`,
		);
	};
	const dragMove = (event: DragMoveEvent) => {
		const currentDrag = dragRef.current;
		const canvas =
			editorRef.current?.querySelector<HTMLElement>("[data-home-canvas]");
		if (!currentDrag || !canvas) return;
		let index: number | null = null;
		if (pointer.current) {
			const scroll =
				editorRef.current?.querySelector<HTMLElement>("[data-home-scroll]")
					?.scrollTop ?? 0;
			const previous = lastPlacement.current;
			if (
				previous &&
				previous.x === pointer.current.x &&
				previous.y === pointer.current.y &&
				previous.scroll === scroll
			)
				return;
			// Layout changes also trigger collision events. Only physical movement or scrolling chooses a new slot.
			lastPlacement.current = { ...pointer.current, scroll };
			const box = canvas.getBoundingClientRect();
			const rects = [
				...canvas.querySelectorAll<HTMLElement>("[data-home-widget]"),
			].map((element) => ({
				id: element.dataset.homeWidget ?? "",
				...widgetBounds(element),
			}));
			index = homeInsertionIndex(
				currentDrag.widgets,
				currentDrag.widget.id,
				pointer.current,
				box,
				rects,
			);
		} else return;
		if (index === null) {
			if (currentDrag.inside) setDragState({ ...currentDrag, inside: false });
			return;
		}
		const next = insertHomeWidget(
			currentDrag.widgets,
			currentDrag.widget,
			index,
		);
		if (
			!currentDrag.inside ||
			next.some((widget, i) => widget.id !== currentDrag.widgets[i]?.id)
		) {
			setDragState({ ...currentDrag, widgets: next, inside: true });
			setAnnouncement(
				`${currentDrag.widget.title ?? "Widget"}, position ${index + 1} of ${next.length}`,
			);
		}
	};
	const dragEnd = (_event: DragEndEvent) => {
		const currentDrag = dragRef.current;
		setDragState(null);
		pointer.current = null;
		if (!editing || saving || !currentDrag?.inside) return;
		if (
			currentDrag.widgets.some(
				(item, index) => item.id !== draft.widgets[index]?.id,
			) ||
			currentDrag.widgets.length !== draft.widgets.length
		) {
			change({ ...draft, widgets: currentDrag.widgets });
			setSelectedId(currentDrag.widget.id);
			setAnnouncement(`${currentDrag.widget.title ?? "Widget"} placed`);
		}
	};
	const duplicate = (widget: IHomeWidget) => {
		if (draft.widgets.length >= MAX_HOME_WIDGETS) return;
		const clone = { ...structuredClone(widget), id: crypto.randomUUID() };
		change((page) => {
			const widgets = [...page.widgets];
			widgets.splice(
				widgets.findIndex((item) => item.id === widget.id) + 1,
				0,
				clone,
			);
			return { ...page, widgets };
		});
		setSelectedId(clone.id);
		setPanel("settings");
	};
	const remove = (widget: IHomeWidget) => {
		change((page) => ({
			...page,
			widgets: page.widgets.filter((item) => item.id !== widget.id),
		}));
		if (selectedId === widget.id) {
			setSelectedId(null);
			setPanel("catalog");
		}
		setAnnouncement(`${widget.title ?? "Widget"} removed. Undo is available.`);
	};
	const move = (widget: IHomeWidget, offset: number) => {
		const index = draft.widgets.findIndex((item) => item.id === widget.id);
		const target = draft.widgets[index + offset];
		if (target) change((page) => moveHomeWidget(page, widget.id, target.id));
	};
	const reset = () => {
		change(structuredClone(defaultLayout));
		setResetPending(true);
		setConfirm(null);
		setSelectedId(null);
		setPanel("catalog");
	};

	return (
		<div
			ref={editorRef}
			className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background"
			data-home-editor
			data-editing={editing}
		>
			<div className="sticky top-0 z-20 flex min-h-16 shrink-0 flex-wrap items-center justify-between gap-3 border-b border-border/50 bg-background/95 px-4 py-3 backdrop-blur-xl sm:px-7">
				<div className="flex min-w-0 flex-wrap items-center gap-3">
					<span className="text-base font-semibold tracking-tight">
						{admin ? "Default home" : "Home"}
					</span>
					<span
						className={cn(
							"rounded-full border px-2.5 py-1 text-[11px] font-medium",
							editing
								? "border-primary/25 bg-primary/10 text-primary"
								: "border-border/60 text-muted-foreground",
						)}
					>
						{editing
							? dirty
								? "Unsaved changes"
								: "Editing layout"
							: runtimeSaving
								? "Saving…"
								: sourceLabel}
					</span>
					{toolbar}
				</div>
				<div className="flex flex-wrap items-center gap-1.5">
					{editing ? (
						<>
							<div className="mr-1 hidden items-center rounded-lg border border-border/60 p-0.5 sm:flex">
								{(
									[
										{ id: "desktop", icon: Monitor },
										{ id: "tablet", icon: Tablet },
										{ id: "phone", icon: Smartphone },
									] as const
								).map(({ id, icon: Icon }) => (
									<Button
										key={id}
										size="icon"
										variant={preview === id ? "secondary" : "ghost"}
										className="h-7 w-8"
										aria-label={`Preview ${id} layout`}
										aria-pressed={preview === id}
										onClick={() => setPreview(id)}
									>
										<Icon className="h-3.5 w-3.5" />
									</Button>
								))}
							</div>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8"
								aria-label="Undo layout change"
								disabled={!past.length || saving}
								onClick={undo}
							>
								<Undo2 className="h-4 w-4" />
							</Button>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8"
								aria-label="Redo layout change"
								disabled={!future.length || saving}
								onClick={redo}
							>
								<Redo2 className="h-4 w-4" />
							</Button>
							<Button
								variant="outline"
								size="sm"
								onClick={() => setPanel(panel === "catalog" ? null : "catalog")}
								disabled={saving}
							>
								<Plus className="h-4 w-4" />
								<span className="hidden min-[400px]:inline">Add widget</span>
							</Button>
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										className="h-8 w-8"
										aria-label="Layout options"
									>
										<MoreHorizontal className="h-4 w-4" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end">
									<DropdownMenuItem
										onSelect={() => setConfirm("reset")}
										disabled={saving}
									>
										<RotateCcw className="h-4 w-4" />
										{admin ? "Use inherited default" : "Reset to default"}
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
							<Button
								variant="ghost"
								size="sm"
								disabled={saving}
								onClick={() =>
									dirty ? setConfirm("discard") : finishEditing()
								}
							>
								Cancel
							</Button>
							<Button
								size="sm"
								onClick={() => void save()}
								disabled={saving || disabled}
							>
								{saving ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									<Check className="h-4 w-4" />
								)}
								{admin ? "Publish" : "Save"}
							</Button>
						</>
					) : (
						<Button
							variant="outline"
							size="sm"
							onClick={begin}
							disabled={disabled || runtimeSaving}
						>
							<Pencil className="h-3.5 w-3.5" />
							{hasDraft
								? "Resume editing"
								: admin
									? "Edit default"
									: "Customize"}
						</Button>
					)}
				</div>
			</div>
			<DndContext
				sensors={sensors}
				measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
				collisionDetection={(args) => {
					pointer.current = args.pointerCoordinates;
					if (args.pointerCoordinates) {
						const hits = pointerWithin(args);
						const widget = hits.find((hit) => hit.id !== "home-canvas");
						return widget ? [widget] : hits;
					}
					return closestCenter({
						...args,
						droppableContainers: args.droppableContainers.filter(
							(item) => item.id !== "home-canvas" && item.id !== args.active.id,
						),
					});
				}}
				onDragStart={dragStart}
				onDragMove={dragMove}
				onDragOver={dragMove}
				onDragCancel={() => {
					setDragState(null);
					pointer.current = null;
					setAnnouncement("Move cancelled");
				}}
				onDragEnd={dragEnd}
			>
				<div
					className="relative flex min-h-0 flex-1"
					inert={saving || runtimeSaving}
				>
					<div
						data-home-scroll
						style={{ overflowAnchor: "none" }}
						className={cn(
							"min-w-0 flex-1 overflow-y-auto overscroll-contain px-4 py-6 sm:px-7 sm:py-8",
							editing && "bg-muted/20",
						)}
					>
						<div
							className={cn(
								"mx-auto transition-[max-width] duration-200",
								preview === "phone"
									? "max-w-[390px]"
									: preview === "tablet"
										? "max-w-[768px]"
										: "max-w-[1600px]",
							)}
						>
							{editing && (
								<div className="mb-5 flex items-center justify-between gap-3 text-xs text-muted-foreground">
									<span>Drag to arrange. Resize to make room.</span>
									<span className="tabular-nums">
										{current.widgets.length} widgets
									</span>
								</div>
							)}
							<HomeCanvas
								widgets={drag?.widgets ?? current.widgets}
								draggedId={drag?.widget.id}
								dropInside={drag?.inside ?? false}
								editing={editing}
								selectedId={selectedId}
								onSelect={(widget) => {
									setSelectedId(widget.id);
									setPanel("settings");
								}}
								onResize={(widget, size) => update(widget.id, { size })}
								onDuplicate={duplicate}
								onRemove={remove}
								onMove={move}
								onAdd={() => {
									if (!editing) begin();
									else setPanel("catalog");
								}}
								onConfigChange={
									admin && !editing
										? undefined
										: (widget, config) => {
												if (editing) update(widget.id, { config });
												else void saveRuntimeConfig(widget, config);
											}
								}
							/>
						</div>
					</div>
					{editing && panel && (
						<HomeWidgetPanel
							title={panel === "catalog" ? "Widget catalog" : "Widget settings"}
							onClose={() => setPanel(null)}
						>
							<div className="flex h-16 shrink-0 items-center justify-between border-b border-border/50 px-5">
								<div className="flex items-center gap-2">
									{panel === "settings" && (
										<Button
											variant="ghost"
											size="icon"
											className="-ml-2 h-7 w-7"
											onClick={() => setPanel("catalog")}
											aria-label="Back to widget catalog"
										>
											<ArrowLeft className="h-4 w-4" />
										</Button>
									)}
									<h2 className="text-sm font-semibold">
										{panel === "catalog"
											? "Make it your home"
											: "Widget settings"}
									</h2>
								</div>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7"
									aria-label="Close widget panel"
									onClick={() => setPanel(null)}
								>
									<X className="h-4 w-4" />
								</Button>
							</div>
							{panel === "catalog" ? (
								<WidgetCatalog onAdd={add} />
							) : selected ? (
								<WidgetInspector
									widget={selected}
									onChange={(values) => update(selected.id, values)}
									onRemove={() => remove(selected)}
								/>
							) : (
								<div className="p-5 text-sm text-muted-foreground">
									Select a widget to configure it.
								</div>
							)}
						</HomeWidgetPanel>
					)}
				</div>
				{typeof document !== "undefined" &&
					createPortal(
						<DragOverlay
							style={{ pointerEvents: "none" }}
							zIndex={80}
							adjustScale={false}
							dropAnimation={null}
							modifiers={[keepPreviewInWindow]}
						>
							{drag && <WidgetDragPreview drag={drag} />}
						</DragOverlay>,
						document.body,
					)}
			</DndContext>
			<div className="sr-only" aria-live="polite">
				{announcement}
			</div>
			<AlertDialog
				open={confirm !== null}
				onOpenChange={(open) => {
					if (!open) setConfirm(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{confirm === "reset"
								? admin
									? "Use the inherited default?"
									: "Reset your home?"
								: "Discard layout changes?"}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{confirm === "reset"
								? "This replaces your draft with the latest default. Save to apply it and follow future default updates."
								: "Your saved home will stay as it was before you started editing."}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Keep editing</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => (confirm === "reset" ? reset() : finishEditing())}
						>
							{confirm === "reset" ? "Reset draft" : "Discard changes"}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

function HomeWidgetPanel({
	title,
	onClose,
	children,
}: { title: string; onClose: () => void; children: ReactNode }) {
	const [narrow, setNarrow] = useState(
		() =>
			typeof window !== "undefined" &&
			window.matchMedia("(max-width: 1023px)").matches,
	);
	const returnFocus = useRef<HTMLElement | null>(
		typeof document !== "undefined"
			? (document.activeElement as HTMLElement)
			: null,
	);
	useEffect(() => {
		const media = window.matchMedia("(max-width: 1023px)");
		const update = () => setNarrow(media.matches);
		update();
		media.addEventListener("change", update);
		return () => media.removeEventListener("change", update);
	}, []);
	if (narrow)
		return (
			<Sheet
				open
				onOpenChange={(open) => {
					if (!open) onClose();
				}}
			>
				<SheetContent
					className="w-full max-w-[360px] gap-0 bg-background sm:max-w-[360px] [&>button]:hidden [&>div.relative]:min-h-0 [&>div.relative]:gap-0"
					onCloseAutoFocus={(event) => {
						event.preventDefault();
						returnFocus.current?.focus();
					}}
				>
					<SheetTitle className="sr-only">{title}</SheetTitle>
					<SheetDescription className="sr-only">
						Add and configure home widgets. Close this panel to return to your
						layout.
					</SheetDescription>
					{children}
				</SheetContent>
			</Sheet>
		);
	return (
		<aside
			className="flex w-[340px] shrink-0 flex-col border-l border-border/60 bg-background"
			aria-label={title}
		>
			{children}
		</aside>
	);
}

const keepPreviewInWindow: Modifier = ({
	transform,
	overlayNodeRect,
	windowRect,
}) => {
	if (!overlayNodeRect || !windowRect) return transform;
	const left = windowRect.left + 8 - overlayNodeRect.left;
	const right = windowRect.right - 8 - overlayNodeRect.right;
	const top = windowRect.top + 8 - overlayNodeRect.top;
	const bottom = windowRect.bottom - 8 - overlayNodeRect.bottom;
	return {
		...transform,
		x: Math.max(left, Math.min(right, transform.x)),
		y: Math.max(top, Math.min(bottom, transform.y)),
	};
};

function cloneWidgetPreview(element: HTMLElement) {
	const box = element.getBoundingClientRect();
	const snapshot = element.cloneNode(true) as HTMLElement;
	snapshot.removeAttribute("data-home-widget");
	snapshot.removeAttribute("data-home-placeholder");
	snapshot.removeAttribute("id");
	for (const node of snapshot.querySelectorAll("[id]"))
		node.removeAttribute("id");
	for (const node of snapshot.querySelectorAll("[data-home-drop-hint]"))
		node.remove();
	snapshot.classList.remove(
		"border-dashed",
		"border-primary/70",
		"bg-primary/5",
		"ring-1",
		"ring-primary/25",
	);
	Object.assign(snapshot.style, {
		width: `${box.width}px`,
		height: `${box.height}px`,
		transform: "none",
		margin: "0",
		opacity: "1",
	});
	return snapshot;
}

function rectValues(rect: DOMRect) {
	return {
		left: rect.left,
		top: rect.top,
		right: rect.right,
		bottom: rect.bottom,
	};
}

function widgetBounds(element: HTMLElement) {
	const bounds = rectValues(element.getBoundingClientRect());
	const transform = getComputedStyle(element).transform;
	if (transform === "none") return bounds;
	const matrix = new DOMMatrixReadOnly(transform);
	return {
		left: bounds.left - matrix.m41,
		right: bounds.right - matrix.m41,
		top: bounds.top - matrix.m42,
		bottom: bounds.bottom - matrix.m42,
	};
}

function WidgetDragPreview({
	drag,
}: {
	drag: {
		widget: IHomeWidget;
		snapshot: HTMLElement | null;
		width: number;
		height: number;
	};
}) {
	const ref = useRef<HTMLDivElement>(null);
	useLayoutEffect(() => {
		if (!ref.current || !drag.snapshot) return;
		ref.current.replaceChildren(drag.snapshot);
	}, [drag.snapshot]);
	return drag.snapshot ? (
		<div
			ref={ref}
			aria-hidden
			inert
			data-home-drag-preview
			className="pointer-events-none overflow-hidden rounded-2xl bg-background shadow-2xl ring-2 ring-primary/60"
			style={{ width: drag.width, height: drag.height, opacity: 0.94 }}
		/>
	) : (
		<div
			data-home-drag-preview
			className="flex w-64 items-center gap-3 rounded-xl border border-primary/40 bg-background p-4 shadow-2xl"
		>
			<HomeWidgetIcon
				name={getHomeWidgetPreset(drag.widget)?.icon}
				className="size-6 text-primary"
			/>
			<div className="min-w-0">
				<p className="truncate text-sm font-semibold">{drag.widget.title}</p>
				<p className="mt-1 text-xs text-muted-foreground">
					Place anywhere on your home
				</p>
			</div>
		</div>
	);
}

function HomeCanvas({
	widgets,
	draggedId,
	dropInside,
	editing,
	selectedId,
	onSelect,
	onResize,
	onDuplicate,
	onRemove,
	onMove,
	onAdd,
	onConfigChange,
}: {
	widgets: IHomeWidget[];
	draggedId?: string;
	dropInside: boolean;
	editing: boolean;
	selectedId: string | null;
	onSelect: (widget: IHomeWidget) => void;
	onResize: (widget: IHomeWidget, size: IHomeWidget["size"]) => void;
	onDuplicate: (widget: IHomeWidget) => void;
	onRemove: (widget: IHomeWidget) => void;
	onMove: (widget: IHomeWidget, offset: number) => void;
	onAdd: () => void;
	onConfigChange?: (
		widget: IHomeWidget,
		config: Record<string, unknown>,
	) => void;
}) {
	const ref = useRef<HTMLDivElement | null>(null);
	const [width, setWidth] = useState(1200);
	const { setNodeRef, isOver } = useDroppable({
		id: "home-canvas",
		disabled: !editing,
	});
	useEffect(() => {
		if (!ref.current) return;
		const observer = new ResizeObserver(([entry]) =>
			setWidth(entry.contentRect.width),
		);
		observer.observe(ref.current);
		return () => observer.disconnect();
	}, []);
	const columns = responsiveHomeColumns(width);
	const positions = useRef(new Map<string, { left: number; top: number }>());
	const order = widgets.map((widget) => widget.id).join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: Re-measure after React changes widget order.
	useLayoutEffect(() => {
		const nodes =
			ref.current?.querySelectorAll<HTMLElement>("[data-home-widget]");
		if (!nodes) return;
		const next = new Map<string, { left: number; top: number }>();
		const animate =
			editing && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		for (const node of nodes) {
			const id = node.dataset.homeWidget ?? "";
			const bounds = widgetBounds(node);
			next.set(id, { left: bounds.left, top: bounds.top });
			const before = positions.current.get(id);
			for (const animation of node.getAnimations()) animation.cancel();
			if (!animate || !before || id === draggedId) continue;
			const x = before.left - bounds.left;
			const y = before.top - bounds.top;
			if (Math.abs(x) + Math.abs(y) > 1)
				node.animate(
					[
						{ transform: `translate(${x}px, ${y}px)` },
						{ transform: "translate(0, 0)" },
					],
					{ duration: 160, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" },
				);
		}
		positions.current = next;
	}, [order, editing, draggedId]);
	return (
		<div
			ref={(node) => {
				ref.current = node;
				setNodeRef(node);
			}}
			className={cn(
				"min-h-40 rounded-xl",
				isOver && editing && "ring-1 ring-primary/20",
			)}
			data-home-canvas
			data-grid-columns={columns}
		>
			<SortableContext
				items={widgets.map((widget) => widget.id)}
				strategy={rectSortingStrategy}
			>
				<div
					className="grid items-stretch"
					style={{
						gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
						gridAutoRows: `${HOME_GRID_TRACK}px`,
						gap: `${HOME_GRID_GAP}px`,
					}}
				>
					{widgets.map((widget, index) => (
						<HomeWidgetFrame
							key={widget.id}
							widget={widget}
							placeholder={widget.id === draggedId}
							activeDrop={widget.id === draggedId && dropInside}
							editing={editing}
							selected={selectedId === widget.id}
							columns={columns}
							canvasWidth={width}
							first={index === 0}
							last={index === widgets.length - 1}
							onSelect={() => onSelect(widget)}
							onResize={(size) => onResize(widget, size)}
							onDuplicate={() => onDuplicate(widget)}
							onRemove={() => onRemove(widget)}
							onMove={(offset) => onMove(widget, offset)}
							onConfigChange={
								onConfigChange
									? (config) => onConfigChange(widget, config)
									: undefined
							}
						/>
					))}
				</div>
			</SortableContext>
			{(editing || !widgets.length) && (
				<button
					type="button"
					className="mt-5 flex min-h-28 w-full flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-border bg-background/40 px-4 py-6 text-sm text-muted-foreground transition-colors hover:border-primary/50 hover:bg-primary/5 hover:text-foreground"
					onClick={onAdd}
				>
					<span className="flex h-9 w-9 items-center justify-center rounded-full bg-muted">
						<Plus className="h-5 w-5" />
					</span>
					{widgets.length
						? "Add a widget or drop one here"
						: "Your space, your way. Add your first widget."}
				</button>
			)}
		</div>
	);
}

function HomeWidgetFrame({
	widget,
	placeholder,
	activeDrop,
	editing,
	selected,
	columns,
	canvasWidth,
	first,
	last,
	onSelect,
	onResize,
	onDuplicate,
	onRemove,
	onMove,
	onConfigChange,
}: {
	widget: IHomeWidget;
	placeholder: boolean;
	activeDrop: boolean;
	editing: boolean;
	selected: boolean;
	columns: number;
	canvasWidth: number;
	first: boolean;
	last: boolean;
	onSelect: () => void;
	onResize: (size: IHomeWidget["size"]) => void;
	onDuplicate: () => void;
	onRemove: () => void;
	onMove: (offset: number) => void;
	onConfigChange?: (config: Record<string, unknown>) => void;
}) {
	const { attributes, listeners, setNodeRef } = useSortable({
		id: widget.id,
		disabled: !editing,
	});
	const [resize, setResize] = useState<IHomeWidget["size"] | null>(null);
	const resizeRef = useRef<IHomeWidget["size"] | null>(null);
	const origin = useRef<{
		x: number;
		y: number;
		size: IHomeWidget["size"];
		height: number;
	} | null>(null);
	const frameRef = useRef<HTMLElement | null>(null);
	const contentRef = useRef<HTMLDivElement>(null);
	const [contentHeight, setContentHeight] = useState(120);
	const [dataState, setDataState] = useState("");
	useEffect(() => {
		const cancel = (event: KeyboardEvent) => {
			if (event.key !== "Escape" || !origin.current) return;
			event.preventDefault();
			event.stopPropagation();
			origin.current = null;
			resizeRef.current = null;
			setResize(null);
		};
		window.addEventListener("keydown", cancel, true);
		return () => window.removeEventListener("keydown", cancel, true);
	}, []);
	const size = resize ?? widget.size;
	const autoHeight = homeWidgetAutoHeight({ ...widget, size });
	useLayoutEffect(() => {
		const content = contentRef.current;
		if (!content) return;
		const measure = () =>
			setContentHeight(Math.ceil(content.getBoundingClientRect().height) + 2);
		const readDataState = () =>
			setDataState(
				content.querySelector<HTMLElement>("[data-home-data-state]")?.dataset
					.homeDataState ?? "",
			);
		const observer = new ResizeObserver(measure);
		observer.observe(content);
		const mutations = new MutationObserver(readDataState);
		mutations.observe(content, {
			subtree: true,
			childList: true,
			attributes: true,
			attributeFilter: ["data-home-data-state"],
		});
		readDataState();
		measure();
		return () => {
			observer.disconnect();
			mutations.disconnect();
		};
	}, []);
	const metric = ["stat", "metricstrip", "progress", "bullet"].includes(
		String(widget.config.visualization),
	);
	const bodyHeight =
		autoHeight && widget.type === "data" && dataState === "ready" && !metric
			? widget.config.visualization === "gauge"
				? 220
				: widget.config.visualization === "calendar"
					? 240
					: 280
			: undefined;
	const embedHeight =
		autoHeight && widget.type === "app-embed" && widget.config.appId
			? Math.max(360, homeWidgetHeight(widget))
			: undefined;
	const minHeight =
		widget.type === "app-embed"
			? 240
			: widget.type === "data" && !metric
				? 220
				: 96;
	const height = autoHeight
		? Math.max(56, contentHeight)
		: Math.max(minHeight, homeWidgetHeight({ ...widget, size }));
	const resizedSize = (
		nextColumns: number,
		nextHeight: number,
	): IHomeWidget["size"] => {
		const height = Math.max(
			minHeight,
			Math.min(1240, Math.round(nextHeight / 8) * 8),
		);
		return {
			columns: Math.max(1, Math.min(12, nextColumns)),
			rows: Math.max(
				1,
				Math.min(
					12,
					Math.ceil(
						(height + HOME_GRID_GAP) / (HOME_ROW_HEIGHT + HOME_GRID_GAP),
					),
				),
			),
			heightMode: "fixed",
			height,
		};
	};
	const preset = getHomeWidgetPreset(widget);
	const ownHeader = [
		"greeting",
		"flowpilot",
		"app-embed",
		"quick-actions",
	].includes(widget.type);
	const accent = ACCENTS[widget.appearance.accent] ?? ACCENTS.neutral;
	const style: CSSProperties = {
		gridColumn: `span ${homeWidgetSpan(size.columns, columns)}`,
		gridRow: `span ${homeGridRowSpan(height)}`,
		height: autoHeight ? undefined : height,
		position: "relative",
		"--home-accent": accent,
	} as CSSProperties;
	if (widget.appearance.variant === "tinted")
		style.backgroundColor = `color-mix(in srgb, ${accent} 9%, var(--card))`;
	const borderless = widget.appearance.variant === "borderless";
	return (
		<section
			ref={(node) => {
				frameRef.current = node;
				setNodeRef(node);
			}}
			style={style}
			data-home-widget={widget.id}
			data-widget-type={widget.type}
			data-height-mode={autoHeight ? "auto" : "fixed"}
			data-home-resizing={resize ? "true" : undefined}
			data-home-placeholder={
				placeholder ? (activeDrop ? "active" : "outside") : undefined
			}
			className={cn(
				"group/widget relative flex min-h-0 min-w-0 flex-col overflow-hidden rounded-2xl",
				borderless && !editing && autoHeight && "overflow-visible rounded-none",
				borderless
					? "bg-transparent"
					: "border border-border/60 bg-card/70 shadow-sm shadow-black/[0.02]",
				editing && "border border-border/70 bg-card/80",
				selected &&
					editing &&
					"ring-2 ring-primary ring-offset-2 ring-offset-background",
				placeholder &&
					"border-dashed border-primary/70 bg-primary/5 ring-1 ring-primary/25",
			)}
		>
			{editing && !placeholder && (
				<div
					className={cn(
						"absolute top-2 inset-x-2 z-20 flex h-8 items-center justify-between gap-2 rounded-lg border border-border/70 bg-background/95 px-2 shadow-sm transition-opacity group-hover/widget:opacity-100 group-focus-within/widget:opacity-100",
						selected ? "opacity-100" : "opacity-0",
					)}
				>
					<button
						type="button"
						className="flex min-w-0 flex-1 cursor-grab touch-none items-center gap-2 text-left text-xs text-muted-foreground active:cursor-grabbing"
						{...attributes}
						{...listeners}
						aria-label={`Move ${widget.title ?? preset?.name ?? "widget"}`}
					>
						<GripVertical className="h-3.5 w-3.5 shrink-0" />
						<span className="truncate">
							{widget.title || preset?.name || widget.type}
						</span>
					</button>
					<span className="text-[10px] tabular-nums text-muted-foreground">
						{size.columns}/12 ·{" "}
						{autoHeight ? "Auto" : `${Math.round(height)}px`}
					</span>
					<Button
						variant="ghost"
						size="icon"
						className="h-6 w-6"
						onClick={onSelect}
						aria-label={`Configure ${widget.title ?? "widget"}`}
					>
						<Settings2 className="h-3 w-3" />
					</Button>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-6 w-6"
								aria-label={`Options for ${widget.title ?? "widget"}`}
							>
								<MoreHorizontal className="h-3.5 w-3.5" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<DropdownMenuItem onSelect={onSelect}>
								<Settings2 className="h-4 w-4" />
								Configure
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={onDuplicate}>
								<Copy className="h-4 w-4" />
								Duplicate
							</DropdownMenuItem>
							<DropdownMenuItem disabled={first} onSelect={() => onMove(-1)}>
								<ArrowUp className="h-4 w-4" />
								Move earlier
							</DropdownMenuItem>
							<DropdownMenuItem disabled={last} onSelect={() => onMove(1)}>
								<ArrowDown className="h-4 w-4" />
								Move later
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								onSelect={onRemove}
								className="text-destructive"
							>
								<Trash2 className="h-4 w-4" />
								Remove
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
			)}
			{placeholder && (
				<div
					data-home-drop-hint
					className="pointer-events-none absolute inset-0 z-30 flex items-start justify-start rounded-2xl bg-background/65 p-3 backdrop-blur-[1px]"
				>
					<span className="rounded-full border border-primary/30 bg-background px-4 py-2 text-xs font-medium text-primary shadow-sm">
						{activeDrop ? "Drop here" : "Move onto your home"}
					</span>
				</div>
			)}
			<div
				ref={contentRef}
				className={cn(
					"min-w-0",
					(!autoHeight || embedHeight) && "flex h-full min-h-0 flex-col",
				)}
				style={{ height: embedHeight }}
			>
				{!ownHeader && (widget.title || widget.description) && (
					<header className="shrink-0 px-4 pt-4 pb-3">
						<h2 className="flex items-center gap-2 text-sm font-semibold tracking-tight">
							<HomeWidgetIcon
								name={preset?.icon}
								className="h-4 w-4 shrink-0 text-[var(--home-accent)]"
							/>
							{widget.title}
						</h2>
						{widget.description && (
							<p className="mt-1.5 text-xs leading-relaxed text-muted-foreground">
								{widget.description}
							</p>
						)}
					</header>
				)}
				<div
					className={cn(
						"relative min-h-0 min-w-0",
						(!autoHeight || embedHeight) && "flex-1 overflow-auto",
						!ownHeader && "px-4 pb-4",
						!ownHeader && !widget.title && !widget.description && "pt-4",
						ownHeader && autoHeight && !embedHeight && "overflow-hidden",
					)}
				>
					<WidgetErrorBoundary
						key={`${widget.id}:${widget.type}`}
						resetKey={JSON.stringify(widget.config)}
					>
						<div
							className={cn(
								"min-h-0 min-w-0",
								(!autoHeight || embedHeight) && "h-full",
							)}
							style={{ height: bodyHeight }}
							inert={editing}
						>
							{widget.type === "data" ? (
								<HomeDataWidget widget={widget} editing={editing} />
							) : (
								<HomeWidgetContent
									widget={widget}
									editing={editing}
									onUpdate={onConfigChange}
								/>
							)}
						</div>
					</WidgetErrorBoundary>
					{editing && (
						<button
							type="button"
							className="absolute inset-0 z-[1] cursor-grab active:cursor-grabbing"
							{...listeners}
							aria-label={`Select ${widget.title ?? "widget"}`}
							onClick={onSelect}
						/>
					)}
				</div>
			</div>
			{editing && !placeholder && (
				<button
					type="button"
					aria-label={`Resize ${widget.title ?? "widget"}`}
					title="Drag to resize. Use arrow keys for precise sizing."
					className="absolute bottom-0 right-0 z-10 flex h-7 w-7 cursor-nwse-resize touch-none items-center justify-center rounded-tl-lg bg-background/90 text-muted-foreground hover:bg-primary hover:text-primary-foreground focus-visible:outline-2 focus-visible:outline-primary"
					onPointerDown={(event) => {
						event.preventDefault();
						event.stopPropagation();
						event.currentTarget.setPointerCapture(event.pointerId);
						origin.current = {
							x: event.clientX,
							y: event.clientY,
							size: widget.size,
							height:
								frameRef.current?.getBoundingClientRect().height ?? height,
						};
						resizeRef.current = widget.size;
					}}
					onPointerMove={(event) => {
						if (!origin.current) return;
						const unit = (canvasWidth + HOME_GRID_GAP) / columns;
						const next = resizedSize(
							columns === 1
								? origin.current.size.columns
								: origin.current.size.columns +
										Math.round((event.clientX - origin.current.x) / unit) *
											(12 / columns),
							origin.current.height + event.clientY - origin.current.y,
						);
						resizeRef.current = next;
						setResize(next);
					}}
					onPointerUp={(event) => {
						if (!origin.current) return;
						event.currentTarget.releasePointerCapture(event.pointerId);
						origin.current = null;
						if (resizeRef.current) onResize(resizeRef.current);
						setResize(null);
						resizeRef.current = null;
					}}
					onPointerCancel={() => {
						origin.current = null;
						resizeRef.current = null;
						setResize(null);
					}}
					onKeyDown={(event) => {
						if (
							!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
								event.key,
							)
						)
							return;
						event.preventDefault();
						event.stopPropagation();
						onResize(
							resizedSize(
								widget.size.columns +
									(event.key === "ArrowRight"
										? 1
										: event.key === "ArrowLeft"
											? -1
											: 0),
								(frameRef.current?.getBoundingClientRect().height ?? height) +
									(event.key === "ArrowDown"
										? 16
										: event.key === "ArrowUp"
											? -16
											: 0),
							),
						);
					}}
				>
					<Maximize2 className="h-3 w-3 rotate-90" />
				</button>
			)}
		</section>
	);
}

function WidgetCatalog({ onAdd }: { onAdd: (id: string) => void }) {
	const [query, setQuery] = useState("");
	const [category, setCategory] = useState<HomeWidgetCategory | "all">("all");
	const filtered = useMemo(
		() =>
			HOME_WIDGET_PRESETS.filter(
				(preset) =>
					(category === "all" || preset.category === category) &&
					`${preset.name} ${preset.description}`
						.toLowerCase()
						.includes(query.toLowerCase().trim()),
			),
		[query, category],
	);
	return (
		<>
			<div className="space-y-4 border-b border-border/50 p-5">
				<p className="text-xs leading-relaxed text-muted-foreground">
					Apps, insights, and a little space for you. Add a widget, then make it
					your own.
				</p>
				<div className="relative">
					<Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
					<Input
						aria-label="Search widgets"
						placeholder={`Search ${HOME_WIDGET_PRESETS.length} widgets…`}
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						className="pl-9"
					/>
				</div>
				<div className="flex flex-wrap gap-1.5">
					{CATEGORIES.map((item) => (
						<button
							type="button"
							key={item.id}
							onClick={() => setCategory(item.id)}
							aria-pressed={category === item.id}
							className={cn(
								"rounded-full border px-3 py-1.5 text-xs font-medium transition-colors",
								category === item.id
									? "border-foreground bg-foreground text-background"
									: "border-border/60 text-muted-foreground hover:border-foreground/30 hover:text-foreground",
							)}
						>
							{item.name}
						</button>
					))}
				</div>
			</div>
			<div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-4">
				{filtered.map((preset) => (
					<CatalogItem
						key={preset.id}
						preset={preset}
						onAdd={() => onAdd(preset.id)}
					/>
				))}
				{!filtered.length && (
					<p className="py-10 text-center text-sm text-muted-foreground">
						No widgets match this search.
					</p>
				)}
			</div>
			<div className="border-t border-border/50 px-5 py-3 text-[11px] leading-relaxed text-muted-foreground">
				Click to add, or drag a widget onto your home.
			</div>
		</>
	);
}

function CatalogItem({
	preset,
	onAdd,
}: { preset: HomeWidgetPreset; onAdd: () => void }) {
	const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
		id: `preset:${preset.id}`,
	});
	return (
		<div
			ref={setNodeRef}
			style={{
				zIndex: isDragging ? 60 : undefined,
			}}
			className={cn(
				"group flex items-start gap-3 rounded-xl border border-border/60 bg-card p-3 transition-colors hover:border-primary/40 hover:bg-accent/50",
				isDragging && "opacity-35",
			)}
		>
			<button
				type="button"
				{...attributes}
				{...listeners}
				className="mt-1.5 cursor-grab touch-none text-muted-foreground/50 hover:text-foreground"
				aria-label={`Drag ${preset.name} to home`}
			>
				<GripVertical className="h-3.5 w-3.5" />
			</button>
			<button
				type="button"
				onClick={onAdd}
				{...listeners}
				className="flex min-w-0 flex-1 items-start gap-3 text-left"
				aria-label={`Add ${preset.name}`}
			>
				<span
					className={cn(
						"flex h-10 w-10 shrink-0 items-center justify-center rounded-lg",
						preset.category === "data"
							? "bg-blue-500/10 text-blue-500"
							: preset.category === "apps"
								? "bg-orange-500/10 text-orange-500"
								: preset.category === "assistant"
									? "bg-violet-500/10 text-violet-500"
									: "bg-muted text-muted-foreground",
					)}
				>
					<HomeWidgetIcon name={preset.icon} className="h-5 w-5" />
				</span>
				<span className="min-w-0">
					<span className="block text-xs font-semibold leading-5">
						{preset.name}
					</span>
					<span className="mt-0.5 block text-[11px] leading-4 text-muted-foreground">
						{preset.description}
					</span>
				</span>
				<Plus className="mt-1 h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100" />
			</button>
		</div>
	);
}

function WidgetInspector({
	widget,
	onChange,
	onRemove,
}: {
	widget: IHomeWidget;
	onChange: (values: Partial<IHomeWidget>) => void;
	onRemove: () => void;
}) {
	const preset = getHomeWidgetPreset(widget);
	return (
		<div className="min-h-0 flex-1 overflow-y-auto">
			<div className="space-y-5 p-5">
				<div className="flex items-center gap-3 rounded-xl bg-muted/50 p-3">
					<HomeWidgetIcon
						name={preset?.icon}
						className="h-5 w-5 text-primary"
					/>
					<div>
						<p className="text-xs font-semibold">
							{preset?.name ?? widget.type}
						</p>
						<p className="text-[11px] text-muted-foreground">
							Changes appear on your canvas.
						</p>
					</div>
				</div>
				<div className="space-y-2">
					<Label htmlFor="home-widget-title">Title</Label>
					<Input
						id="home-widget-title"
						value={widget.title ?? ""}
						onChange={(event) => onChange({ title: event.target.value })}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="home-widget-description">Description</Label>
					<Textarea
						id="home-widget-description"
						value={widget.description ?? ""}
						onChange={(event) => onChange({ description: event.target.value })}
						rows={2}
						placeholder="Add a little context"
					/>
				</div>
				<div className="grid grid-cols-2 gap-3">
					<div className="space-y-2">
						<Label htmlFor="home-widget-width">Width</Label>
						<select
							id="home-widget-width"
							className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
							value={widget.size.columns}
							onChange={(event) =>
								event.target.value !== "custom" &&
								onChange({
									size: { ...widget.size, columns: Number(event.target.value) },
								})
							}
						>
							{Array.from({ length: 12 }, (_, index) => index + 1).map(
								(size) => (
									<option key={size} value={size}>
										{size} / 12
									</option>
								),
							)}
						</select>
					</div>
					<div className="space-y-2">
						<Label htmlFor="home-widget-height">Height</Label>
						<select
							id="home-widget-height"
							className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
							value={
								homeWidgetAutoHeight(widget)
									? "auto"
									: widget.size.height
										? "custom"
										: Math.max(widget.size.rows, minimumHomeWidgetRows(widget))
							}
							onChange={(event) =>
								event.target.value !== "custom" &&
								onChange({
									size:
										event.target.value === "auto"
											? {
													...widget.size,
													heightMode: "auto",
													height: undefined,
												}
											: {
													...widget.size,
													rows: Number(event.target.value),
													heightMode: "fixed",
													height: undefined,
												},
								})
							}
						>
							<option value="auto">Fit content</option>
							{widget.size.height && (
								<option value="custom">{widget.size.height}px</option>
							)}
							{Array.from(
								{ length: 13 - minimumHomeWidgetRows(widget) },
								(_, index) => index + minimumHomeWidgetRows(widget),
							).map((size) => (
								<option key={size} value={size}>
									{size} {size === 1 ? "row" : "rows"}
								</option>
							))}
						</select>
					</div>
				</div>
				<div className="space-y-2">
					<Label htmlFor="home-widget-style">Widget surface</Label>
					<select
						id="home-widget-style"
						className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
						value={
							["card", "borderless", "tinted"].includes(
								widget.appearance.variant,
							)
								? widget.appearance.variant
								: "card"
						}
						onChange={(event) =>
							onChange({
								appearance: {
									...widget.appearance,
									variant: event.target.value,
								},
							})
						}
					>
						{["card", "borderless", "tinted"].map((variant) => (
							<option key={variant} value={variant}>
								{variant.charAt(0).toUpperCase() + variant.slice(1)}
							</option>
						))}
					</select>
				</div>
				<div className="space-y-2">
					<Label>Accent</Label>
					<div className="flex flex-wrap gap-2">
						{Object.entries(ACCENTS).map(([name, color]) => (
							<button
								type="button"
								key={name}
								aria-label={`${name} accent`}
								aria-pressed={widget.appearance.accent === name}
								onClick={() =>
									onChange({
										appearance: { ...widget.appearance, accent: name },
									})
								}
								className={cn(
									"flex h-7 w-7 items-center justify-center rounded-full border-2",
									widget.appearance.accent === name
										? "border-foreground"
										: "border-transparent",
								)}
								style={{ backgroundColor: color }}
							>
								{widget.appearance.accent === name && (
									<Check className="h-3 w-3 text-background" />
								)}
							</button>
						))}
					</div>
				</div>
			</div>
			<div className="border-t border-border/60 p-5">
				{widget.type === "data" ? (
					<HomeDataWidgetSettings
						widget={widget}
						onChange={(config) => onChange({ config })}
					/>
				) : (
					<HomeWidgetSettings
						widget={widget}
						onChange={(config) => onChange({ config })}
					/>
				)}
			</div>
			<div className="border-t border-border/60 p-5">
				<Button
					variant="outline"
					size="sm"
					className="w-full text-destructive"
					onClick={onRemove}
				>
					<Trash2 className="h-3.5 w-3.5" />
					Remove widget
				</Button>
			</div>
		</div>
	);
}

class WidgetErrorBoundary extends Component<
	{ children: ReactNode; resetKey: string },
	{ failed: boolean }
> {
	state = { failed: false };
	componentDidUpdate(previous: { resetKey: string }) {
		if (this.state.failed && previous.resetKey !== this.props.resetKey)
			this.setState({ failed: false });
	}
	static getDerivedStateFromError() {
		return { failed: true };
	}
	render() {
		return this.state.failed ? (
			<div className="flex h-full min-h-24 flex-col items-center justify-center gap-3 p-4 text-center text-sm text-muted-foreground">
				<p>This widget could not be displayed.</p>
				<Button
					variant="outline"
					size="sm"
					onClick={() => this.setState({ failed: false })}
				>
					Try again
				</Button>
			</div>
		) : (
			this.props.children
		);
	}
}
