"use client";

import { useDndContext, useDraggable, useDroppable } from "@dnd-kit/core";
import {
	SortableContext,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	Grid3x3,
	GripVertical,
	Plus,
	Search,
	Trash2,
} from "lucide-react";
import { type CSSProperties, useId } from "react";
import { categoryColor } from "../../../../lib/category-meta";
import { cn } from "../../../../lib/utils";
import { pickExploreView } from "../../../store/explore/explore-model";
import {
	BENTO_ROW_HEIGHTS,
	BENTO_SLOTS,
	type BentoSlotGeometry,
	EXPLORE_LIMITS,
	type ExploreItemRef,
	type ExploreLayoutDoc,
	type ExplorePlacement,
	type ExplorePlacementContent,
	type ExploreSlot,
	type ExploreSlotKey,
	type ExploreViewer,
	type ResolvedExplore,
} from "../../../store/explore/explore-types";
import { usePackageGradient } from "../../../store/package-card";
import { Button } from "../../../ui/button";
import { type AddHandler, AddMenu } from "./add-menu";
import {
	type AddTarget,
	type AdminT,
	LIBRARY_KINDS,
	type TraceMarkers,
	addChoicesFor,
	addTargets,
	audienceLabel,
	effectiveStatus,
	findSlot,
	kindBadgeLabel,
	layoutRows,
	placementSummary,
	refKey,
	slotAccepts,
	slotLabel,
	slotSize,
	statusLabel,
	toneLabel,
} from "./explore-admin-model";
import {
	AudienceIcons,
	FALLBACK_CHIP,
	KindIcon,
	StatusDot,
	TONE_STYLES,
} from "./explore-admin-visuals";
import type { ItemNameOf } from "./inspector-items";

export const slotDropId = (key: string) => `s:${key}`;
export const tileDragId = (id: string) => `t:${id}`;
export const rowDragId = (key: string) => `r:${key}`;
export const NEW_ROW_DROP = "s:new-row";

const CANVAS_ROWS = BENTO_ROW_HEIGHTS.map((height) =>
	Math.max(64, Math.round(height * 0.62)),
);

interface DragPayload {
	placementId?: string;
	content?: ExplorePlacementContent;
}

/** Whether the placement being dragged may land in `slot`; undefined while nothing is dragged. */
function useDropVerdict(slot: string | null): boolean | undefined {
	const { active } = useDndContext();
	const content = (active?.data.current as DragPayload | undefined)?.content;
	if (!content) return undefined;
	return slot ? slotAccepts(slot, content) : undefined;
}

function dropRing(isOver: boolean, verdict: boolean | undefined) {
	if (!isOver || verdict === undefined) return undefined;
	return verdict
		? "ring-2 ring-primary ring-offset-2 ring-offset-background"
		: "ring-2 ring-destructive/70 ring-offset-2 ring-offset-background cursor-not-allowed";
}

function TileBadge({
	kind,
	label,
	className,
}: {
	kind: ExplorePlacement["kind"];
	label: string;
	className?: string;
}) {
	return (
		<span
			className={cn(
				"inline-flex min-w-0 shrink-0 items-center gap-1.25 font-mono text-[9.5px] tracking-[0.06em] whitespace-nowrap uppercase",
				className,
			)}
		>
			<KindIcon kind={kind} className="size-3" />
			{label}
		</span>
	);
}

function DragHandle({
	id,
	placement,
	label,
}: {
	id: string;
	placement: ExplorePlacement;
	label: string;
}) {
	const { attributes, listeners, setNodeRef } = useDraggable({
		id,
		data: { placementId: placement.id, content: placement.content },
	});
	return (
		<button
			ref={setNodeRef}
			type="button"
			aria-label={label}
			title={label}
			className="absolute top-1.5 right-1.5 z-2 flex size-6 cursor-grab touch-none items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
			{...attributes}
			{...listeners}
		>
			<GripVertical className="size-3.5" />
		</button>
	);
}

function PackageWash({ id }: { id: string }) {
	const gradient = usePackageGradient(id);
	return (
		<span
			className="absolute inset-y-0 right-0 w-[46%] opacity-55 [mask-image:linear-gradient(to_left,#000,transparent)]"
			style={{ background: gradient }}
		/>
	);
}

function tileImage(
	placement: ExplorePlacement,
	refs: ReadonlyMap<string, ExploreItemRef>,
): string | null {
	const content = placement.content;
	if (content.kind === "announcement" || content.kind === "rail") return null;
	const item = placement.items[0];
	if (!item) return null;
	return (
		item.artworkUrl ?? refs.get(refKey(item.kind, item.id))?.coverUrl ?? null
	);
}

function TileDecoration({
	slot,
	placement,
	refs,
	preview,
}: {
	slot: BentoSlotGeometry;
	placement: ExplorePlacement;
	refs: ReadonlyMap<string, ExploreItemRef>;
	preview?: ResolvedExplore;
}) {
	const image = tileImage(placement, refs);
	const first = placement.items[0];
	const grid = preview ? pickExploreView(preview, "all").grid : undefined;
	return (
		<span
			aria-hidden="true"
			className="pointer-events-none absolute inset-0 overflow-hidden rounded-[9px]"
		>
			{image ? (
				<>
					<img
						src={image}
						alt=""
						className="absolute inset-0 size-full object-cover opacity-30"
					/>
					<span className="absolute inset-0 bg-linear-to-r from-card/95 to-card/35" />
				</>
			) : first?.kind === "package" ? (
				<PackageWash id={first.id} />
			) : null}
			{slot.key === "stat" && grid?.stat?.stat && (
				<span className="absolute right-2 bottom-0.5 text-[44px] leading-11 font-semibold tracking-[-0.05em] text-foreground/8">
					{grid.stat.stat.total}
				</span>
			)}
			{slot.key === "categories" && grid?.categories?.categories && (
				<span className="absolute top-8 right-2.5 grid grid-cols-[repeat(2,14px)] gap-1">
					{grid.categories.categories.slice(0, 4).map((category) => (
						<span
							key={category.appCategory}
							className="size-3.5 rounded opacity-60"
							style={{ background: categoryColor(category.appCategory) }}
						/>
					))}
				</span>
			)}
		</span>
	);
}

/** What the tile shows beyond its name (status, fallback, preview marker, audience), for screen readers. */
function MarkerDescription({
	id,
	placement,
	fallback,
	sub,
}: {
	id: string;
	placement: ExplorePlacement;
	fallback: boolean;
	sub: string;
}) {
	const { t } = useTranslation("admin");
	const audience = placement.audience.length
		? placement.audience
		: ["everyone"];
	const parts = [
		statusLabel(effectiveStatus(placement), t),
		fallback ? t("exploreFallback", "fallback") : null,
		sub,
		...audience.map((tag) => audienceLabel(tag, t)),
	];
	return (
		<span id={id} hidden>
			{parts.filter(Boolean).join(" · ")}
		</span>
	);
}

function frameClasses({
	selected,
	slotSelected,
	fallback,
}: {
	selected: boolean;
	slotSelected: boolean;
	fallback: boolean;
}) {
	if (selected) {
		return "z-1 border-[1.5px] border-primary bg-primary/6 ring-3 ring-primary/20";
	}
	if (slotSelected)
		return "border-[1.5px] border-dashed border-primary bg-card";
	if (fallback) return "border border-dashed border-blue-500/75 bg-card";
	return "border bg-card";
}

function GridTile({
	layout,
	geometry,
	slot,
	displayedId,
	chosen,
	markers,
	selectedId,
	refs,
	nameOf,
	preview,
	onSelect,
	onAdd,
}: {
	layout: ExploreLayoutDoc;
	geometry: BentoSlotGeometry;
	slot: ExploreSlot | undefined;
	displayedId: string | null | undefined;
	chosen: string | null | undefined;
	markers?: TraceMarkers;
	selectedId: string | null;
	refs: ReadonlyMap<string, ExploreItemRef>;
	nameOf: ItemNameOf;
	preview?: ResolvedExplore;
	onSelect: (id: string) => void;
	onAdd: AddHandler;
}) {
	const { t } = useTranslation("admin");
	const describedBy = useId();
	const key = geometry.key;
	const { setNodeRef, isOver } = useDroppable({
		id: slotDropId(key),
		data: { slotKey: key },
	});
	const verdict = useDropVerdict(key);
	const placements = slot?.placements ?? [];
	const style: CSSProperties = {
		gridColumn: `${geometry.col} / span ${geometry.span}`,
		gridRow: `${geometry.row} / span ${geometry.rowSpan}`,
	};
	const shown = placements.find((placement) => placement.id === displayedId);
	const placement = shown ?? placements[0];
	if (!placement) {
		const targets = slotTargets(layout, key);
		return (
			<div
				ref={setNodeRef}
				style={style}
				className={cn("min-w-0 rounded-[10px]", dropRing(isOver, verdict))}
			>
				<AddMenu layout={layout} targets={targets} onAdd={onAdd}>
					<button
						type="button"
						disabled={!targets.length}
						aria-label={t("exploreAddToSlot", "Add a placement to {{slot}}", {
							slot: slotLabel(layout, key, t),
						})}
						className="flex size-full flex-col items-center justify-center gap-1 rounded-[10px] border border-dashed bg-card/40 text-xs text-muted-foreground transition-colors hover:border-primary/55 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-60"
					>
						<Plus className="size-4" />
						<span className="truncate px-2">
							{slotLabel(layout, key, t)} · {t("exploreEmptySlot", "empty")}
						</span>
					</button>
				</AddMenu>
			</div>
		);
	}
	const selected = placement.id === selectedId;
	const slotSelected =
		!selected && placements.some((entry) => entry.id === selectedId);
	const fallback = !!markers?.fallback.has(placement.id);
	const nothing = markers !== undefined && chosen === null;
	const tone =
		placement.content.kind === "announcement" && !nothing
			? placement.content.tone
			: undefined;
	const sub = nothing
		? t("exploreNothingForAudience", "Nothing live for this audience")
		: tone && placement.content.kind === "announcement"
			? toneLabel(tone, t)
			: placementSummary(placement, t, nameOf);
	const status = effectiveStatus(placement);
	return (
		<div
			ref={setNodeRef}
			style={style}
			className={cn(
				"relative min-w-0 rounded-[10px]",
				dropRing(isOver, verdict),
			)}
		>
			<MarkerDescription
				id={describedBy}
				placement={placement}
				fallback={fallback}
				sub={sub}
			/>
			<button
				type="button"
				aria-pressed={selected}
				aria-label={t("exploreSelectTile", "Select {{name}} ({{size}})", {
					name: placement.name,
					size: slotSize(key),
				})}
				aria-describedby={describedBy}
				onClick={() => onSelect(placement.id)}
				className={cn(
					"relative block size-full min-w-0 rounded-[10px] text-left transition-colors hover:border-primary/55 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					frameClasses({ selected, slotSelected, fallback }),
					tone &&
						!selected &&
						!slotSelected &&
						!fallback &&
						TONE_STYLES[tone].tint,
					nothing && "opacity-55",
				)}
			>
				{!nothing && (
					<TileDecoration
						slot={geometry}
						placement={placement}
						refs={refs}
						preview={preview}
					/>
				)}
				<span className="relative flex h-full min-w-0 flex-col gap-0.75 px-2.5 py-2">
					<span className="flex min-w-0 items-center gap-1.5 pr-6">
						<TileBadge
							kind={placement.content.kind}
							label={kindBadgeLabel(placement.content.kind, t)}
							className={
								tone
									? TONE_STYLES[tone].text
									: selected
										? "text-primary"
										: "text-muted-foreground"
							}
						/>
						{fallback && (
							<span
								className={cn(FALLBACK_CHIP, "border-solid bg-blue-500/14")}
							>
								{t("exploreFallback", "fallback")}
							</span>
						)}
					</span>
					<span className="line-clamp-2 text-[12.5px] leading-4 font-semibold">
						{placement.name}
					</span>
					<span className="truncate text-[11px] leading-3.5 text-muted-foreground">
						{sub}
					</span>
					<span className="mt-auto flex min-w-0 items-center gap-1.25">
						<StatusDot status={status} />
						<AudienceIcons audience={placement.audience} />
						<span className="ml-auto shrink-0 font-mono text-[10.5px] tabular-nums text-muted-foreground">
							{slotSize(key)}
						</span>
					</span>
				</span>
			</button>
			<DragHandle
				id={tileDragId(placement.id)}
				placement={placement}
				label={t("exploreDragToMove", "Drag {{name}} to another slot", {
					name: placement.name,
				})}
			/>
		</div>
	);
}

function slotTargets(
	layout: ExploreLayoutDoc,
	slot: ExploreSlotKey,
): AddTarget[] {
	const choices = addChoicesFor(layout, slot);
	return choices.length ? [{ slot, newRow: false, choices }] : [];
}

interface Ghost {
	kind: "app" | "pkg";
	left: number;
	width: number;
	top: number;
	height: number;
}

const app = (left: number, width: number): Ghost => ({
	kind: "app",
	left,
	width,
	top: 0,
	height: 100,
});
const pkg = (left: number, width: number, top = 0, height = 100): Ghost => ({
	kind: "pkg",
	left,
	width,
	top,
	height,
});
const quarters = (make: (left: number, width: number) => Ghost) =>
	[0, 1, 2, 3].map((index) => make(index * 25.4, 23.8));

/** Schematic item blocks per rail, mirroring the landing's row layouts (§3.6). */
export function rowGhosts(
	content: ExplorePlacementContent,
	dev: boolean,
): Ghost[] {
	if (content.kind === "collection") {
		return dev
			? [app(0, 23.8), app(25.4, 23.8), pkg(50.8, 23.8), pkg(76.2, 23.8)]
			: quarters(app);
	}
	if (content.kind !== "rail") return [];
	switch (content.rail) {
		case "trending":
			return dev
				? [
						app(0, 23.8),
						pkg(25.4, 49.2, 0, 46),
						pkg(25.4, 49.2, 54, 46),
						app(76.2, 23.8),
					]
				: quarters(app);
		case "top_paid":
			return dev
				? [app(0, 23.8), app(25.4, 23.8), pkg(50.8, 23.8), pkg(76.2, 23.8)]
				: [app(0, 49.2), app(50.8, 49.2)];
		case "for_builders":
			return quarters(pkg);
		case "suites":
			return [app(0, 32.2), app(33.9, 32.2), app(67.8, 32.2)];
		default:
			return quarters(app);
	}
}

function rowIsTall(content: ExplorePlacementContent): boolean {
	return !(
		content.kind === "rail" &&
		(content.rail === "top_paid" || content.rail === "suites")
	);
}

function Ghosts({ ghosts }: { ghosts: Ghost[] }) {
	return (
		<span aria-hidden="true" className="relative min-w-0 flex-1">
			{ghosts.map((ghost, index) => (
				<span
					key={`${ghost.kind}-${index}`}
					className={cn(
						"absolute rounded border",
						ghost.kind === "app"
							? "border-foreground/8 bg-foreground/10"
							: "border-foreground/16 bg-foreground/3 bg-[radial-gradient(color-mix(in_oklab,var(--foreground)_28%,transparent)_0.6px,transparent_0.6px)] bg-size-[4px_4px]",
					)}
					style={{
						left: `${ghost.left}%`,
						width: `${ghost.width}%`,
						top: `${ghost.top}%`,
						height: `${ghost.height}%`,
					}}
				>
					<span className="absolute bottom-0.75 left-1.25 font-mono text-[8.5px] tracking-[0.06em] text-foreground/55 uppercase">
						{ghost.kind}
					</span>
				</span>
			))}
		</span>
	);
}

function CanvasRow({
	layout,
	slot,
	displayedId,
	chosen,
	markers,
	selectedId,
	viewer,
	nameOf,
	onSelect,
	onAdd,
	onRemoveRow,
}: {
	layout: ExploreLayoutDoc;
	slot: ExploreSlot;
	displayedId: string | null | undefined;
	chosen: string | null | undefined;
	markers?: TraceMarkers;
	selectedId: string | null;
	viewer: ExploreViewer;
	nameOf: ItemNameOf;
	onSelect: (id: string) => void;
	onAdd: AddHandler;
	onRemoveRow: (key: string) => void;
}) {
	const { t } = useTranslation("admin");
	const describedBy = useId();
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
		isOver,
	} = useSortable({ id: rowDragId(slot.key), data: { rowKey: slot.key } });
	const verdict = useDropVerdict(slot.key);
	const rowName = slotLabel(layout, slot.key, t);
	const placement =
		slot.placements.find((entry) => entry.id === displayedId) ??
		slot.placements[0];
	const grip = (
		<button
			type="button"
			className="absolute top-1.5 right-1.5 z-2 flex size-6 cursor-grab touch-none items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
			aria-label={t("exploreReorderRow", "Reorder {{row}}", { row: rowName })}
			{...attributes}
			{...listeners}
		>
			<GripVertical className="size-3.5" />
		</button>
	);
	const wrapper = cn(
		"relative shrink-0 rounded-[10px]",
		isDragging && "z-10 opacity-70",
		dropRing(isOver, verdict),
	);
	const style = { transform: CSS.Transform.toString(transform), transition };
	if (!placement) {
		const targets = slotTargets(layout, slot.key);
		return (
			<div ref={setNodeRef} style={style} className={wrapper}>
				<div className="flex h-12 items-center gap-2 rounded-[10px] border border-dashed bg-card/40 px-3 pr-9 text-xs text-muted-foreground">
					<span className="min-w-0 truncate">
						{t("exploreEmptyRow", "{{row}} is empty", { row: rowName })}
					</span>
					<AddMenu layout={layout} targets={targets} onAdd={onAdd}>
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="ml-auto h-7 gap-1 text-xs"
						>
							<Plus className="size-3" />
							{t("exploreAdd", "Add")}
						</Button>
					</AddMenu>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="h-7 gap-1 text-xs text-muted-foreground"
						onClick={() => onRemoveRow(slot.key)}
					>
						<Trash2 className="size-3" />
						{t("exploreRemoveRow", "Remove row")}
					</Button>
				</div>
				{grip}
			</div>
		);
	}
	const selected = placement.id === selectedId;
	const slotSelected =
		!selected && slot.placements.some((entry) => entry.id === selectedId);
	const fallback = !!markers?.fallback.has(placement.id);
	const nothing = markers !== undefined && chosen === null;
	const hiddenRail = !placement.enabled && placement.content.kind === "rail";
	const tall = rowIsTall(placement.content);
	const sub = hiddenRail
		? t("exploreRailHiddenShort", "Hidden from the page")
		: nothing
			? t("exploreNothingForAudience", "Nothing live for this audience")
			: placementSummary(placement, t, nameOf);
	return (
		<div ref={setNodeRef} style={style} className={wrapper}>
			<MarkerDescription
				id={describedBy}
				placement={placement}
				fallback={fallback}
				sub={sub}
			/>
			<button
				type="button"
				aria-pressed={selected}
				aria-label={t("exploreSelectRow", "Select {{row}}: {{name}}", {
					row: rowName,
					name: placement.name,
				})}
				aria-describedby={describedBy}
				onClick={() => onSelect(placement.id)}
				className={cn(
					"relative block w-full min-w-0 rounded-[10px] text-left transition-colors hover:border-primary/55 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					tall ? "h-26" : "h-12",
					frameClasses({ selected, slotSelected, fallback }),
					(hiddenRail || nothing) && "border-dashed opacity-55",
				)}
			>
				<span className="relative flex h-full min-w-0 items-stretch gap-3.5 py-2 pr-9 pl-2.5">
					<span className="flex w-46 shrink-0 flex-col gap-0.75">
						<span className="flex min-w-0 items-center gap-1.5">
							<TileBadge
								kind={placement.content.kind}
								label={kindBadgeLabel(placement.content.kind, t)}
								className={selected ? "text-primary" : "text-muted-foreground"}
							/>
							<span className="min-w-0 truncate text-[12.5px] leading-4 font-semibold">
								{placement.name}
							</span>
							{fallback && (
								<span
									className={cn(FALLBACK_CHIP, "border-solid bg-blue-500/14")}
								>
									{t("exploreFallback", "fallback")}
								</span>
							)}
						</span>
						<span className="truncate text-[11px] leading-3.5 text-muted-foreground">
							{sub}
						</span>
						{tall && (
							<span className="mt-auto flex items-center gap-1.25">
								<StatusDot status={effectiveStatus(placement)} />
								<AudienceIcons audience={placement.audience} />
								<span className="ml-auto font-mono text-[10.5px] text-muted-foreground">
									{slotSize(slot.key)}
								</span>
							</span>
						)}
					</span>
					{!hiddenRail && (
						<Ghosts ghosts={rowGhosts(placement.content, viewer.dev)} />
					)}
				</span>
			</button>
			{grip}
		</div>
	);
}

function NewRowZone({
	layout,
	full,
	onAdd,
	onAddRow,
}: {
	layout: ExploreLayoutDoc;
	full: boolean;
	onAdd: AddHandler;
	onAddRow: () => void;
}) {
	const { t } = useTranslation("admin");
	const { setNodeRef, isOver } = useDroppable({ id: NEW_ROW_DROP });
	const { active } = useDndContext();
	const content = (active?.data.current as DragPayload | undefined)?.content;
	const verdict =
		content === undefined ? undefined : !full && slotAccepts("row:x", content);
	const targets = full
		? []
		: LIBRARY_KINDS.flatMap((entry) =>
				addTargets(layout, entry).filter((target) => target.newRow),
			).reduce<AddTarget[]>((merged, target) => {
				const existing = merged.find((entry) => entry.slot === target.slot);
				if (existing) existing.choices.push(...target.choices);
				else merged.push({ ...target, choices: [...target.choices] });
				return merged;
			}, []);
	return (
		<div
			ref={setNodeRef}
			className={cn(
				"flex h-11 shrink-0 items-center justify-center gap-3 rounded-[10px] border border-dashed text-xs text-muted-foreground",
				dropRing(isOver, verdict),
			)}
		>
			{full ? (
				<span>
					{t("exploreRowsFull", "The page holds the maximum of {{max}} rows", {
						max: EXPLORE_LIMITS.rows,
					})}
				</span>
			) : (
				<>
					<span className="hidden sm:inline">
						{t("exploreDropNewRow", "Drop a placement here to add a row")}
					</span>
					<AddMenu
						layout={layout}
						targets={targets}
						onAdd={onAdd}
						align="center"
					>
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="h-7 gap-1 text-xs"
						>
							<Plus className="size-3" />
							{t("exploreAddToNewRow", "Add to a new row")}
						</Button>
					</AddMenu>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="h-7 gap-1 text-xs"
						onClick={onAddRow}
					>
						{t("exploreAddRow", "Add empty row")}
					</Button>
				</>
			)}
		</div>
	);
}

function Legend() {
	const { t } = useTranslation("admin");
	return (
		<>
			<span className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
				<span
					aria-hidden="true"
					className="h-3.5 w-2.5 rounded-sm bg-foreground/14"
				/>
				{t("exploreLegendApp", "app")}
			</span>
			<span className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
				<span
					aria-hidden="true"
					className="h-2.25 w-4 rounded-sm border border-foreground/22 bg-[radial-gradient(color-mix(in_oklab,var(--foreground)_35%,transparent)_0.6px,transparent_0.6px)] bg-size-[3px_3px]"
				/>
				{t("exploreLegendPackage", "package")}
			</span>
			<span className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
				<span
					aria-hidden="true"
					className="h-2.5 w-3 rounded-sm border border-dashed border-blue-500"
				/>
				{t("exploreLegendFallback", "fallback")}
			</span>
		</>
	);
}

function PreviewChrome({ viewer }: { viewer: ExploreViewer }) {
	const { t } = useTranslation("admin");
	return (
		<div aria-hidden="true" className="flex h-6 items-center gap-2">
			<span className="text-[13px] font-semibold tracking-[-0.01em]">
				{t("exploreChromeTitle", "Explore")}
			</span>
			<span className="ml-auto inline-flex h-5 w-36 items-center gap-1.25 rounded-[5px] border bg-card px-1.75 text-[10px] text-muted-foreground">
				<Search className="size-2.5" />
				{t("exploreChromeSearch", "Search")}
			</span>
			{viewer.dev && (
				<span className="inline-flex h-5 items-center gap-0.5 rounded-[5px] border bg-card p-0.5 text-[9.5px] text-muted-foreground">
					<span className="rounded-[3px] bg-muted px-1.25 text-foreground">
						{t("exploreChromeAll", "All")}
					</span>
					<span className="px-1.25">{t("exploreChromeApps", "Apps")}</span>
					<span className="px-1.25">
						{t("exploreChromePackages", "Packages")}
					</span>
				</span>
			)}
			{viewer.signedIn ? (
				<span className="size-5 rounded-full border bg-muted" />
			) : (
				<span className="inline-flex h-5 items-center rounded-[5px] bg-primary px-2 text-[10px] font-semibold text-primary-foreground">
					{t("exploreChromeSignIn", "Sign in")}
				</span>
			)}
		</div>
	);
}

export function previewLabel(viewer: ExploreViewer, t: AdminT) {
	return [
		viewer.dev
			? t("explorePreviewDev", "developer mode")
			: t("explorePreviewNoDev", "no developer mode"),
		viewer.signedIn
			? audienceLabel("signed_in", t).toLowerCase()
			: audienceLabel("signed_out", t).toLowerCase(),
		audienceLabel(viewer.platform, t),
		viewer.language,
	].join(" · ");
}

export function ExploreCanvas({
	layout,
	markers,
	preview,
	previewPending,
	previewFailed,
	onRetryPreview,
	viewer,
	selectedId,
	refs,
	nameOf,
	showGrid,
	onToggleGrid,
	onSelect,
	onAdd,
	onAddRow,
	onRemoveRow,
}: {
	layout: ExploreLayoutDoc;
	markers?: TraceMarkers;
	preview?: ResolvedExplore;
	previewPending: boolean;
	previewFailed: boolean;
	onRetryPreview: () => void;
	viewer: ExploreViewer;
	selectedId: string | null;
	refs: ReadonlyMap<string, ExploreItemRef>;
	nameOf: ItemNameOf;
	showGrid: boolean;
	onToggleGrid: () => void;
	onSelect: (id: string) => void;
	onAdd: AddHandler;
	onAddRow: () => void;
	onRemoveRow: (key: string) => void;
}) {
	const { t } = useTranslation("admin");
	const rows = layoutRows(layout);
	const displayed = (key: string) =>
		markers
			? markers.chosen.get(key)
			: findSlot(layout, key)?.placements[0]?.id;
	return (
		<div className="flex flex-col px-4 py-4 lg:h-full lg:min-h-0 lg:px-5">
			<div className="flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
				<div className="flex min-w-0 items-baseline gap-2.5">
					<h2 className="text-[13px] font-semibold whitespace-nowrap">
						{t("explorePageLayout", "Page layout")}
					</h2>
					{previewFailed ? (
						<span
							role="alert"
							className="inline-flex min-w-0 items-center gap-1.5 text-xs text-amber-700 dark:text-amber-400"
						>
							<AlertTriangle aria-hidden="true" className="size-3.5 shrink-0" />
							<span className="truncate">
								{t(
									"explorePreviewUnavailable",
									"Preview unavailable, so fallback and hidden markers are off.",
								)}
							</span>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="h-6 px-1.5 text-xs"
								disabled={previewPending}
								onClick={onRetryPreview}
							>
								{t("exploreRetry", "Try again")}
							</Button>
						</span>
					) : (
						<span className="truncate text-xs text-muted-foreground">
							{t("explorePreviewLabel", "Preview: {{viewer}}", {
								viewer: previewLabel(viewer, t),
							})}
							{previewPending && "…"}
						</span>
					)}
				</div>
				<div className="flex shrink-0 items-center gap-3">
					<span className="hidden items-center gap-3 sm:inline-flex">
						<Legend />
					</span>
					<Button
						type="button"
						variant="outline"
						size="sm"
						aria-pressed={showGrid}
						onClick={onToggleGrid}
						className={cn(
							"h-6.5 gap-1.5 px-2.25 text-[11.5px]",
							showGrid &&
								"border-blue-500/45 bg-blue-500/12 text-blue-700 hover:bg-blue-500/16 dark:text-blue-400",
						)}
					>
						<Grid3x3 className="size-3.5" />
						{t("exploreGridToggle", "12-col grid")}
					</Button>
				</div>
			</div>
			<div className="relative mt-3 flex min-h-0 flex-1 overflow-hidden rounded-xl border bg-muted/20">
				<div
					aria-hidden="true"
					className="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--border)_0.8px,transparent_0.8px)] bg-size-[12px_12px]"
				/>
				{viewer.platform === "desktop" && (
					<div
						aria-hidden="true"
						className="relative hidden w-4 shrink-0 flex-col items-center gap-1.75 border-r bg-background pt-3.5 sm:flex"
					>
						<span className="size-2 rounded-xs bg-muted-foreground" />
						<span className="size-1.5 rounded-xs bg-border" />
						<span className="size-1.5 rounded-xs bg-muted-foreground/70" />
						<span className="size-1.5 rounded-xs bg-border" />
					</div>
				)}
				<div className="relative min-w-0 flex-1 overflow-x-auto lg:overflow-y-auto">
					<div className="flex min-w-140 flex-col gap-3 px-4 pt-3.5 pb-4">
						<PreviewChrome viewer={viewer} />
						<div
							className="relative grid grid-cols-12 gap-2"
							style={{
								gridTemplateRows: CANVAS_ROWS.map(
									(height) => `${height}px`,
								).join(" "),
							}}
						>
							{BENTO_SLOTS.map((geometry) => (
								<GridTile
									key={geometry.key}
									layout={layout}
									geometry={geometry}
									slot={findSlot(layout, geometry.key)}
									displayedId={displayed(geometry.key)}
									chosen={markers?.chosen.get(geometry.key)}
									markers={markers}
									selectedId={selectedId}
									refs={refs}
									nameOf={nameOf}
									preview={preview}
									onSelect={onSelect}
									onAdd={onAdd}
								/>
							))}
							{showGrid && (
								<span
									aria-hidden="true"
									className="pointer-events-none absolute inset-0 z-4 grid grid-cols-12 gap-2"
								>
									{Array.from({ length: 12 }, (_, index) => (
										<span
											// biome-ignore lint/suspicious/noArrayIndexKey: The twelve guides are fixed columns.
											key={index}
											className="border-x border-dashed border-blue-500/28 bg-blue-500/6"
										/>
									))}
								</span>
							)}
						</div>
						<SortableContext
							items={rows.map((row) => rowDragId(row.key))}
							strategy={verticalListSortingStrategy}
						>
							<ol
								aria-label={t("exploreRows", "Rows")}
								className="flex flex-col gap-3"
							>
								{rows.map((row) => (
									<li key={row.key}>
										<CanvasRow
											layout={layout}
											slot={row}
											displayedId={displayed(row.key)}
											chosen={markers?.chosen.get(row.key)}
											markers={markers}
											selectedId={selectedId}
											viewer={viewer}
											nameOf={nameOf}
											onSelect={onSelect}
											onAdd={onAdd}
											onRemoveRow={onRemoveRow}
										/>
									</li>
								))}
							</ol>
						</SortableContext>
						<NewRowZone
							layout={layout}
							full={rows.length >= EXPLORE_LIMITS.rows}
							onAdd={onAdd}
							onAddRow={onAddRow}
						/>
					</div>
				</div>
			</div>
		</div>
	);
}
