"use client";

import { useTranslation } from "@flow-like/locales";
import { useDraggable } from "@dnd-kit/core";
import {
	ChevronRight,
	Grid3X3,
	Layers,
	List,
	Loader2,
	Search,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useInvoke } from "../../hooks";
import { useSearch } from "../../hooks/use-search-index";
import { cn } from "../../lib";
import { useBackend } from "../../state/backend-state";
import type { IUserWidgetInfo } from "../../state/backend-state/user-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { Input } from "../ui/input";
import { ScrollArea } from "../ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { WIDGET_DND_TYPE, type WidgetDragData } from "./BuilderDndContext";

export interface WidgetSelectorProps {
	currentAppId?: string;
	className?: string;
	onSelectWidget?: (appId: string, widgetId: string) => void;
	onDragStart?: (appId: string, widgetId: string) => void;
}

interface GroupedWidgets {
	appId: string;
	appName: string;
	widgets: IUserWidgetInfo[];
}

export function WidgetSelector({
	currentAppId,
	className,
	onSelectWidget,
	onDragStart,
}: WidgetSelectorProps) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const [searchQuery, setSearchQuery] = useState("");
	const [viewMode, setViewMode] = useState<"list" | "grid">("list");
	const [openApps, setOpenApps] = useState<Set<string>>(
		new Set(currentAppId ? [currentAppId] : []),
	);
	const [widgetProjectsInitialized, setWidgetProjectsInitialized] =
		useState(false);
	const [selectedTab, setSelectedTab] = useState<"current" | "all">(
		currentAppId ? "current" : "all",
	);
	const [previewWidget, setPreviewWidget] = useState<IUserWidgetInfo | null>(
		null,
	);

	const {
		data: widgets,
		isLoading,
		isError,
		refetch,
	} = useInvoke(backend.userState.getUserWidgets, backend.userState, []);

	const { data: apps } = useInvoke(
		backend.appState.getApps,
		backend.appState,
		[],
	);

	const appNameById = useMemo(() => {
		const names = new Map<string, string>();
		for (const [app, metadata] of apps ?? []) {
			const appName =
				typeof metadata?.name === "string" ? metadata.name.trim() : "";
			if (app?.id && appName) {
				names.set(app.id, appName);
			}
		}
		return names;
	}, [apps]);

	const namedWidgets = useMemo<IUserWidgetInfo[]>(() => {
		if (!widgets) return [];

		return widgets.flatMap((widget) => {
			const name = getWidgetDisplayName(widget);
			if (!name) return [];

			return [
				{
					...widget,
					metadata: {
						...widget.metadata,
						name,
					},
				},
			];
		});
	}, [widgets]);

	const groupedWidgets = useMemo<GroupedWidgets[]>(() => {
		if (!namedWidgets.length) return [];

		const groups = new Map<string, IUserWidgetInfo[]>();

		for (const widget of namedWidgets) {
			const existing = groups.get(widget.appId) || [];
			existing.push(widget);
			groups.set(widget.appId, existing);
		}

		return Array.from(groups.entries())
			.map(([appId, appWidgets]) => ({
				appId,
				appName: getProjectDisplayName(appId, appNameById, currentAppId),
				widgets: appWidgets.sort((a, b) =>
					a.metadata.name.localeCompare(b.metadata.name),
				),
			}))
			.sort((a, b) => {
				if (a.appId === currentAppId) return -1;
				if (b.appId === currentAppId) return 1;
				return a.appName.localeCompare(b.appName);
			});
	}, [namedWidgets, appNameById, currentAppId]);

	useEffect(() => {
		if (widgetProjectsInitialized || groupedWidgets.length === 0) return;

		setOpenApps(new Set(groupedWidgets.map((group) => group.appId)));
		setWidgetProjectsInitialized(true);
	}, [groupedWidgets, widgetProjectsInitialized]);

	const currentAppWidgets = useMemo(() => {
		if (!currentAppId) return [];
		return groupedWidgets.find((g) => g.appId === currentAppId)?.widgets || [];
	}, [groupedWidgets, currentAppId]);

	const otherAppsWidgets = useMemo(() => {
		if (!currentAppId) return groupedWidgets;
		return groupedWidgets.filter((g) => g.appId !== currentAppId);
	}, [groupedWidgets, currentAppId]);

	const matchedWidgets = useSearch(namedWidgets, searchQuery, {
		fields: ["metadata.name", "metadata.description", "metadata.tags"],
		boost: { "metadata.name": 3, "metadata.tags": 1.5 },
	});

	const matchedWidgetIds = useMemo(
		() => new Set(matchedWidgets.map((w) => `${w.appId}:${w.widgetId}`)),
		[matchedWidgets],
	);

	const isSearching = searchQuery.trim().length > 0;

	const filteredCurrentWidgets = useMemo(() => {
		if (!isSearching) return currentAppWidgets;
		return currentAppWidgets.filter((w) =>
			matchedWidgetIds.has(`${w.appId}:${w.widgetId}`),
		);
	}, [currentAppWidgets, matchedWidgetIds, isSearching]);

	const filterGroups = useCallback(
		(groups: GroupedWidgets[]) => {
			if (!isSearching) return groups;
			return groups
				.map((group) => ({
					...group,
					widgets: group.widgets.filter((w) =>
						matchedWidgetIds.has(`${w.appId}:${w.widgetId}`),
					),
				}))
				.filter((group) => group.widgets.length > 0);
		},
		[matchedWidgetIds, isSearching],
	);

	const filteredOtherWidgets = useMemo(
		() => filterGroups(otherAppsWidgets),
		[otherAppsWidgets, filterGroups],
	);

	const filteredGroupedWidgets = useMemo(
		() => filterGroups(groupedWidgets),
		[groupedWidgets, filterGroups],
	);

	const setAppOpen = useCallback((appId: string, open: boolean) => {
		setOpenApps((prev) => {
			const next = new Set(prev);
			if (open) {
				next.add(appId);
			} else {
				next.delete(appId);
			}
			return next;
		});
	}, []);

	const handleSelectWidget = useCallback(
		(widget: IUserWidgetInfo) => {
			onSelectWidget?.(widget.appId, widget.widgetId);
		},
		[onSelectWidget],
	);

	if (isLoading) {
		return (
			<div className={cn("flex items-center justify-center p-4", className)}>
				<Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
				<span className="ml-2 text-sm text-muted-foreground">
					{t('loadingWidgets', 'Loading widgets...')}
				</span>
			</div>
		);
	}

	if (isError) {
		return (
			<div className={cn("flex flex-col items-center p-4 gap-2", className)}>
				<p className="text-sm text-muted-foreground">{t('failedToLoadWidgets', 'Failed to load widgets')}</p>
				<Button variant="outline" size="sm" onClick={() => refetch()}>
					{t('retry', 'Retry')}
				</Button>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex flex-col h-full bg-background border-r overflow-hidden",
				className,
			)}
		>
			{/* Header */}
			<div className="p-3 border-b shrink-0 space-y-2">
				<div className="flex items-center justify-between">
					<div className="flex items-center gap-2">
						<Layers className="h-4 w-4 text-muted-foreground" />
						<span className="text-sm font-medium">{t('widgets', 'Widgets')}</span>
					</div>
					<div className="flex items-center gap-1">
						<Button
							variant={viewMode === "list" ? "secondary" : "ghost"}
							size="icon"
							className="h-7 w-7"
							onClick={() => setViewMode("list")}
						>
							<List className="h-3.5 w-3.5" />
						</Button>
						<Button
							variant={viewMode === "grid" ? "secondary" : "ghost"}
							size="icon"
							className="h-7 w-7"
							onClick={() => setViewMode("grid")}
						>
							<Grid3X3 className="h-3.5 w-3.5" />
						</Button>
					</div>
				</div>
				<div className="relative">
					<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t('searchWidgets', 'Search widgets...')}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-8"
					/>
				</div>
			</div>

			{/* Content */}
			{currentAppId ? (
				<Tabs
					value={selectedTab}
					onValueChange={(v) => setSelectedTab(v as "current" | "all")}
					className="flex-1 flex flex-col min-h-0"
				>
					<TabsList className="mx-3 mt-2 grid grid-cols-2">
						<TabsTrigger value="current">{t('currentProject', 'Current Project')}</TabsTrigger>
						<TabsTrigger value="all">{t('allProjects', 'All Projects')}</TabsTrigger>
					</TabsList>

					<TabsContent value="current" className="flex-1 min-h-0 mt-0">
						<ScrollArea className="h-full">
							<div className="p-2 space-y-1">
								{filteredCurrentWidgets.length === 0 ? (
									<div className="p-4 text-center text-sm text-muted-foreground">
										{searchQuery
											? t('noWidgetsMatchYourSearch', 'No widgets match your search')
											: t('noWidgetsInThisProject', 'No widgets in this project')}
									</div>
								) : viewMode === "list" ? (
									filteredCurrentWidgets.map((widget) => (
										<WidgetListItem
											key={widget.widgetId}
											widget={widget}
											onSelect={handleSelectWidget}
											onPreview={setPreviewWidget}
											onDragStart={onDragStart}
										/>
									))
								) : (
									<div className="grid grid-cols-2 gap-2">
										{filteredCurrentWidgets.map((widget) => (
											<WidgetGridItem
												key={widget.widgetId}
												widget={widget}
												onSelect={handleSelectWidget}
												onPreview={setPreviewWidget}
												onDragStart={onDragStart}
											/>
										))}
									</div>
								)}
							</div>
						</ScrollArea>
					</TabsContent>

					<TabsContent value="all" className="flex-1 min-h-0 mt-0">
						<ScrollArea className="h-full">
							<div className="p-2 space-y-1">
								{filteredOtherWidgets.length === 0 ? (
									<div className="p-4 text-center text-sm text-muted-foreground">
										{searchQuery
											? t('noWidgetsMatchYourSearch', 'No widgets match your search')
											: `No widgets from other projects`}
									</div>
								) : (
									filteredOtherWidgets.map((group) => (
										<Collapsible
											key={group.appId}
											open={!!searchQuery.trim() || openApps.has(group.appId)}
											onOpenChange={(open) => setAppOpen(group.appId, open)}
										>
											<CollapsibleTrigger className="flex w-full items-center justify-between p-2 text-sm font-medium text-muted-foreground hover:bg-muted rounded">
												<span className="truncate">{group.appName}</span>
												<div className="flex items-center gap-2">
													<Badge variant="secondary" className="text-xs">
														{group.widgets.length}
													</Badge>
													<ChevronRight
														className={cn(
															"h-4 w-4 transition-transform duration-200",
															(!!searchQuery.trim() ||
																openApps.has(group.appId)) &&
																"rotate-90",
														)}
													/>
												</div>
											</CollapsibleTrigger>
											<CollapsibleContent className="pt-1 space-y-0.5 ml-2">
												{viewMode === "list" ? (
													group.widgets.map((widget) => (
														<WidgetListItem
															key={widget.widgetId}
															widget={widget}
															onSelect={handleSelectWidget}
															onPreview={setPreviewWidget}
															onDragStart={onDragStart}
														/>
													))
												) : (
													<div className="grid grid-cols-2 gap-2">
														{group.widgets.map((widget) => (
															<WidgetGridItem
																key={widget.widgetId}
																widget={widget}
																onSelect={handleSelectWidget}
																onPreview={setPreviewWidget}
																onDragStart={onDragStart}
															/>
														))}
													</div>
												)}
											</CollapsibleContent>
										</Collapsible>
									))
								)}
							</div>
						</ScrollArea>
					</TabsContent>
				</Tabs>
			) : (
				<ScrollArea className="flex-1 min-h-0">
					<div className="p-2 space-y-1">
						{filteredGroupedWidgets.length === 0 ? (
							<div className="p-4 text-center text-sm text-muted-foreground">
								{searchQuery
									? t('noWidgetsMatchYourSearch', 'No widgets match your search')
									: t('noWidgetsAvailable', 'No widgets available')}
							</div>
						) : (
							filteredGroupedWidgets.map((group) => {
								const isOpen =
									!!searchQuery.trim() || openApps.has(group.appId);

								return (
									<Collapsible
										key={group.appId}
										open={isOpen}
										onOpenChange={(open) => setAppOpen(group.appId, open)}
									>
										<CollapsibleTrigger className="flex w-full items-center justify-between p-2 text-sm font-medium text-muted-foreground hover:bg-muted rounded">
											<span className="truncate">{group.appName}</span>
											<div className="flex items-center gap-2">
												<Badge variant="secondary" className="text-xs">
													{group.widgets.length}
												</Badge>
												<ChevronRight
													className={cn(
														"h-4 w-4 transition-transform duration-200",
														isOpen && "rotate-90",
													)}
												/>
											</div>
										</CollapsibleTrigger>
										<CollapsibleContent className="pt-1 space-y-0.5 ml-2">
											{viewMode === "list" ? (
												group.widgets.map((widget) => (
													<WidgetListItem
														key={widget.widgetId}
														widget={widget}
														onSelect={handleSelectWidget}
														onPreview={setPreviewWidget}
														onDragStart={onDragStart}
													/>
												))
											) : (
												<div className="grid grid-cols-2 gap-2">
													{group.widgets.map((widget) => (
														<WidgetGridItem
															key={widget.widgetId}
															widget={widget}
															onSelect={handleSelectWidget}
															onPreview={setPreviewWidget}
															onDragStart={onDragStart}
														/>
													))}
												</div>
											)}
										</CollapsibleContent>
									</Collapsible>
								);
							})
						)}
					</div>
				</ScrollArea>
			)}

			{/* Preview Dialog */}
			<Dialog
				open={!!previewWidget}
				onOpenChange={() => setPreviewWidget(null)}
			>
				<DialogContent className="max-w-lg">
					<DialogHeader>
						<DialogTitle>{previewWidget?.metadata.name}</DialogTitle>
					</DialogHeader>
					{previewWidget && (
						<div className="space-y-4">
							{previewWidget.metadata.thumbnail && (
								<div className="aspect-video bg-muted rounded-lg overflow-hidden">
									<img
										src={previewWidget.metadata.thumbnail}
										alt={previewWidget.metadata.name}
										className="w-full h-full object-cover"
									/>
								</div>
							)}
							<p className="text-sm text-muted-foreground">
								{previewWidget.metadata.description ||
									t('noDescriptionAvailable', 'No description available')}
							</p>
							{previewWidget.metadata.tags.length > 0 && (
								<div className="flex flex-wrap gap-1">
									{previewWidget.metadata.tags.map((tag) => (
										<Badge key={tag} variant="secondary">
											{tag}
										</Badge>
									))}
								</div>
							)}
							<Button
								className="w-full"
								onClick={() => {
									handleSelectWidget(previewWidget);
									setPreviewWidget(null);
								}}
							>
								{t('useWidget', 'Use Widget')}
							</Button>
						</div>
					)}
				</DialogContent>
			</Dialog>
		</div>
	);
}

function getWidgetDisplayName(widget: IUserWidgetInfo): string | null {
	if (!widget?.appId || !widget?.widgetId || !widget?.metadata) return null;

	const name =
		typeof widget.metadata.name === "string" ? widget.metadata.name.trim() : "";
	if (!name || name === widget.widgetId) return null;

	return name;
}

function getProjectDisplayName(
	appId: string,
	appNameById: Map<string, string>,
	currentAppId?: string,
): string {
	const appName = appNameById.get(appId);
	if (appName) return appName;
	return appId === currentAppId ? "Current Project" : "Unnamed Project";
}

interface WidgetItemProps {
	widget: IUserWidgetInfo;
	onSelect: (widget: IUserWidgetInfo) => void;
	onPreview: (widget: IUserWidgetInfo) => void;
	onDragStart?: (appId: string, widgetId: string) => void;
}

function WidgetListItem({
	widget,
	onSelect,
	onPreview,
	onDragStart,
}: WidgetItemProps) {
	const { t } = useTranslation("flow");
	const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
		id: `selector-list-${widget.appId}-${widget.widgetId}`,
		data: {
			type: WIDGET_DND_TYPE,
			appId: widget.appId,
			widgetId: widget.widgetId,
		} satisfies WidgetDragData,
	});

	return (
		<div
			ref={setNodeRef}
			{...listeners}
			{...attributes}
			className={cn(
				"flex items-center gap-2 px-3 py-2 text-sm rounded cursor-grab hover:bg-muted active:cursor-grabbing select-none group touch-none",
				isDragging && "opacity-50",
			)}
			onDoubleClick={() => onSelect(widget)}
			title={widget.metadata.description}
		>
			{widget.metadata.thumbnail ? (
				<img
					src={widget.metadata.thumbnail}
					alt=""
					className="w-8 h-8 rounded object-cover shrink-0"
				/>
			) : (
				<div className="w-8 h-8 rounded bg-muted flex items-center justify-center shrink-0">
					<Layers className="h-4 w-4 text-muted-foreground" />
				</div>
			)}
			<div className="flex-1 min-w-0">
				<p className="truncate font-medium">{widget.metadata.name}</p>
				{widget.metadata.description && (
					<p className="truncate text-xs text-muted-foreground">
						{widget.metadata.description}
					</p>
				)}
			</div>
			<Button
				variant="ghost"
				size="icon"
				className="h-7 w-7 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
				onClick={(e) => {
					e.stopPropagation();
					onPreview(widget);
				}}
			>
				<Search className="h-3.5 w-3.5" />
			</Button>
		</div>
	);
}

function WidgetGridItem({
	widget,
	onSelect,
	onPreview,
	onDragStart,
}: WidgetItemProps) {
	const { t } = useTranslation("flow");
	const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
		id: `selector-grid-${widget.appId}-${widget.widgetId}`,
		data: {
			type: WIDGET_DND_TYPE,
			appId: widget.appId,
			widgetId: widget.widgetId,
		} satisfies WidgetDragData,
	});

	return (
		<div
			ref={setNodeRef}
			{...listeners}
			{...attributes}
			className={cn(
				"flex flex-col rounded border cursor-grab hover:bg-muted active:cursor-grabbing select-none group overflow-hidden touch-none",
				isDragging && "opacity-50",
			)}
			onDoubleClick={() => onSelect(widget)}
			title={widget.metadata.description}
		>
			{widget.metadata.thumbnail ? (
				<div className="aspect-video bg-muted">
					<img
						src={widget.metadata.thumbnail}
						alt=""
						className="w-full h-full object-cover"
					/>
				</div>
			) : (
				<div className="aspect-video bg-muted flex items-center justify-center">
					<Layers className="h-8 w-8 text-muted-foreground/50" />
				</div>
			)}
			<div className="p-2">
				<p className="truncate text-sm font-medium">{widget.metadata.name}</p>
			</div>
		</div>
	);
}

export default WidgetSelector;
