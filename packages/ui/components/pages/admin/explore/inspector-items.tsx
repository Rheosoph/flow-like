"use client";

import {
	DndContext,
	type DragEndEvent,
	KeyboardSensor,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	arrayMove,
	sortableKeyboardCoordinates,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	AlertTriangle,
	Code2,
	GripVertical,
	Layers,
	Loader2,
	Plus,
	Search,
	X,
} from "lucide-react";
import { type ReactNode, useEffect, useId, useMemo, useState } from "react";
import { getApiOrigin } from "../../../../lib/api-url";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { categoryColor } from "../../../../lib/category-meta";
import { asArray } from "../../../../lib/response-shape";
import { IAppCategory } from "../../../../lib/schema/app/app";
import { cn } from "../../../../lib/utils";
import { useBackend } from "../../../../state/backend-state";
import { HomeAppPicker } from "../../../home/home-widget-settings";
import { ProfileMediaField } from "../../../profile-templates/profile-media-field";
import {
	EXPLORE_LIMITS,
	type ExploreAccent,
	type ExploreItemKind,
	type ExploreItemRef,
	type ExploreLayoutDoc,
	type ExplorePlacementItem,
} from "../../../store/explore/explore-types";
import {
	getPackageInitials,
	usePackageGradient,
} from "../../../store/package-card";
import { Button } from "../../../ui/button";
import { Checkbox } from "../../../ui/checkbox";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../../ui/tabs";
import {
	collectionPlacements,
	refKey,
	replaceKindItems,
} from "./explore-admin-model";
import { StatusPill } from "./explore-admin-visuals";
import { CountedField, FieldError, InspectorSection } from "./inspector-fields";

export type ItemNameOf = (
	item: Pick<ExplorePlacementItem, "kind" | "id">,
) => string;

/** Names from the server's refs, collection placements of the layout, then names the pickers just saw. */
export function useItemNames(
	layout: ExploreLayoutDoc,
	refs: ReadonlyMap<string, ExploreItemRef>,
	picked: ReadonlyMap<string, string>,
): ItemNameOf {
	return useMemo(() => {
		const collections = new Map(
			collectionPlacements(layout).map((placement) => [
				placement.id,
				placement.content.kind === "collection"
					? placement.content.title || placement.name
					: placement.name,
			]),
		);
		return (item) => {
			const key = refKey(item.kind, item.id);
			if (item.kind === "collection") {
				return collections.get(item.id) ?? picked.get(key) ?? item.id;
			}
			return refs.get(key)?.name ?? picked.get(key) ?? item.id;
		};
	}, [layout, refs, picked]);
}

function KindChip({ kind }: { kind: ExploreItemKind }) {
	const { t } = useTranslation("admin");
	const label =
		kind === "app"
			? t("exploreItemKindApp", "App")
			: kind === "package"
				? t("exploreItemKindPackage", "Package")
				: t("exploreItemKindCollection", "Collection");
	return (
		<span className="inline-flex h-4 shrink-0 items-center rounded bg-muted px-1.25 font-mono text-[9.5px] uppercase tracking-[0.05em] text-muted-foreground">
			{label}
		</span>
	);
}

function PackageThumb({ id, name }: { id: string; name: string }) {
	const gradient = usePackageGradient(id);
	return (
		<span
			className="flex size-full items-center justify-center font-mono text-[9px] font-bold text-white/80"
			style={{ background: gradient }}
		>
			{getPackageInitials(name)}
		</span>
	);
}

export function ItemThumb({
	item,
	name,
	itemRef,
	className,
}: {
	item: Pick<ExplorePlacementItem, "kind" | "id" | "artworkUrl">;
	name: string;
	itemRef?: ExploreItemRef;
	className?: string;
}) {
	const [failed, setFailed] = useState<string | null>(null);
	const image =
		item.artworkUrl ?? itemRef?.coverUrl ?? itemRef?.iconUrl ?? null;
	return (
		<span
			aria-hidden="true"
			className={cn(
				"flex h-6.5 w-10 shrink-0 items-center justify-center overflow-hidden rounded-[5px] border bg-muted",
				className,
			)}
		>
			{image && failed !== image ? (
				<img
					src={image}
					alt=""
					className="size-full object-cover"
					onError={() => setFailed(image)}
				/>
			) : item.kind === "package" ? (
				<PackageThumb id={item.id} name={name} />
			) : item.kind === "collection" ? (
				<Layers className="size-3.5 text-muted-foreground" />
			) : (
				<span className="font-mono text-[9px] font-bold text-muted-foreground">
					{getPackageInitials(name)}
				</span>
			)}
		</span>
	);
}

function ItemWarning({ itemRef }: { itemRef?: ExploreItemRef }) {
	const { t } = useTranslation("admin");
	if (!itemRef || (itemRef.exists && itemRef.public)) return null;
	const label = !itemRef.exists
		? t("exploreItemMissing", "This item no longer exists, so viewers skip it")
		: t("exploreItemPrivate", "This item is not public, so viewers skip it");
	return (
		<AlertTriangle
			role="img"
			aria-label={label}
			className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400"
		>
			<title>{label}</title>
		</AlertTriangle>
	);
}

function SortableItemRow({
	item,
	id,
	name,
	itemRef,
	selected,
	onSelect,
	onRemove,
}: {
	item: ExplorePlacementItem;
	id: string;
	name: string;
	itemRef?: ExploreItemRef;
	selected: boolean;
	onSelect?: () => void;
	onRemove: () => void;
}) {
	const { t } = useTranslation("admin");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({ id });
	const devOnly = t("exploreItemDevOnly", "Only shown with developer mode on");
	return (
		<li
			ref={setNodeRef}
			style={{ transform: CSS.Transform.toString(transform), transition }}
			className={cn(
				"flex items-center gap-2 rounded-lg border py-1.25 pr-1.5 pl-1",
				selected ? "border-primary bg-muted/60" : "bg-card",
				isDragging && "z-10 opacity-70 shadow-lg",
			)}
		>
			<button
				type="button"
				className="flex size-5 shrink-0 cursor-grab touch-none items-center justify-center rounded text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
				aria-label={t("exploreItemReorder", "Reorder {{name}}", { name })}
				{...attributes}
				{...listeners}
			>
				<GripVertical className="size-3.5" />
			</button>
			<ItemThumb item={item} name={name} itemRef={itemRef} />
			<button
				type="button"
				aria-pressed={onSelect ? selected : undefined}
				disabled={!onSelect}
				onClick={onSelect}
				className="flex min-w-0 flex-1 items-center gap-1.5 text-left disabled:cursor-default"
			>
				<span
					className={cn(
						"min-w-0 truncate font-medium",
						item.kind === "package"
							? "font-mono text-xs"
							: "text-[13px] leading-4.25",
					)}
				>
					{name}
				</span>
				{item.kind === "package" && (
					<Code2
						role="img"
						aria-label={devOnly}
						className="size-3.5 shrink-0 text-primary"
					>
						<title>{devOnly}</title>
					</Code2>
				)}
				<ItemWarning itemRef={itemRef} />
				<span className="ml-auto">
					<KindChip kind={item.kind} />
				</span>
			</button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-5.5 text-muted-foreground"
				aria-label={t("exploreItemRemove", "Remove {{name}}", { name })}
				onClick={onRemove}
			>
				<X className="size-3.5" />
			</Button>
		</li>
	);
}

export function ItemsEditor({
	title,
	meta,
	addLabel,
	items,
	kinds,
	max,
	layout,
	placementId,
	refs,
	nameOf,
	selectedIndex,
	error,
	onSelect,
	onChange,
	onPicked,
}: {
	title: string;
	meta?: ReactNode;
	addLabel: string;
	items: readonly ExplorePlacementItem[];
	kinds: readonly ExploreItemKind[];
	max: number;
	layout: ExploreLayoutDoc;
	placementId: string;
	refs: ReadonlyMap<string, ExploreItemRef>;
	nameOf: ItemNameOf;
	selectedIndex?: number;
	error?: string;
	onSelect?: (index: number) => void;
	onChange: (items: ExplorePlacementItem[]) => void;
	onPicked: (key: string, name: string) => void;
}) {
	const { t } = useTranslation("admin");
	const [open, setOpen] = useState(false);
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
		useSensor(KeyboardSensor, {
			coordinateGetter: sortableKeyboardCoordinates,
		}),
	);
	const ids = items.map((item) => refKey(item.kind, item.id));
	const dragEnd = ({ active, over }: DragEndEvent) => {
		if (!over || active.id === over.id) return;
		const from = ids.indexOf(String(active.id));
		const to = ids.indexOf(String(over.id));
		if (from < 0 || to < 0) return;
		onChange(arrayMove([...items], from, to));
		if (selectedIndex === from) onSelect?.(to);
	};
	const errorId = `items-${placementId}-error`;
	return (
		<InspectorSection
			title={title}
			meta={meta}
			action={
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="h-6 gap-1 border-dashed px-2 text-[11.5px] text-muted-foreground"
					onClick={() => setOpen(true)}
				>
					<Plus className="size-3" />
					{addLabel}
				</Button>
			}
		>
			{items.length ? (
				<DndContext
					sensors={sensors}
					collisionDetection={closestCenter}
					onDragEnd={dragEnd}
				>
					<SortableContext items={ids} strategy={verticalListSortingStrategy}>
						<ul
							className="flex flex-col gap-1.5"
							aria-describedby={error ? errorId : undefined}
						>
							{items.map((item, index) => (
								<SortableItemRow
									key={ids[index]}
									id={ids[index]}
									item={item}
									name={nameOf(item)}
									itemRef={refs.get(ids[index])}
									selected={selectedIndex === index}
									onSelect={onSelect ? () => onSelect(index) : undefined}
									onRemove={() =>
										onChange(items.filter((_, position) => position !== index))
									}
								/>
							))}
						</ul>
					</SortableContext>
				</DndContext>
			) : (
				<p className="rounded-lg border border-dashed px-3 py-3 text-center text-xs text-muted-foreground">
					{t("exploreItemsEmpty", "No items yet.")}
				</p>
			)}
			<FieldError id={errorId} message={error} />
			<ItemPickerDialog
				open={open}
				onOpenChange={setOpen}
				items={items}
				kinds={kinds}
				max={max}
				layout={layout}
				placementId={placementId}
				onChange={onChange}
				onPicked={onPicked}
			/>
		</InspectorSection>
	);
}

function ItemPickerDialog({
	open,
	onOpenChange,
	items,
	kinds,
	max,
	layout,
	placementId,
	onChange,
	onPicked,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	items: readonly ExplorePlacementItem[];
	kinds: readonly ExploreItemKind[];
	max: number;
	layout: ExploreLayoutDoc;
	placementId: string;
	onChange: (items: ExplorePlacementItem[]) => void;
	onPicked: (key: string, name: string) => void;
}) {
	const { t } = useTranslation("admin");
	const single = max === 1;
	const idsOf = (kind: ExploreItemKind) =>
		items.filter((item) => item.kind === kind).map((item) => item.id);
	const room = (kind: ExploreItemKind) =>
		single ? 1 : max - (items.length - idsOf(kind).length);
	const choose = (kind: ExploreItemKind, ids: string[]) => {
		if (single) {
			const id = ids[ids.length - 1];
			onChange(
				id ? [{ kind, id }] : items.filter((item) => item.kind !== kind),
			);
			return;
		}
		if (ids.length > room(kind)) return;
		onChange(replaceKindItems(items, kind, ids));
	};
	const tabLabel = (kind: ExploreItemKind) =>
		kind === "app"
			? t("exploreItemTabApps", "Apps")
			: kind === "package"
				? t("exploreItemTabPackages", "Packages")
				: t("exploreItemTabCollections", "Collections");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85vh] flex-col sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{single
							? t("exploreItemPickOne", "Pick the item")
							: t("exploreItemPickMany", "Pick items")}
					</DialogTitle>
					<DialogDescription>
						{single
							? t(
									"exploreItemPickOneHint",
									"Choosing another item replaces the current one.",
								)
							: t(
									"exploreItemPickManyHint",
									"Choose up to {{max}} items in total.",
									{
										max,
									},
								)}
					</DialogDescription>
				</DialogHeader>
				<DialogBody>
					<Tabs defaultValue={kinds[0]} className="min-w-0">
						{kinds.length > 1 && (
							<TabsList className="w-full">
								{kinds.map((kind) => (
									<TabsTrigger key={kind} value={kind} className="flex-1">
										{tabLabel(kind)}
									</TabsTrigger>
								))}
							</TabsList>
						)}
						{kinds.includes("app") && (
							<TabsContent value="app" className="pt-3">
								<HomeAppPicker
									label={tabLabel("app")}
									value={idsOf("app")}
									multiple={!single}
									maxItems={single ? undefined : room("app")}
									onChange={(ids) => choose("app", ids)}
								/>
							</TabsContent>
						)}
						{kinds.includes("package") && (
							<TabsContent value="package" className="pt-3">
								<PackagePicker
									value={idsOf("package")}
									limit={room("package")}
									multiple={!single}
									onChange={(ids) => choose("package", ids)}
									onPicked={(id, name) => onPicked(refKey("package", id), name)}
								/>
							</TabsContent>
						)}
						{kinds.includes("collection") && (
							<TabsContent value="collection" className="pt-3">
								<CollectionPicker
									layout={layout}
									excludeId={placementId}
									value={idsOf("collection")}
									limit={room("collection")}
									onChange={(ids) => choose("collection", ids)}
								/>
							</TabsContent>
						)}
					</Tabs>
				</DialogBody>
				<DialogFooter>
					<Button type="button" onClick={() => onOpenChange(false)}>
						{t("exploreItemPickDone", "Done")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function PickerRow({
	checked,
	disabled,
	onToggle,
	children,
}: {
	checked: boolean;
	disabled: boolean;
	onToggle: () => void;
	children: ReactNode;
}) {
	const id = useId();
	return (
		<li
			className={cn(
				"flex items-center gap-2 rounded p-2 hover:bg-muted",
				disabled && "opacity-60",
			)}
		>
			<Checkbox
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onToggle}
			/>
			<label
				htmlFor={id}
				className={cn(
					"flex min-w-0 flex-1 cursor-pointer items-center gap-2",
					disabled && "cursor-not-allowed",
				)}
			>
				{children}
			</label>
		</li>
	);
}

function toggleId(
	value: readonly string[],
	id: string,
	multiple: boolean,
	limit: number,
): string[] | null {
	if (!multiple) return value[0] === id ? [] : [id];
	if (value.includes(id)) return value.filter((entry) => entry !== id);
	return value.length < limit ? [...value, id] : null;
}

export function PackagePicker({
	value,
	limit,
	multiple,
	onChange,
	onPicked,
}: {
	value: string[];
	limit: number;
	multiple: boolean;
	onChange: (ids: string[]) => void;
	onPicked: (id: string, name: string) => void;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const origin = getApiOrigin(backend.profile);
	const [query, setQuery] = useState("");
	const [search, setSearch] = useState("");
	useEffect(() => {
		const timer = setTimeout(() => setSearch(query.trim()), 300);
		return () => clearTimeout(timer);
	}, [query]);
	const results = useQuery({
		queryKey: ["admin", "explore", origin, "package-picker", search],
		queryFn: () =>
			backend.registryState.searchPackages({
				query: search || undefined,
				sortBy: search ? "relevance" : "downloads",
				sortDesc: true,
				limit: 20,
			}),
		staleTime: 60_000,
	});
	const found = asArray(results.data?.packages);
	const unknown = value.filter((id) => !found.some((entry) => entry.id === id));
	const selected = useQuery({
		queryKey: ["admin", "explore", origin, "package-names", unknown],
		queryFn: () =>
			backend.registryState.searchPackages({
				ids: unknown,
				limit: unknown.length,
			}),
		enabled: unknown.length > 0,
		staleTime: 60_000,
	});
	const names = new Map(
		[...found, ...asArray(selected.data?.packages)].map((entry) => [
			entry.id,
			entry.name,
		]),
	);
	const toggle = (id: string, name: string) => {
		const next = toggleId(value, id, multiple, limit);
		if (!next) return;
		onPicked(id, name);
		onChange(next);
	};
	return (
		<div className="flex flex-col gap-2">
			<div className="relative">
				<Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
				<Input
					className="pl-8"
					value={query}
					aria-label={t("explorePackageSearch", "Search packages")}
					placeholder={t("explorePackageSearchPlaceholder", "Search packages…")}
					onChange={(event) => setQuery(event.target.value)}
				/>
			</div>
			{value.length > 0 && (
				<ol
					className="flex flex-col gap-1"
					aria-label={t("explorePackagesSelected", "Selected packages")}
				>
					{value.map((id) => (
						<li
							key={id}
							className="flex items-center gap-2 rounded-md bg-primary/10 px-2 py-1 font-mono text-xs"
						>
							<span className="min-w-0 flex-1 truncate">
								{names.get(id) ?? id}
							</span>
							<button
								type="button"
								className="flex size-6 items-center justify-center rounded hover:bg-primary/10"
								aria-label={t("exploreItemRemove", "Remove {{name}}", {
									name: names.get(id) ?? id,
								})}
								onClick={() => onChange(value.filter((entry) => entry !== id))}
							>
								<X className="size-3" />
							</button>
						</li>
					))}
				</ol>
			)}
			<ul className="max-h-60 space-y-0.5 overflow-y-auto rounded-md border p-1">
				{results.isLoading ? (
					<li className="flex items-center gap-2 p-2 text-xs text-muted-foreground">
						<Loader2 className="size-3 animate-spin" />
						{t("explorePackagesLoading", "Loading packages…")}
					</li>
				) : results.isError ? (
					<li className="p-2 text-xs text-destructive">
						{t("explorePackagesError", "Packages could not be loaded.")}
					</li>
				) : found.length ? (
					found.map((entry) => (
						<PickerRow
							key={entry.id}
							checked={value.includes(entry.id)}
							disabled={
								multiple && !value.includes(entry.id) && value.length >= limit
							}
							onToggle={() => toggle(entry.id, entry.name)}
						>
							<span className="min-w-0 flex-1 truncate font-mono text-xs">
								{entry.name}
							</span>
							{entry.verified && (
								<span className="shrink-0 text-[10.5px] text-primary">
									{t("explorePackageVerified", "Verified")}
								</span>
							)}
						</PickerRow>
					))
				) : (
					<li className="p-2 text-xs text-muted-foreground">
						{t("explorePackagesEmpty", "No packages match.")}
					</li>
				)}
			</ul>
		</div>
	);
}

function CollectionPicker({
	layout,
	excludeId,
	value,
	limit,
	onChange,
}: {
	layout: ExploreLayoutDoc;
	excludeId: string;
	value: string[];
	limit: number;
	onChange: (ids: string[]) => void;
}) {
	const { t } = useTranslation("admin");
	const collections = collectionPlacements(layout).filter(
		(placement) => placement.id !== excludeId,
	);
	if (!collections.length) {
		return (
			<p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
				{t(
					"exploreCollectionsEmpty",
					"Add a collection placement first. Unplaced collections work too.",
				)}
			</p>
		);
	}
	return (
		<ul className="max-h-60 space-y-0.5 overflow-y-auto rounded-md border p-1">
			{collections.map((placement) => (
				<PickerRow
					key={placement.id}
					checked={value.includes(placement.id)}
					disabled={!value.includes(placement.id) && value.length >= limit}
					onToggle={() => {
						const next = toggleId(value, placement.id, true, limit);
						if (next) onChange(next);
					}}
				>
					<span className="min-w-0 flex-1 truncate text-sm">
						{placement.name}
					</span>
					{placement.status && <StatusPill status={placement.status} />}
				</PickerRow>
			))}
		</ul>
	);
}

const ACCENT_CATEGORIES: readonly IAppCategory[] = [
	IAppCategory.Lifestyle,
	IAppCategory.Shopping,
	IAppCategory.Photography,
	IAppCategory.Finance,
	IAppCategory.Travel,
	IAppCategory.Business,
	IAppCategory.Communication,
	IAppCategory.Entertainment,
];

function AccentSwatches({
	value,
	onChange,
}: {
	value: ExploreAccent | null | undefined;
	onChange: (accent: ExploreAccent | null) => void;
}) {
	const { t } = useTranslation("admin");
	const categoryLabel = useAppCategoryLabel();
	const current = value ?? "auto";
	return (
		<fieldset
			aria-label={t("exploreAccent", "Accent")}
			className="flex min-w-0 flex-wrap gap-2"
		>
			<button
				type="button"
				aria-pressed={current === "auto"}
				aria-label={t("exploreAccentAuto", "Accent from the item's category")}
				title={t("exploreAccentAuto", "Accent from the item's category")}
				onClick={() => onChange(null)}
				className={cn(
					"flex size-5.5 items-center justify-center rounded-full border border-dashed text-[9px] font-semibold text-muted-foreground",
					current === "auto" &&
						"border-solid border-foreground text-foreground ring-2 ring-ring ring-offset-2 ring-offset-background",
				)}
			>
				A
			</button>
			{ACCENT_CATEGORIES.map((category) => {
				const accent: ExploreAccent = `category:${category}`;
				const label = t("exploreAccentCategory", "{{category}} accent", {
					category: categoryLabel(category),
				});
				return (
					<button
						key={category}
						type="button"
						aria-pressed={current === accent}
						aria-label={label}
						title={label}
						onClick={() => onChange(accent)}
						className={cn(
							"size-5.5 rounded-full",
							current === accent &&
								"ring-2 ring-ring ring-offset-2 ring-offset-background",
						)}
						style={{ background: categoryColor(category) }}
					/>
				);
			})}
		</fieldset>
	);
}

/** Headline, subline, artwork and accent overrides of one slide or featured item. */
export function ItemOverridesEditor({
	item,
	index,
	name,
	itemRef,
	issueFor,
	artwork,
	onPatch,
}: {
	item: ExplorePlacementItem;
	index: number;
	name: string;
	itemRef?: ExploreItemRef;
	issueFor: (field: string) => string | undefined;
	artwork: {
		available: boolean | undefined;
		upload: (file: Blob) => Promise<string>;
	};
	/** Applied to the latest draft, so an upload that finishes later never reverts newer edits. */
	onPatch: (patch: Partial<Omit<ExplorePlacementItem, "kind" | "id">>) => void;
}) {
	const { t } = useTranslation("admin");
	const [replacing, setReplacing] = useState(false);
	const label = item.artworkUrl
		? t("exploreArtworkCustom", "Custom artwork")
		: itemRef?.coverUrl || itemRef?.iconUrl
			? t("exploreArtworkCover", "Item cover")
			: t("exploreArtworkGenerated", "Generated");
	return (
		<>
			<InspectorSection
				title={t("exploreCopyFor", "Copy for {{name}}", { name })}
			>
				<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
					<CountedField
						label={t("exploreHeadlineOverride", "Headline override")}
						value={item.headline}
						max={EXPLORE_LIMITS.headline}
						placeholder={t(
							"exploreHeadlinePlaceholder",
							"{{name}} (item name)",
							{
								name,
							},
						)}
						error={issueFor(`items.${index}.headline`)}
						onChange={(headline) => onPatch({ headline })}
					/>
					<CountedField
						label={t("exploreSublineOverride", "Subline override")}
						value={item.subline}
						max={EXPLORE_LIMITS.subline}
						multiline
						placeholder={t(
							"exploreSublinePlaceholder",
							"Uses the item's description",
						)}
						error={issueFor(`items.${index}.subline`)}
						onChange={(subline) => onPatch({ subline })}
					/>
				</div>
			</InspectorSection>
			<InspectorSection title={t("exploreLook", "Look")}>
				<div className="flex gap-3.5">
					<div className="relative h-17 w-30 shrink-0 overflow-hidden rounded-lg border bg-muted">
						<ItemThumb
							item={item}
							name={name}
							itemRef={itemRef}
							className="size-full rounded-none border-0"
						/>
						<span className="absolute bottom-1.25 left-1.25 rounded bg-black/60 px-1.25 py-0.5 text-[10px] text-white">
							{label}
						</span>
					</div>
					<div className="flex min-w-0 flex-1 flex-col gap-2">
						<span className="text-xs text-muted-foreground">
							{t("exploreAccent", "Accent")}
						</span>
						<AccentSwatches
							value={item.accent}
							onChange={(accent) => onPatch({ accent })}
						/>
						<div className="flex flex-wrap gap-1.5">
							<Button
								type="button"
								variant="secondary"
								size="sm"
								className="h-6.5 px-2.5 text-xs"
								disabled={artwork.available === false}
								aria-expanded={replacing}
								onClick={() => setReplacing((value) => !value)}
							>
								{t("exploreReplaceArtwork", "Replace artwork")}
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="h-6.5 px-2 text-xs text-muted-foreground"
								disabled={!item.artworkUrl}
								onClick={() => onPatch({ artworkUrl: null })}
							>
								{t("exploreUseItemCover", "Use item cover")}
							</Button>
						</div>
					</div>
				</div>
				{artwork.available === false && (
					<p className="text-[11.5px] leading-snug text-muted-foreground">
						{t(
							"exploreArtworkNoCdn",
							"This hub has no CDN for Explore artwork. Use the item cover instead.",
						)}
					</p>
				)}
				{replacing && artwork.available !== false && (
					<ProfileMediaField
						label={t("exploreArtwork", "Artwork")}
						kind="cover"
						value={item.artworkUrl}
						upload={artwork.upload}
						onChange={(artworkUrl) => onPatch({ artworkUrl })}
					/>
				)}
				<FieldError
					id={`artwork-${index}-error`}
					message={issueFor(`items.${index}.artworkUrl`)}
				/>
			</InspectorSection>
		</>
	);
}
