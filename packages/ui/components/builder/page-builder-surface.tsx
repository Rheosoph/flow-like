"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowLeft,
	Check,
	Loader2,
	Save,
	Settings,
	Workflow,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { cn } from "../../lib";
import { useBackend } from "../../state/backend-state";
import type {
	IPage,
	IWidgetRef,
	PageLayoutType,
} from "../../state/backend-state/page-state";
import type { SurfaceComponent } from "../a2ui/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { ScrollArea } from "../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Separator } from "../ui/separator";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "../ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { Textarea } from "../ui/textarea";
import { WidgetBuilder } from "./WidgetBuilder";

const AUTO_SAVE_DELAY = 2000; // 2 seconds debounce for components
const METADATA_SAVE_DELAY = 1000; // 1 second debounce for metadata/canvas settings (shorter to reduce data loss)

const LAYOUT_TYPES: {
	value: PageLayoutType;
	label: string;
	description: string;
}[] = [
	{
		value: "freeform",
		label: "Freeform",
		description: "Free positioning of elements",
	},
	{ value: "stack", label: "Stack", description: "Vertical stacking layout" },
	{ value: "grid", label: "Grid", description: "Grid-based layout" },
	{
		value: "sidebar",
		label: "Sidebar",
		description: "Main content with sidebar",
	},
	{
		value: "holyGrail",
		label: "Holy Grail",
		description: "Classic web layout pattern",
	},
];

export interface PageBuilderSurfaceProps {
	appId: string;
	pageId: string;
	boardId?: string;
	className?: string;
	/** Renders the back button when provided. */
	onClose?: () => void;
	/** "Open Flow" button in the header. Only rendered when a boardId is known. */
	onOpenFlow?: () => void;
	/** Cmd/Ctrl+Shift+F shortcut. Only bound when a boardId is known. */
	onOpenBoard?: () => void;
	/** The user picked another page in the builder's page switcher. */
	onPageChange?: (pageId: string) => void;
}

export function PageBuilderSurface({
	appId,
	pageId,
	boardId,
	className,
	onClose,
	onOpenFlow,
	onOpenBoard,
	onPageChange,
}: Readonly<PageBuilderSurfaceProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();

	// Fetch all pages for the app (for action context)
	const allPages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		!!appId,
		[appId],
	);

	// Fetch board to extract workflow events (simple event nodes)
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId ?? ""],
		!!appId && !!boardId,
		[appId, boardId],
	);

	const [page, setPage] = useState<IPage | null>(null);
	const [isLoading, setIsLoading] = useState(true);
	const [isSaving, setIsSaving] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
	const [lastSavedAt, setLastSavedAt] = useState<Date | null>(null);

	// Track last saved components for diff comparison
	const lastSavedComponentsRef = useRef<string>("");
	const lastSavedWidgetRefsRef = useRef<string>("{}");
	const autoSaveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const autoSaveMetadataTimeoutRef = useRef<ReturnType<
		typeof setTimeout
	> | null>(null);
	// Store page in ref for use in callbacks without causing re-renders
	const pageRef = useRef<IPage | null>(null);

	// Build action context for the widget builder (pages, workflow events, behavior hooks)
	const actionContext = useMemo(() => {
		const pages =
			allPages.data?.map((pageInfo) => ({
				id: pageInfo.pageId,
				name: pageInfo.name || pageInfo.pageId,
			})) ?? [];

		const workflowEvents = board.data?.nodes
			? Object.values(board.data.nodes)
					.filter((node) => node.name === "events_simple")
					.map((node) => ({
						nodeId: node.id,
						name:
							node.friendly_name ||
							node.comment ||
							t("unnamedEvent", "Unnamed Event"),
					}))
			: [];

		return {
			appId,
			boardId,
			pages,
			workflowEvents,
			// Pass behavior hooks for preview mode
			pageId: page?.id,
			onLoadEventId: page?.onLoadEventId,
			onUnloadEventId: page?.onUnloadEventId,
			onIntervalEventId: page?.onIntervalEventId,
			onIntervalSeconds: page?.onIntervalSeconds,
		};
	}, [appId, boardId, page, allPages.data, board.data?.nodes, t]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `t` only names a page that does not exist yet; re-running on a language switch would reload over the working copy.
	useEffect(() => {
		const loadPage = async () => {
			if (!pageId || !appId) {
				setIsLoading(false);
				return;
			}

			try {
				const loadedPage = await backend.pageState.getPage(
					appId,
					pageId,
					boardId,
				);
				// Ensure boardId is set on the loaded page
				const pageWithBoard = {
					...loadedPage,
					boardId: loadedPage.boardId || boardId,
				};
				setPage(pageWithBoard);
				pageRef.current = pageWithBoard;
				// Store initial state for diff comparison
				lastSavedComponentsRef.current = JSON.stringify(
					loadedPage.components ?? [],
				);
				lastSavedWidgetRefsRef.current = JSON.stringify(
					loadedPage.widgetRefs ?? {},
				);
			} catch {
				const newPage: IPage = {
					id: pageId,
					name: t("newPage", "New Page"),
					content: [],
					layoutType: "freeform",
					components: [],
					createdAt: new Date().toISOString(),
					updatedAt: new Date().toISOString(),
					boardId, // Set boardId for new pages
				};
				setPage(newPage);
				pageRef.current = newPage;
				lastSavedComponentsRef.current = "[]";
				lastSavedWidgetRefsRef.current = "{}";
			} finally {
				setIsLoading(false);
			}
		};

		loadPage();
	}, [pageId, appId, boardId, backend.pageState]);

	// Save pending changes on unmount instead of just cancelling
	useEffect(() => {
		return () => {
			// If there's a pending auto-save, execute it immediately
			if (autoSaveTimeoutRef.current) {
				clearTimeout(autoSaveTimeoutRef.current);
				autoSaveTimeoutRef.current = null;
			}
			if (autoSaveMetadataTimeoutRef.current) {
				clearTimeout(autoSaveMetadataTimeoutRef.current);
				autoSaveMetadataTimeoutRef.current = null;
				// Save immediately on unmount if we have pending metadata changes
				const currentPage = pageRef.current;
				if (currentPage && appId) {
					const pageToSave = {
						...currentPage,
						updatedAt: new Date().toISOString(),
					};
					backend.pageState.updatePage(appId, pageToSave).catch((error) => {
						console.error("Failed to save page on unmount:", error);
					});
				}
			}
		};
	}, [appId, backend.pageState]);

	// Keyboard shortcut to jump to flow editor (Cmd/Ctrl+Shift+F)
	useEffect(() => {
		if (!boardId || !onOpenBoard) return;
		const handleKeyDown = (e: KeyboardEvent) => {
			if (
				(e.metaKey || e.ctrlKey) &&
				e.shiftKey &&
				e.key.toLowerCase() === "f"
			) {
				e.preventDefault();
				onOpenBoard();
			}
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, [boardId, onOpenBoard]);

	const performSave = useCallback(
		async (
			components: SurfaceComponent[],
			widgetRefs?: Record<string, IWidgetRef>,
		) => {
			const currentPage = pageRef.current;
			if (!currentPage || !appId) return;

			const componentsJson = JSON.stringify(components);
			const nextWidgetRefs = widgetRefs ?? currentPage.widgetRefs ?? {};
			const widgetRefsJson = JSON.stringify(nextWidgetRefs);
			// Skip if no changes
			if (
				componentsJson === lastSavedComponentsRef.current &&
				widgetRefsJson === lastSavedWidgetRefsRef.current
			) {
				return;
			}

			setIsSaving(true);
			try {
				const updatedPage = {
					...currentPage,
					components,
					widgetRefs: nextWidgetRefs,
					updatedAt: new Date().toISOString(),
				};
				await backend.pageState.updatePage(appId, updatedPage);
				pageRef.current = updatedPage;
				setPage(updatedPage);
				lastSavedComponentsRef.current = componentsJson;
				lastSavedWidgetRefsRef.current = widgetRefsJson;
				setHasUnsavedChanges(false);
				setLastSavedAt(new Date());
			} catch (error) {
				console.error("Failed to save page:", error);
			} finally {
				setIsSaving(false);
			}
		},
		[appId, backend.pageState],
	);

	// Handle component changes from WidgetBuilder - triggers auto-save
	const handleComponentsChange = useCallback(
		(
			components: SurfaceComponent[],
			widgetRefs?: Record<string, IWidgetRef>,
		) => {
			// Check if there are actual changes using ref (avoid state dependency)
			const componentsJson = JSON.stringify(components);
			const widgetRefsJson = JSON.stringify(widgetRefs ?? {});
			if (
				componentsJson === lastSavedComponentsRef.current &&
				widgetRefsJson === lastSavedWidgetRefsRef.current
			) {
				return; // No changes, skip
			}

			setHasUnsavedChanges(true);

			// Clear existing auto-save timeout
			if (autoSaveTimeoutRef.current) {
				clearTimeout(autoSaveTimeoutRef.current);
			}

			// Schedule auto-save with debounce
			autoSaveTimeoutRef.current = setTimeout(() => {
				performSave(components, widgetRefs);
			}, AUTO_SAVE_DELAY);
		},
		[performSave],
	);

	// Manual save (immediate, from toolbar)
	const handleSave = useCallback(
		async (
			components: SurfaceComponent[],
			widgetRefs?: Record<string, IWidgetRef>,
		) => {
			// Clear any pending auto-save
			if (autoSaveTimeoutRef.current) {
				clearTimeout(autoSaveTimeoutRef.current);
				autoSaveTimeoutRef.current = null;
			}
			await performSave(components, widgetRefs);
		},
		[performSave],
	);

	const updatePageProperty = useCallback(
		<K extends keyof IPage>(key: K, value: IPage[K]) => {
			let updatedPage: IPage | null = null;

			setPage((prev) => {
				if (!prev) return prev;
				updatedPage = { ...prev, [key]: value };
				pageRef.current = updatedPage;
				return updatedPage;
			});

			// Schedule auto-save for metadata changes
			if (autoSaveMetadataTimeoutRef.current) {
				clearTimeout(autoSaveMetadataTimeoutRef.current);
			}
			autoSaveMetadataTimeoutRef.current = setTimeout(async () => {
				// Read current page from ref at execution time
				const currentPage = pageRef.current;
				if (!currentPage || !appId) return;

				setIsSaving(true);
				try {
					const pageToSave = {
						...currentPage,
						updatedAt: new Date().toISOString(),
					};
					await backend.pageState.updatePage(appId, pageToSave);
					pageRef.current = pageToSave;
					setPage(pageToSave);
					setLastSavedAt(new Date());
				} catch (error) {
					console.error("Failed to save page metadata:", error);
				} finally {
					setIsSaving(false);
				}
			}, METADATA_SAVE_DELAY);
		},
		[appId, backend.pageState],
	);

	const handleSaveMetadata = useCallback(async () => {
		// Clear any pending auto-save
		if (autoSaveMetadataTimeoutRef.current) {
			clearTimeout(autoSaveMetadataTimeoutRef.current);
			autoSaveMetadataTimeoutRef.current = null;
		}

		// Read current page from ref
		const currentPage = pageRef.current;
		if (!currentPage || !appId) return;

		setIsSaving(true);
		try {
			const pageToSave = {
				...currentPage,
				updatedAt: new Date().toISOString(),
			};
			await backend.pageState.updatePage(appId, pageToSave);
			pageRef.current = pageToSave;
			setPage(pageToSave);
			setLastSavedAt(new Date());
		} catch (error) {
			console.error("Failed to save page metadata:", error);
		} finally {
			setIsSaving(false);
		}
	}, [appId, backend.pageState]);

	if (!pageId) {
		return (
			<div
				className={cn(
					"flex items-center justify-center h-full min-h-0",
					className,
				)}
			>
				<p className="text-muted-foreground">
					{t("pageNotFound", "Page not found")}
				</p>
			</div>
		);
	}

	if (isLoading) {
		return (
			<div
				className={cn(
					"flex items-center justify-center h-full min-h-0 gap-2",
					className,
				)}
			>
				<Loader2 className="h-5 w-5 animate-spin" />
				<p className="text-muted-foreground">
					{t("loadingPage", "Loading page...")}
				</p>
			</div>
		);
	}

	if (!page) {
		return (
			<div
				className={cn(
					"flex items-center justify-center h-full min-h-0",
					className,
				)}
			>
				<p className="text-muted-foreground">
					{t("pageNotFound", "Page not found")}
				</p>
			</div>
		);
	}

	return (
		<div className={cn("flex flex-col h-full min-h-0", className)}>
			{/* Header */}
			<div className="flex items-center justify-between px-4 py-3 border-b bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/60">
				<div className="flex items-center gap-4">
					{onClose && (
						<Button
							variant="ghost"
							size="icon"
							onClick={onClose}
							aria-label={t("back", "Back")}
						>
							<ArrowLeft className="h-4 w-4" />
						</Button>
					)}
					<div>
						<h1 className="text-lg font-semibold">{page.name}</h1>
						<p className="text-sm text-muted-foreground">
							{t("visualPageBuilder", "Visual Page Builder")}
						</p>
					</div>
					{page.version && (
						<Badge variant="secondary">
							v{page.version[0]}.{page.version[1]}.{page.version[2]}
						</Badge>
					)}
					<Badge variant="outline">{page.layoutType}</Badge>
				</div>
				<div className="flex items-center gap-2">
					{/* Open Flow button */}
					{boardId && onOpenFlow && (
						<Button variant="outline" size="sm" onClick={onOpenFlow}>
							<Workflow className="h-4 w-4 mr-2" />
							{t("openFlow", "Open Flow")}
						</Button>
					)}
					{/* Save status indicator */}
					{isSaving ? (
						<div className="flex items-center gap-1.5 text-sm text-muted-foreground">
							<Loader2 className="h-3 w-3 animate-spin" />
							<span>{t("saving", "Saving...")}</span>
						</div>
					) : hasUnsavedChanges ? (
						<div className="flex items-center gap-1.5 text-sm text-muted-foreground">
							<span className="h-2 w-2 rounded-full bg-yellow-500" />
							<span>{t("unsavedChanges", "Unsaved changes")}</span>
						</div>
					) : lastSavedAt ? (
						<div className="flex items-center gap-1.5 text-sm text-muted-foreground">
							<Check className="h-3 w-3 text-green-500" />
							<span>{t("saved", "Saved")}</span>
						</div>
					) : null}
					<Button
						variant="outline"
						size="sm"
						onClick={() => setShowSettings(!showSettings)}
					>
						<Settings className="h-4 w-4 mr-2" />
						{t("settings", "Settings")}
					</Button>
				</div>
			</div>

			{/* Main Content */}
			<div className="flex-1 min-h-0 flex">
				{/* Page Builder using WidgetBuilder */}
				<div className="flex-1 min-h-0">
					<WidgetBuilder
						initialComponents={page.components}
						initialWidgetRefs={page.widgetRefs}
						widgetId={page.id}
						surfaceId={`page-${page.id}`}
						onSave={handleSave}
						onChange={handleComponentsChange}
						className="h-full"
						externalAssistant
						actionContext={actionContext}
						currentPageId={pageId}
						onPageChange={(newPageId) => {
							if (newPageId === pageId) return; // Skip if same page
							onPageChange?.(newPageId);
						}}
						initialCanvasSettings={{
							backgroundColor: page.canvasSettings?.backgroundColor,
							backgroundImage: page.canvasSettings?.backgroundImage,
							padding: page.canvasSettings?.padding,
							customCss: page.canvasSettings?.customCss,
						}}
						onCanvasSettingsChange={(settings) => {
							updatePageProperty("canvasSettings", {
								backgroundColor: settings.backgroundColor,
								backgroundImage: settings.backgroundImage,
								padding: settings.padding,
								customCss: settings.customCss,
							});
						}}
					/>
				</div>

				{/* Settings Sheet */}
				<Sheet open={showSettings} onOpenChange={setShowSettings}>
					<SheetContent className="w-96 sm:max-w-md overflow-hidden flex flex-col">
						<SheetHeader>
							<SheetTitle>{t("pageSettings", "Page Settings")}</SheetTitle>
						</SheetHeader>
						<ScrollArea className="flex-1 -mx-6 px-6">
							<PageSettingsPanel
								page={page}
								onUpdatePage={updatePageProperty}
								onSave={handleSaveMetadata}
								isSaving={isSaving}
								workflowEvents={actionContext.workflowEvents}
							/>
						</ScrollArea>
					</SheetContent>
				</Sheet>
			</div>
		</div>
	);
}

function PageSettingsPanel({
	page,
	onUpdatePage,
	onSave,
	isSaving,
	workflowEvents,
}: Readonly<{
	page: IPage;
	onUpdatePage: <K extends keyof IPage>(key: K, value: IPage[K]) => void;
	onSave: () => void;
	isSaving: boolean;
	workflowEvents: { nodeId: string; name: string }[];
}>) {
	const { t } = useTranslation("common");
	const updateMeta = (key: string, value: string | undefined) => {
		const currentMeta = page.meta || { keywords: [] };
		onUpdatePage("meta", { ...currentMeta, [key]: value });
	};

	const updateCanvasSettings = (key: string, value: string | undefined) => {
		const currentSettings = page.canvasSettings || {};
		onUpdatePage("canvasSettings", { ...currentSettings, [key]: value });
	};

	return (
		<Tabs defaultValue="general" className="w-full">
			<TabsList className="w-full justify-start px-4 pt-2">
				<TabsTrigger value="general">{t("general", "General")}</TabsTrigger>
				<TabsTrigger value="behavior">{t("behavior", "Behavior")}</TabsTrigger>
				<TabsTrigger value="layout">{t("layout", "Layout")}</TabsTrigger>
				<TabsTrigger value="seo">SEO</TabsTrigger>
			</TabsList>

			<TabsContent value="general" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label htmlFor="name">{t("pageName", "Page Name")}</Label>
					<Input
						id="name"
						value={page.name}
						onChange={(e) => onUpdatePage("name", e.target.value)}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="description">{t("description", "Description")}</Label>
					<Textarea
						id="description"
						value={page.meta?.description || ""}
						onChange={(e) =>
							updateMeta("description", e.target.value || undefined)
						}
						className="min-h-20"
						placeholder={t(
							"describeWhatThisPageIsFor",
							"Describe what this page is for...",
						)}
					/>
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>{t("pageId", "Page ID")}</Label>
					<Input value={page.id} disabled />
				</div>
				<div className="space-y-2">
					<Label>{t("version", "Version")}</Label>
					<Input
						value={
							page.version
								? `${page.version[0]}.${page.version[1]}.${page.version[2]}`
								: t("notVersioned", "Not versioned")
						}
						disabled
					/>
				</div>
				<Separator />
				<Button onClick={onSave} disabled={isSaving} className="w-full">
					{isSaving ? (
						<Loader2 className="h-4 w-4 mr-2 animate-spin" />
					) : (
						<Save className="h-4 w-4 mr-2" />
					)}
					{t("saveMetadata", "Save Metadata")}
				</Button>
			</TabsContent>

			<TabsContent value="behavior" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label>{t("onPageLoad", "On Page Load")}</Label>
					<Select
						value={page.onLoadEventId || "none"}
						onValueChange={(v) =>
							onUpdatePage("onLoadEventId", v === "none" ? undefined : v)
						}
					>
						<SelectTrigger>
							<SelectValue
								placeholder={t("noEventSelected", "No event selected")}
							/>
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">{t("none", "None")}</SelectItem>
							{workflowEvents.map((event) => (
								<SelectItem key={event.nodeId} value={event.nodeId}>
									{event.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					<p className="text-xs text-muted-foreground">
						{t(
							"executesWhenThePageFirstLoads",
							"Executes when the page first loads",
						)}
					</p>
				</div>

				<div className="space-y-2">
					<Label>{t("onPageUnload", "On Page Unload")}</Label>
					<Select
						value={page.onUnloadEventId || "none"}
						onValueChange={(v) =>
							onUpdatePage("onUnloadEventId", v === "none" ? undefined : v)
						}
					>
						<SelectTrigger>
							<SelectValue
								placeholder={t("noEventSelected", "No event selected")}
							/>
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">{t("none", "None")}</SelectItem>
							{workflowEvents.map((event) => (
								<SelectItem key={event.nodeId} value={event.nodeId}>
									{event.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					<p className="text-xs text-muted-foreground">
						{t(
							"executesWhenNavigatingAwayFromThePage",
							"Executes when navigating away from the page",
						)}
					</p>
				</div>

				<div className="space-y-2">
					<Label>{t("onInterval", "On Interval")}</Label>
					<Select
						value={page.onIntervalEventId || "none"}
						onValueChange={(v) =>
							onUpdatePage("onIntervalEventId", v === "none" ? undefined : v)
						}
					>
						<SelectTrigger>
							<SelectValue
								placeholder={t("noEventSelected", "No event selected")}
							/>
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">{t("none", "None")}</SelectItem>
							{workflowEvents.map((event) => (
								<SelectItem key={event.nodeId} value={event.nodeId}>
									{event.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					{page.onIntervalEventId && (
						<div className="flex items-center gap-2 mt-2">
							<Label className="text-sm whitespace-nowrap">
								{t("every", "Every")}
							</Label>
							<Input
								type="number"
								min={1}
								value={page.onIntervalSeconds || 60}
								onChange={(e) => {
									const value = Number.parseInt(e.target.value, 10);
									if (value > 0) {
										onUpdatePage("onIntervalSeconds", value);
									}
								}}
								className="w-20"
							/>
							<span className="text-sm text-muted-foreground">seconds</span>
						</div>
					)}
					<p className="text-xs text-muted-foreground">
						{t(
							"executesAtAFixedTimeIntervalForPollingrefresh",
							"Executes at a fixed time interval (for polling/refresh)",
						)}
					</p>
				</div>

				{workflowEvents.length === 0 && (
					<p className="text-sm text-muted-foreground">
						{t(
							"noWorkflowEventsAvailableCreateSimpleEventNodesInYourFlowToUseHere",
							"No workflow events available. Create Simple Event nodes in your flow to use here.",
						)}
					</p>
				)}
			</TabsContent>

			<TabsContent value="layout" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label>{t("layoutType", "Layout Type")}</Label>
					<Select
						value={page.layoutType}
						onValueChange={(v) =>
							onUpdatePage("layoutType", v as PageLayoutType)
						}
					>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{LAYOUT_TYPES.map((lt) => (
								<SelectItem key={lt.value} value={lt.value}>
									<div className="flex flex-col">
										<span>{lt.label}</span>
										<span className="text-xs text-muted-foreground">
											{lt.description}
										</span>
									</div>
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>{t("backgroundColor", "Background Color")}</Label>
					<div className="flex gap-2">
						<Input
							type="color"
							value={page.canvasSettings?.backgroundColor || "#ffffff"}
							onChange={(e) =>
								updateCanvasSettings("backgroundColor", e.target.value)
							}
							className="w-12 h-9 p-1"
						/>
						<Input
							value={page.canvasSettings?.backgroundColor || ""}
							onChange={(e) =>
								updateCanvasSettings(
									"backgroundColor",
									e.target.value || undefined,
								)
							}
							placeholder={t(
								"ffffffOrBgbackground",
								"#ffffff or bg-background",
							)}
							className="flex-1"
						/>
					</div>
				</div>
				<div className="space-y-2">
					<Label>{t("backgroundImage", "Background Image")}</Label>
					<Input
						value={page.canvasSettings?.backgroundImage || ""}
						onChange={(e) =>
							updateCanvasSettings(
								"backgroundImage",
								e.target.value || undefined,
							)
						}
						placeholder={t("urlOrStoragePath", "URL or storage:// path")}
					/>
				</div>
				<div className="space-y-2">
					<Label>{t("padding", "Padding")}</Label>
					<Input
						value={page.canvasSettings?.padding || ""}
						onChange={(e) =>
							updateCanvasSettings("padding", e.target.value || undefined)
						}
						placeholder="1rem"
					/>
				</div>
			</TabsContent>

			<TabsContent value="seo" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label htmlFor="title">{t("seoTitle", "SEO Title")}</Label>
					<Input
						id="title"
						value={page.title || ""}
						onChange={(e) => onUpdatePage("title", e.target.value || undefined)}
						placeholder={t(
							"pageTitleForSearchEngines",
							"Page title for search engines",
						)}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="seo-description">
						{t("metaDescription", "Meta Description")}
					</Label>
					<Textarea
						id="seo-description"
						value={page.meta?.description || ""}
						onChange={(e) =>
							updateMeta("description", e.target.value || undefined)
						}
						className="min-h-20"
						placeholder={t(
							"descriptionShownInSearchResults",
							"Description shown in search results",
						)}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="favicon">{t("faviconUrl", "Favicon URL")}</Label>
					<Input
						id="favicon"
						value={page.meta?.favicon || ""}
						onChange={(e) => updateMeta("favicon", e.target.value || undefined)}
						placeholder="/favicon.ico"
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="theme-color">{t("themeColor", "Theme Color")}</Label>
					<div className="flex gap-2">
						<Input
							id="theme-color"
							type="color"
							value={page.meta?.themeColor || "#000000"}
							onChange={(e) => updateMeta("themeColor", e.target.value)}
							className="w-12 h-9 p-1"
						/>
						<Input
							value={page.meta?.themeColor || ""}
							onChange={(e) =>
								updateMeta("themeColor", e.target.value || undefined)
							}
							placeholder="#000000"
							className="flex-1"
						/>
					</div>
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>{t("created", "Created")}</Label>
					<Input value={new Date(page.createdAt).toLocaleString()} disabled />
				</div>
				<div className="space-y-2">
					<Label>{t("lastUpdated", "Last Updated")}</Label>
					<Input value={new Date(page.updatedAt).toLocaleString()} disabled />
				</div>
			</TabsContent>
		</Tabs>
	);
}
