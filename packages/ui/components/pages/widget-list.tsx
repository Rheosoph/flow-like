"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	Grid2X2,
	LayoutGridIcon,
	Loader2,
	MoreVertical,
	Plus,
	Rows3,
	Search,
	Settings,
	Trash2,
} from "lucide-react";
import {
	type CSSProperties,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks";
import { useSearch } from "../../hooks/use-search-index";
import {
	type IMetadata,
	cn,
	formatRelativeTime,
	nowSystemTime,
	useSetQueryParams,
} from "../../lib";
import { useBackend } from "../../state/backend-state";
import type { IWidget } from "../../state/backend-state/widget-state";
import type { IDate } from "../../types";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { DataProvider } from "../a2ui/DataContext";
import type { Surface, SurfaceComponent } from "../a2ui/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Textarea } from "../ui/textarea";

type WidgetEntry = [string, string, IMetadata | undefined];
type SortKey = "updated" | "created" | "name";
type Density = "comfortable" | "large";

const DENSITY: Record<Density, { min: number; scale: number }> = {
	comfortable: { min: 264, scale: 0.62 },
	large: { min: 340, scale: 0.8 },
};

/** Width the surface is rendered at before being scaled down into a tile. */
const PREVIEW_WIDTH = 360;

const DOT_GRID: CSSProperties = {
	backgroundImage:
		"radial-gradient(circle at 1px 1px, var(--border) 1px, transparent 0)",
	backgroundSize: "12px 12px",
};

function secondsOf(date?: IMetadata["created_at"]) {
	return (date as IDate | undefined)?.secs_since_epoch ?? 0;
}

export function WidgetList({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const setQueryParams = useSetQueryParams();
	const [searchTerm, setSearchTerm] = useState("");
	const [sort, setSort] = useState<SortKey>("updated");
	const [density, setDensity] = useState<Density>("comfortable");
	const [activeTag, setActiveTag] = useState<string | null>(null);
	const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
	const [newWidget, setNewWidget] = useState({ name: "", description: "" });

	const widgets = useInvoke(
		backend.widgetState.getWidgets,
		backend.widgetState,
		[appId],
		!!appId,
		[appId],
	);

	// widgets are [appId, widgetId, metadata] tuples
	const matched = useSearch(widgets.data, searchTerm, {
		fields: ["2.name", "2.description", "2.long_description", "2.tags"],
		boost: { "2.name": 3, "2.tags": 1.5 },
	}) as WidgetEntry[];

	const tags = useMemo(() => {
		const counts = new Map<string, number>();
		for (const [, , meta] of widgets.data ?? []) {
			for (const tag of meta?.tags ?? []) {
				counts.set(tag, (counts.get(tag) ?? 0) + 1);
			}
		}
		return Array.from(counts.entries())
			.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
			.slice(0, 8);
	}, [widgets.data]);

	const visible = useMemo(() => {
		const filtered = activeTag
			? matched.filter(([, , meta]) => meta?.tags?.includes(activeTag))
			: matched;

		return [...filtered].sort(([, aId, a], [, bId, b]) => {
			if (sort === "name") {
				return (a?.name ?? aId).localeCompare(b?.name ?? bId);
			}
			const key = sort === "created" ? "created_at" : "updated_at";
			return secondsOf(b?.[key]) - secondsOf(a?.[key]);
		});
	}, [matched, activeTag, sort]);

	const handleCreateWidget = useCallback(async () => {
		if (!appId || !newWidget.name.trim()) {
			toast.error("Please enter a widget name");
			return;
		}

		const widgetId = createId();
		await backend.widgetState.createWidget(
			appId,
			widgetId,
			newWidget.name.trim(),
			newWidget.description.trim() || undefined,
		);
		await backend.widgetState.pushWidgetMeta(appId, widgetId, {
			name: newWidget.name.trim(),
			description: newWidget.description.trim() || "",
			tags: [],
			long_description: "",
			created_at: nowSystemTime(),
			updated_at: nowSystemTime(),
			preview_media: [],
		});
		await widgets.refetch();
		toast.success("Widget created");
		setIsCreateDialogOpen(false);
		setNewWidget({ name: "", description: "" });
		setQueryParams("widgetId", widgetId);
	}, [appId, newWidget, backend.widgetState, widgets, setQueryParams]);

	const handleDeleteWidget = useCallback(
		async (widgetId: string) => {
			if (!appId) return;
			try {
				await backend.widgetState.deleteWidget(appId, widgetId);
				await widgets.refetch();
				toast.success("Widget deleted");
			} catch (error) {
				console.error("Failed to delete widget:", error);
				toast.error("Failed to delete widget");
			}
		},
		[appId, backend.widgetState, widgets],
	);

	const openWidget = useCallback(
		(widgetId: string) => setQueryParams("widgetId", widgetId),
		[setQueryParams],
	);

	const isEmpty = !widgets.isLoading && visible.length === 0;

	return (
		<main className="flex flex-col grow min-h-0 max-h-full p-6 pt-0 overflow-auto md:overflow-visible">
			<div className="sticky top-0 z-20 flex flex-wrap items-center gap-3 border-b bg-background py-3">
				<h1 className="text-base font-semibold">{t('widgets', 'Widgets')}</h1>
				<Badge
					variant="secondary"
					className="font-mono text-[11px] tabular-nums"
				>
					{widgets.data?.length ?? 0}
				</Badge>

				<div className="relative min-w-[200px] flex-1 max-w-sm">
					<Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
					<Input
						placeholder={t('searchNameDescriptionTags', 'Search name, description, tags…')}
						value={searchTerm}
						onChange={(e) => setSearchTerm(e.target.value)}
						className="h-8 pl-8 text-xs"
						aria-label={t('searchWidgets', 'Search widgets')}
					/>
				</div>

				<div className="flex-1" />

				<Select value={sort} onValueChange={(v) => setSort(v as SortKey)}>
					<SelectTrigger
						className="h-8 w-[168px] text-xs"
						aria-label={t('sortWidgets', 'Sort widgets')}
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="updated">{t('recentlyUpdated', 'Recently updated')}</SelectItem>
						<SelectItem value="created">{t('recentlyCreated', 'Recently created')}</SelectItem>
						<SelectItem value="name">Name</SelectItem>
					</SelectContent>
				</Select>

				<div className="flex h-8 items-center overflow-hidden rounded-md border">
					<DensityButton
						active={density === "comfortable"}
						onClick={() => setDensity("comfortable")}
						label={t('comfortableTiles', 'Comfortable tiles')}
					>
						<Grid2X2 className="h-3.5 w-3.5" />
					</DensityButton>
					<div className="h-full w-px bg-border" />
					<DensityButton
						active={density === "large"}
						onClick={() => setDensity("large")}
						label={t('largeTiles', 'Large tiles')}
					>
						<Rows3 className="h-3.5 w-3.5" />
					</DensityButton>
				</div>

				<Button
					size="sm"
					className="h-8"
					onClick={() => setIsCreateDialogOpen(true)}
				>
					<Plus className="mr-1.5 h-3.5 w-3.5" />
					{t('newWidget', 'New widget')}
				</Button>
			</div>

			{tags.length > 0 && (
				<div className="flex flex-wrap items-center gap-1.5 py-3">
					<TagChip
						active={activeTag === null}
						onClick={() => setActiveTag(null)}
					>
						{t('all', 'All')}
					</TagChip>
					{tags.map(([tag, count]) => (
						<TagChip
							key={tag}
							active={activeTag === tag}
							onClick={() => setActiveTag(activeTag === tag ? null : tag)}
						>
							{tag}
							<span className="ml-1.5 font-mono text-[10px] tabular-nums opacity-70">
								{count}
							</span>
						</TagChip>
					))}
				</div>
			)}

			{widgets.isLoading ? (
				<div className="flex items-center justify-center py-12">
					<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
				</div>
			) : isEmpty ? (
				<EmptyState
					searching={!!searchTerm || !!activeTag}
					onCreate={() => setIsCreateDialogOpen(true)}
				/>
			) : (
				<div
					className="grid gap-3.5 pb-6"
					style={{
						gridTemplateColumns: `repeat(auto-fill, minmax(${DENSITY[density].min}px, 1fr))`,
					}}
				>
					{visible.map(([, widgetId, meta]) => (
						<WidgetTile
							key={widgetId}
							appId={appId}
							widgetId={widgetId}
							meta={meta}
							density={density}
							onOpen={openWidget}
							onDelete={handleDeleteWidget}
						/>
					))}
					<button
						type="button"
						onClick={() => setIsCreateDialogOpen(true)}
						className="flex min-h-[160px] flex-col items-center justify-center gap-2 rounded-lg border border-dashed text-sm text-muted-foreground transition-colors hover:border-primary hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						<Plus className="h-5 w-5" />
						{t('newWidget', 'New widget')}
					</button>
				</div>
			)}

			<CreateWidgetDialog
				open={isCreateDialogOpen}
				onOpenChange={setIsCreateDialogOpen}
				value={newWidget}
				onChange={setNewWidget}
				onSubmit={handleCreateWidget}
			/>
		</main>
	);
}

function DensityButton({
	active,
	onClick,
	label,
	children,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	label: string;
	children: React.ReactNode;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			aria-label={label}
			className={cn(
				"flex h-full items-center px-2.5 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
				active && "bg-muted text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function TagChip({
	active,
	onClick,
	children,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	children: React.ReactNode;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			className={cn(
				"rounded-full border px-2.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				active && "border-primary/55 bg-primary/10 text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function WidgetTile({
	appId,
	widgetId,
	meta,
	density,
	onOpen,
	onDelete,
}: Readonly<{
	appId: string;
	widgetId: string;
	meta?: IMetadata;
	density: Density;
	onOpen: (widgetId: string) => void;
	onDelete: (widgetId: string) => void;
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const [ref, inView] = useInView<HTMLDivElement>();

	const widget = useInvoke(
		backend.widgetState.getWidget,
		backend.widgetState,
		[appId, widgetId],
		inView && !!appId && !!widgetId,
	);

	const name = meta?.name || widget.data?.name || widgetId;
	const version = widget.data?.version;
	const propCount = widget.data?.exposedProps?.length ?? 0;
	const elementCount = widget.data?.components?.length ?? 0;
	const updatedAt = meta?.updated_at ?? meta?.created_at;

	const openBuilder = useCallback(() => {
		window.location.href = `/widget?id=${widgetId}&app=${appId}`;
	}, [widgetId, appId]);

	return (
		<div
			ref={ref}
			className="group relative flex flex-col overflow-hidden rounded-lg border bg-card transition-colors hover:border-primary/50"
		>
			<button
				type="button"
				onClick={() => onOpen(widgetId)}
				aria-label={t('openName', 'Open {{name}}', { name })}
				className="absolute inset-0 z-10 rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
			/>

			<div
				className="relative aspect-[4/3] overflow-hidden border-b bg-muted/40"
				style={DOT_GRID}
			>
				<WidgetThumbnail
					widget={widget.data}
					isLoading={widget.isLoading}
					thumbnail={meta?.thumbnail ?? widget.data?.thumbnail}
					scale={DENSITY[density].scale}
				/>

				<div className="absolute inset-x-0 bottom-0 z-20 flex justify-end gap-1.5 bg-gradient-to-t from-background/80 to-transparent p-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
					<Button
						variant="outline"
						size="sm"
						className="h-6 bg-card px-2 text-[11px]"
						onClick={openBuilder}
					>
						<Settings className="mr-1 h-3 w-3" />
						{t('builder', 'Builder')}
					</Button>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								variant="outline"
								size="icon"
								className="h-6 w-6 bg-card"
								aria-label={t('actionsForName', 'Actions for {{name}}', { name })}
							>
								<MoreVertical className="h-3 w-3" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<DropdownMenuItem onClick={() => onOpen(widgetId)}>
								{t('openDetails', 'Open details')}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={openBuilder}>
								<Settings className="mr-2 h-4 w-4" />
								{t('openInBuilder', 'Open in builder')}
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								className="text-destructive focus:text-destructive"
								onClick={() => onDelete(widgetId)}
							>
								<Trash2 className="mr-2 h-4 w-4" />
								{t('delete', 'Delete')}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
			</div>

			<div className="flex flex-col gap-1 px-2.5 py-2.5">
				<div className="flex items-center gap-1.5">
					<span
						className={cn(
							"h-1.5 w-1.5 shrink-0 rounded-full",
							version ? "bg-muted-foreground/50" : "bg-tertiary",
						)}
					/>
					<span className="truncate text-[13px] font-medium">{name}</span>
				</div>
				<div className="flex items-center gap-1.5 font-mono text-[10.5px] text-muted-foreground tabular-nums">
					{version ? (
						<span className="text-foreground/75">v{version.join(".")}</span>
					) : (
						<span className="text-tertiary">{t('draft', 'Draft')}</span>
					)}
					<span className="opacity-40">·</span>
					<span>{t('propcountProps', '{{propCount}} props', { propCount })}</span>
					<span className="opacity-40">·</span>
					<span>{t('elementcountElements', '{{elementCount}} elements', { elementCount })}</span>
					{updatedAt && (
						<>
							<span className="opacity-40">·</span>
							<span className="truncate">
								{formatRelativeTime(updatedAt as IDate, "short")}
							</span>
						</>
					)}
				</div>
			</div>
		</div>
	);
}

function WidgetThumbnail({
	widget,
	isLoading,
	thumbnail,
	scale,
}: Readonly<{
	widget?: IWidget;
	isLoading: boolean;
	thumbnail?: string | null;
	scale: number;
}>) {
	const { t } = useTranslation("common");
	const surface = useMemo<Surface | null>(() => {
		if (!widget?.components?.length) return null;
		const components = widget.components.reduce<
			Record<string, SurfaceComponent>
		>((acc, component) => {
			acc[component.id] = component;
			return acc;
		}, {});
		const rootComponentId = components.root
			? "root"
			: widget.rootComponentId && components[widget.rootComponentId]
				? widget.rootComponentId
				: (widget.components[0]?.id ?? "");
		return { id: widget.id, rootComponentId, components };
	}, [widget]);

	if (surface) {
		return (
			<div
				aria-hidden="true"
				className="pointer-events-none absolute left-1/2 top-1/2 origin-center"
				style={{
					width: PREVIEW_WIDTH,
					transform: `translate(-50%, -50%) scale(${scale})`,
				}}
			>
				<DataProvider initialData={widget?.dataModel ?? []}>
					<A2UIRenderer surface={surface} isPreviewMode={true} />
				</DataProvider>
			</div>
		);
	}

	if (thumbnail) {
		return (
			<img
				src={thumbnail}
				alt=""
				className="absolute inset-0 h-full w-full object-cover"
			/>
		);
	}

	return (
		<div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 text-muted-foreground">
			{isLoading ? (
				<Loader2 className="h-5 w-5 animate-spin opacity-50" />
			) : (
				<>
					<LayoutGridIcon className="h-7 w-7 opacity-30" />
					<span className="text-[11px] opacity-70">{t('emptyWidget', 'Empty widget')}</span>
				</>
			)}
		</div>
	);
}

function EmptyState({
	searching,
	onCreate,
}: Readonly<{ searching: boolean; onCreate: () => void }>) {
	const { t } = useTranslation("common");
	return (
		<div className="mx-auto flex max-w-md flex-col items-center gap-4 py-12 text-center">
			<div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary/10">
				<LayoutGridIcon className="h-8 w-8 text-primary" />
			</div>
			<h3 className="text-lg font-medium">
				{searching ? t('noWidgetsFound', 'No widgets found') : t('noWidgetsYet', 'No widgets yet')}
			</h3>
			<p className="text-sm text-muted-foreground">
				{searching
					? t('tryADifferentSearchTermOrClearTheTagFilter', 'Try a different search term or clear the tag filter.')
					: t('widgetsAreTheVisualBuildingBlocksOfYourAppFormsDashboardsChatInterfacesAndMoreCreateOneToStartDesigning', 'Widgets are the visual building blocks of your app — forms, dashboards, chat interfaces, and more. Create one to start designing.')}
			</p>
			{!searching && (
				<Button onClick={onCreate}>
					<Plus className="mr-2 h-4 w-4" />
					{t('createYourFirstWidget', 'Create your first widget')}
				</Button>
			)}
		</div>
	);
}

function CreateWidgetDialog({
	open,
	onOpenChange,
	value,
	onChange,
	onSubmit,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	value: { name: string; description: string };
	onChange: (next: { name: string; description: string }) => void;
	onSubmit: () => void;
}>) {
	const { t } = useTranslation("common");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader className="space-y-3">
					<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
						<LayoutGridIcon className="h-6 w-6 text-primary" />
					</div>
					<DialogTitle className="text-center text-xl">
						{t('createNewWidget', 'Create New Widget')}
					</DialogTitle>
					<DialogDescription className="text-center">
						{t('designAReusableUiComponentForYourApplication', 'Design a reusable UI component for your application')}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-6 py-4">
					<div className="space-y-2">
						<Label htmlFor="widget-name" className="text-sm font-medium">
							{t('widgetName', 'Widget Name')}
						</Label>
						<Input
							id="widget-name"
							placeholder={t('enterWidgetName', 'Enter widget name')}
							value={value.name}
							onChange={(e) => onChange({ ...value, name: e.target.value })}
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="widget-description" className="text-sm font-medium">
							{t('description', 'Description')}
						</Label>
						<Textarea
							id="widget-description"
							placeholder={t('describeWhatThisWidgetDoes', 'Describe what this widget does')}
							value={value.description}
							onChange={(e) =>
								onChange({ ...value, description: e.target.value })
							}
							className="min-h-20 resize-none"
						/>
					</div>

					<div className="flex gap-2 pt-4">
						<Button
							onClick={onSubmit}
							disabled={!value.name.trim()}
							className="flex-1"
						>
							{t('createWidget', 'Create Widget')}
						</Button>
						<Button variant="outline" onClick={() => onOpenChange(false)}>
							{t('cancel', 'Cancel')}
						</Button>
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
}

/**
 * Tiles render a live surface, so only fetch and mount the ones the user can
 * actually reach. Once seen, a tile stays mounted — react-query keeps the
 * payload cached anyway.
 */
function useInView<T extends HTMLElement>() {
	const ref = useRef<T>(null);
	const [inView, setInView] = useState(false);

	useEffect(() => {
		if (inView) return;
		const element = ref.current;
		if (!element) return;
		if (typeof IntersectionObserver === "undefined") {
			setInView(true);
			return;
		}
		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((entry) => entry.isIntersecting)) {
					setInView(true);
					observer.disconnect();
				}
			},
			{ rootMargin: "300px" },
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, [inView]);

	return [ref, inView] as const;
}
