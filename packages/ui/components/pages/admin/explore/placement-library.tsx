"use client";

import {
	SortableContext,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "@flow-like/locales";
import { EyeOff, GripVertical, Plus } from "lucide-react";
import { cn } from "../../../../lib/utils";
import type {
	ExploreLayoutDoc,
	ExplorePlacement,
	ExploreSlot,
} from "../../../store/explore/explore-types";
import { type AddHandler, AddMenu } from "./add-menu";
import {
	LIBRARY_KINDS,
	type LibraryKind,
	STATUSES,
	type TraceMarkers,
	addTargets,
	audienceLabel,
	effectiveStatus,
	findSlot,
	libraryLabel,
	orderedSlotKeys,
	scheduleSummary,
	skipReasonLabel,
	statusCountLabel,
	statusCounts,
} from "./explore-admin-model";
import {
	FALLBACK_CHIP,
	KindIcon,
	SECTION_TITLE,
	StatusDot,
	StatusPill,
} from "./explore-admin-visuals";

export const placementDragId = (id: string) => `p:${id}`;

const LIBRARY_ICON: Record<
	LibraryKind,
	Parameters<typeof KindIcon>[0]["kind"]
> = {
	announcement: "announcement",
	spotlight: "spotlight",
	collection: "collection",
	rail: "rail",
	sponsored: "sponsored",
};

function LibraryButton({
	entry,
	layout,
	onAdd,
}: {
	entry: LibraryKind;
	layout: ExploreLayoutDoc;
	onAdd: AddHandler;
}) {
	const { t } = useTranslation("admin");
	const { name, description } = libraryLabel(entry, t);
	const targets = addTargets(layout, entry);
	const button = (
		<button
			type="button"
			disabled={!targets.length}
			title={
				targets.length
					? description
					: t(
							"exploreLibraryFull",
							"The draft holds the maximum number of placements",
						)
			}
			className="flex h-9 min-w-0 items-center gap-2 rounded-lg border bg-card pr-2.5 pl-1.5 text-left transition-colors hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
		>
			<span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-foreground">
				<KindIcon kind={LIBRARY_ICON[entry]} />
			</span>
			<span className="flex min-w-0 flex-1 items-baseline gap-2">
				<span className="shrink-0 text-[12.5px] leading-4 font-semibold">
					{name}
				</span>
				<span className="min-w-0 truncate text-[11px] leading-3.5 text-muted-foreground">
					{description}
				</span>
			</span>
			<span className="inline-flex shrink-0 items-center text-primary">
				<Plus aria-hidden="true" className="size-3.5" />
				<span className="sr-only">{t("exploreAdd", "Add")}</span>
			</span>
		</button>
	);
	if (!targets.length) return button;
	return (
		<AddMenu layout={layout} targets={targets} onAdd={onAdd}>
			{button}
		</AddMenu>
	);
}

function PlacementRow({
	slot,
	placement,
	fallback,
	selected,
	hidden,
	formatDate,
	onSelect,
}: {
	slot: ExploreSlot;
	placement: ExplorePlacement;
	fallback: boolean;
	selected: boolean;
	hidden?: string;
	formatDate: (iso: string) => string;
	onSelect: () => void;
}) {
	const { t } = useTranslation("admin");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id: placementDragId(placement.id),
		data: { placementId: placement.id, content: placement.content },
	});
	const status = effectiveStatus(placement);
	const compact = fallback && slot.area !== "unplaced";
	const meta = [
		scheduleSummary(placement, t, formatDate),
		(placement.audience.length ? placement.audience : ["everyone"])
			.map((tag) => audienceLabel(tag, t))
			.join(", "),
	].join(" · ");
	const hiddenIcon = hidden && (
		<EyeOff
			role="img"
			aria-label={hidden}
			className="size-3.5 shrink-0 text-muted-foreground"
		>
			<title>{hidden}</title>
		</EyeOff>
	);
	return (
		<li
			ref={setNodeRef}
			style={{ transform: CSS.Transform.toString(transform), transition }}
			className={cn(
				"group relative flex items-center rounded-lg border",
				selected
					? "border-border bg-muted/60"
					: "border-transparent hover:bg-muted/40",
				isDragging && "z-10 opacity-60",
			)}
		>
			<button
				type="button"
				className="flex h-6 w-3.5 shrink-0 cursor-grab touch-none items-center justify-center rounded text-muted-foreground opacity-0 group-hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-ring"
				aria-label={t(
					"exploreReorderPriority",
					"Change the priority of {{name}}",
					{
						name: placement.name,
					},
				)}
				{...attributes}
				{...listeners}
			>
				<GripVertical className="size-3" />
			</button>
			<button
				type="button"
				aria-pressed={selected}
				onClick={onSelect}
				className={cn(
					"flex min-w-0 flex-1 items-center gap-2.5 rounded-lg text-left focus-visible:outline-2 focus-visible:outline-ring",
					compact ? "h-7.5 pr-2 pl-0.5" : "py-1 pr-2 pl-0.5",
				)}
			>
				{compact ? (
					<>
						<span aria-hidden="true" className="w-6 shrink-0" />
						<span
							className={cn(
								"min-w-0 truncate text-[12.5px] leading-4.25",
								selected ? "font-semibold" : "font-medium",
								status !== "live" && !selected && "text-muted-foreground",
							)}
						>
							{placement.name}
						</span>
						<span className={FALLBACK_CHIP}>
							{t("exploreFallback", "fallback")}
						</span>
						{hiddenIcon}
						<StatusDot status={status} className="ml-auto" />
					</>
				) : (
					<>
						<span className="flex size-6.5 shrink-0 items-center justify-center rounded-md border bg-card text-muted-foreground">
							<KindIcon kind={placement.content.kind} />
						</span>
						<span className="flex min-w-0 flex-1 flex-col gap-0.5">
							<span className="flex min-w-0 items-center gap-1.5">
								<span
									className={cn(
										"min-w-0 truncate text-[13px] leading-4.25",
										selected ? "font-semibold" : "font-medium",
										status !== "live" && !selected && "text-muted-foreground",
									)}
								>
									{placement.name}
								</span>
								{hiddenIcon && <span className="ml-auto">{hiddenIcon}</span>}
							</span>
							<span className="flex min-w-0 items-center gap-1.5">
								<StatusPill status={status} />
								<span className="min-w-0 truncate text-[11px] leading-3.5 text-muted-foreground">
									{meta}
								</span>
							</span>
						</span>
					</>
				)}
			</button>
		</li>
	);
}

export function PlacementLibrary({
	layout,
	markers,
	selectedId,
	now,
	formatDate,
	onSelect,
	onAdd,
}: {
	layout: ExploreLayoutDoc;
	markers?: TraceMarkers;
	selectedId: string | null;
	now?: string;
	formatDate: (iso: string) => string;
	onSelect: (id: string) => void;
	onAdd: AddHandler;
}) {
	const { t } = useTranslation("admin");
	const counts = statusCounts(layout, now);
	const summary = STATUSES.filter((status) => counts[status] > 0)
		.map((status) => statusCountLabel(status, counts[status], t))
		.join(" · ");
	const slots = orderedSlotKeys(layout)
		.map((key) => findSlot(layout, key))
		.filter(
			(slot): slot is ExploreSlot => !!slot && slot.placements.length > 0,
		);
	return (
		<div className="flex flex-col gap-2 px-3.5 py-3 lg:h-full lg:min-h-0">
			<div className="flex h-4.5 items-center justify-between px-1">
				<h2 className={SECTION_TITLE}>
					{t("exploreAddPlacement", "Add placement")}
				</h2>
				<span className="text-[11px] text-muted-foreground">
					{t("exploreAddHint", "then pick a slot")}
				</span>
			</div>
			<div className="grid grid-cols-1 gap-1 sm:grid-cols-2 md:grid-cols-1">
				{LIBRARY_KINDS.map((entry) => (
					<LibraryButton
						key={entry}
						entry={entry}
						layout={layout}
						onAdd={onAdd}
					/>
				))}
			</div>
			<div className="mt-1.5 flex items-baseline justify-between gap-2 border-t px-1 pt-2">
				<h2 className={cn(SECTION_TITLE, "shrink-0 whitespace-nowrap")}>
					{t("exploreOnThisPage", "On this page")}
				</h2>
				<span className="min-w-0 truncate text-[11px] tabular-nums text-muted-foreground">
					{summary}
				</span>
			</div>
			<div className="flex max-h-96 flex-col gap-2 overflow-y-auto lg:max-h-none lg:min-h-0 lg:flex-1">
				{slots.map((slot) => (
					<SortableContext
						key={slot.key}
						items={slot.placements.map((placement) =>
							placementDragId(placement.id),
						)}
						strategy={verticalListSortingStrategy}
					>
						{slot.area === "unplaced" && (
							<h3 className="px-1 pt-1 text-[11px] font-medium text-muted-foreground">
								{t("exploreSlotUnplacedList", "Unplaced · not on the page")}
							</h3>
						)}
						<ul
							className="flex flex-col gap-px"
							aria-label={
								slot.area === "unplaced"
									? t("exploreSlotUnplaced", "Unplaced")
									: undefined
							}
						>
							{slot.placements.map((placement, index) => {
								const reason = markers?.hidden.get(placement.id);
								return (
									<PlacementRow
										key={placement.id}
										slot={slot}
										placement={placement}
										fallback={index > 0}
										selected={placement.id === selectedId}
										hidden={reason ? skipReasonLabel(reason, t) : undefined}
										formatDate={formatDate}
										onSelect={() => onSelect(placement.id)}
									/>
								);
							})}
						</ul>
					</SortableContext>
				))}
				{!slots.length && (
					<p className="rounded-lg border border-dashed p-4 text-center text-xs text-muted-foreground">
						{t("exploreNoPlacements", "No placements yet. Add one above.")}
					</p>
				)}
			</div>
		</div>
	);
}
