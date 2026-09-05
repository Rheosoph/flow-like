import { useDroppable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import html2canvas from "html2canvas-pro";
import {
	ChevronRight,
	Layers,
	Palette,
	Plus,
	SparklesIcon,
	XIcon,
} from "lucide-react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAssetSource } from "../../hooks/use-asset-source";
import { cn } from "../../lib";
import {
	type AssistantWidgetSurface,
	useAssistantSurface,
} from "../../state/assistant-surface";
import { useBackend } from "../../state/backend-state";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type { IWidget } from "../../state/backend-state/widget-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { useRequestFabBubble } from "../../state/fab-bubble";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { applyA2UIMessage } from "../a2ui/apply-a2ui-message";
import { collectRunElements } from "../a2ui/collect-run-elements";
import type { ElementSource } from "../a2ui/element-materializer";
import { handleElementsRequestMessage } from "../a2ui/elements-request-handler";
import type {
	A2UIClientMessage,
	A2UIComponent,
	A2UIServerMessage,
	Children,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import { handleWidgetQueryMessage } from "../a2ui/widget-query-handler";
import { ScopedCustomCss } from "../scoped-custom-css";
import { Button } from "../ui/button";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../ui/resizable";
import { Sheet, SheetContent } from "../ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { BuilderProvider, useBuilder } from "./BuilderContext";
import {
	BuilderDndProvider,
	type WidgetDragData,
	useBuilderDnd,
} from "./BuilderDndContext";
import { BuilderDragOverlay } from "./BuilderDragOverlay";
import { BuilderRenderer } from "./BuilderRenderer";
import { ComponentPalette } from "./ComponentPalette";
import { DevModePanel } from "./DevModePanel";
import { HierarchyTree } from "./HierarchyTree";
import { Inspector } from "./Inspector";
import { ResponsivePreview } from "./ResponsivePreview";
import { Toolbar } from "./Toolbar";
import { A2UICopilot } from "./a2ui-copilot";
import { useBuilderKeyboardShortcuts } from "./useBuilderKeyboardShortcuts";
export {
	createDefaultComponent,
	getDefaultStyle,
	getDefaultProps,
	normalizeComponent,
	normalizeComponents,
} from "./componentDefaults";

// Re-export DnD types from BuilderDndContext
export {
	COMPONENT_DND_TYPE,
	COMPONENT_MOVE_TYPE,
	WIDGET_DND_TYPE,
	type ComponentDragData as ComponentDragItem,
	type ComponentMoveData as ComponentMoveItem,
	type WidgetDragData as WidgetDragItem,
} from "./BuilderDndContext";

// Container types that can accept children
export const CONTAINER_TYPES = new Set([
	"row",
	"column",
	"stack",
	"grid",
	"card",
	"scrollArea",
	"modal",
	"tabs",
	"accordion",
	"drawer",
	"tooltip",
	"popover",
	"overlay",
	"box",
	"center",
	"absolute",
	"aspectRatio",
]);

// Root component ID constant
export const ROOT_ID = "root";

function isBackgroundClass(value: string | undefined): value is string {
	return value?.startsWith("bg-") ?? false;
}

// Create the default root component
function createRootComponent(): SurfaceComponent {
	return {
		id: ROOT_ID,
		style: {
			className: "flex-1 h-full overflow-auto",
		},
		component: {
			type: "column",
			gap: "8px",
			children: { explicitList: [] },
		} as unknown as A2UIComponent,
	};
}

export interface WidgetBuilderProps {
	className?: string;
	initialComponents?: SurfaceComponent[];
	initialWidgetRefs?: Record<string, IWidgetRef>;
	widgetId?: string;
	surfaceId?: string;
	onSave?: (
		components: SurfaceComponent[],
		widgetRefs?: Record<string, IWidgetRef>,
	) => void;
	onExport?: (components: SurfaceComponent[]) => void;
	onPreview?: () => void;
	onChange?: (
		components: SurfaceComponent[],
		widgetRefs?: Record<string, IWidgetRef>,
	) => void;
	/** Initial canvas settings (background, padding, etc.) */
	initialCanvasSettings?: {
		backgroundColor?: string;
		backgroundImage?: string;
		padding?: string;
		customCss?: string;
	};
	/** Called when canvas settings change */
	onCanvasSettingsChange?: (settings: {
		backgroundColor: string;
		backgroundImage?: string;
		padding: string;
		customCss?: string;
	}) => void;
	/** Context for action editor (pages, events, etc.) */
	actionContext?: {
		appId?: string;
		boardId?: string;
		pageId?: string;
		pages?: { id: string; name: string; boardId?: string }[];
		workflowEvents?: { nodeId: string; name: string }[];
		widgetActions?: { id: string; label: string; description?: string }[];
		eventId?: string;
		onLoadEventId?: string;
		onUnloadEventId?: string;
		onIntervalEventId?: string;
		onIntervalSeconds?: number;
	};
	/** Current page ID for the page switcher */
	currentPageId?: string;
	/** Called when user switches to a different page */
	onPageChange?: (pageId: string) => void;
	/**
	 * When true the host app provides the assistant (global chat) — the FlowPilot button routes to
	 * requestOpenAssistant() and the embedded A2UICopilot panel/sheet are not mounted.
	 */
	externalAssistant?: boolean;
	/**
	 * Receives an imperative handle onto the live component state so the host can rewrite it without
	 * remounting the builder (e.g. renaming a widget action id referenced by components).
	 */
	handleRef?: RefObject<WidgetBuilderHandle | null>;
}

export interface WidgetBuilderHandle {
	getComponents: () => SurfaceComponent[];
	replaceComponents: (components: SurfaceComponent[]) => void;
}

function BuilderHandleBridge({
	handleRef,
}: Readonly<{ handleRef: RefObject<WidgetBuilderHandle | null> }>) {
	const { components, replaceComponents } = useBuilder();

	useEffect(() => {
		handleRef.current = {
			getComponents: () => Array.from(components.values()),
			replaceComponents,
		};
		return () => {
			handleRef.current = null;
		};
	}, [components, replaceComponents, handleRef]);

	return null;
}

export function WidgetBuilder({
	className,
	initialComponents = [],
	initialWidgetRefs,
	widgetId,
	surfaceId = "builder-surface",
	onSave,
	onExport,
	onChange,
	initialCanvasSettings,
	onCanvasSettingsChange,
	actionContext,
	currentPageId,
	onPageChange,
	externalAssistant = false,
	handleRef,
}: WidgetBuilderProps) {
	// Without an in-interface FlowPilot button the floating bubble is this builder's only way into
	// the assistant, so ask for it exactly when we drop our own.
	useRequestFabBubble(externalAssistant);
	const [mode, setMode] = useState<"edit" | "preview">("edit");
	const [leftTab, setLeftTab] = useState<"palette" | "hierarchy">("palette");
	const [copilotOpen, setCopilotOpen] = useState(false);
	const [pendingComponents, setPendingComponents] = useState<
		SurfaceComponent[]
	>([]);

	// Ensure we have a root component
	const componentsWithRoot =
		initialComponents.length > 0 &&
		initialComponents.some((c) => c.id === ROOT_ID)
			? initialComponents
			: [createRootComponent(), ...initialComponents];

	return (
		<BuilderProvider
			initialComponents={componentsWithRoot}
			initialWidgetRefs={initialWidgetRefs}
			onChange={onChange}
			initialCanvasSettings={initialCanvasSettings}
			onCanvasSettingsChange={onCanvasSettingsChange}
			actionContext={actionContext}
		>
			{handleRef && <BuilderHandleBridge handleRef={handleRef} />}
			<WidgetBuilderWithDnd
				className={className}
				surfaceId={surfaceId}
				widgetId={widgetId}
				mode={mode}
				setMode={setMode}
				leftTab={leftTab}
				setLeftTab={setLeftTab}
				copilotOpen={copilotOpen}
				setCopilotOpen={setCopilotOpen}
				pendingComponents={pendingComponents}
				setPendingComponents={setPendingComponents}
				onSave={onSave}
				onExport={onExport}
				currentPageId={currentPageId}
				onPageChange={onPageChange}
				externalAssistant={externalAssistant}
			/>
		</BuilderProvider>
	);
}

interface WidgetBuilderContentProps {
	className?: string;
	surfaceId: string;
	widgetId?: string;
	mode: "edit" | "preview";
	setMode: (mode: "edit" | "preview") => void;
	leftTab: "palette" | "hierarchy";
	setLeftTab: (tab: "palette" | "hierarchy") => void;
	copilotOpen: boolean;
	setCopilotOpen: (open: boolean) => void;
	pendingComponents: SurfaceComponent[];
	setPendingComponents: (components: SurfaceComponent[]) => void;
	onSave?: (
		components: SurfaceComponent[],
		widgetRefs?: Record<string, IWidgetRef>,
	) => void;
	onExport?: (components: SurfaceComponent[]) => void;
	currentPageId?: string;
	onPageChange?: (pageId: string) => void;
	externalAssistant?: boolean;
}

// Wrapper that provides DnD context - must be inside BuilderProvider to access setIsDraggingGlobal
function WidgetBuilderWithDnd(props: WidgetBuilderContentProps) {
	const { setIsDraggingGlobal } = useBuilder();

	return (
		<BuilderDndProvider setIsDraggingGlobal={setIsDraggingGlobal}>
			<BuilderDragOverlay />
			<WidgetBuilderContent {...props} />
		</BuilderDndProvider>
	);
}

function WidgetBuilderContent({
	className,
	surfaceId,
	widgetId,
	mode,
	setMode,
	leftTab,
	setLeftTab,
	copilotOpen,
	setCopilotOpen,
	pendingComponents,
	setPendingComponents,
	onSave,
	onExport,
	currentPageId,
	onPageChange,
	externalAssistant,
}: WidgetBuilderContentProps) {
	const { t } = useTranslation("flow");
	const builder = useBuilder();
	const {
		components,
		selection,
		addComponent,
		updateComponent,
		getComponent,
		widgetRefs,
		actionContext,
		canvasSettings,
		setCanvasSettings,
	} = builder;
	const builderRootRef = useRef<HTMLDivElement>(null);
	const builderId = useId();
	const { activeId } = useBuilderDnd();
	const isDragging = activeId !== null;

	useBuilderKeyboardShortcuts(
		builderRootRef,
		mode === "edit" && !builder.devMode,
		builder,
	);

	// Ref for capturing screenshots of the canvas
	const canvasContainerRef = useRef<HTMLDivElement>(null);

	// Screenshot capture function for FlowPilot
	const captureScreenshot = useCallback(async (): Promise<string | null> => {
		if (!canvasContainerRef.current) return null;
		try {
			const canvas = await html2canvas(canvasContainerRef.current, {
				backgroundColor: null,
				scale: 1,
				logging: false,
				useCORS: true,
			});
			return canvas.toDataURL("image/png");
		} catch (error) {
			console.error("Failed to capture screenshot:", error);
			return null;
		}
	}, []);

	const handleComponentsGenerated = useCallback(
		(newComponents: SurfaceComponent[]) => {
			setPendingComponents(newComponents);
		},
		[setPendingComponents],
	);

	const handleApplyComponents = useCallback(
		(
			_components?: SurfaceComponent[],
			appliedCanvasSettings?: {
				backgroundColor?: string;
				padding?: string;
				customCss?: string;
			},
		) => {
			if (pendingComponents.length === 0) return;

			// Merge, never replace: the copilot omits fields it is not changing, and a plain
			// assignment here dropped the surface's existing customCss whenever it sent back only
			// a backgroundColor. The detached-page write path (`pageWithAppliedComponents`) has
			// always merged — these two must agree or the same emit means different things
			// depending on whether a builder happens to be open.
			if (appliedCanvasSettings) {
				setCanvasSettings({ ...canvasSettings, ...appliedCanvasSettings });
			}

			// Get root component BEFORE adding new components (to avoid stale closure)
			const rootComponent = getComponent(ROOT_ID);

			// Collect all child IDs referenced within the new components
			const referencedChildIds = new Set<string>();
			for (const comp of pendingComponents) {
				const childrenData = (
					comp.component as unknown as Record<string, unknown>
				)?.children as Children | undefined;
				if (childrenData && "explicitList" in childrenData) {
					for (const childId of childrenData.explicitList) {
						referencedChildIds.add(childId);
					}
				}
			}

			// Find top-level components (new components not referenced as children of other new components)
			const topLevelIds: string[] = [];
			for (const comp of pendingComponents) {
				if (!referencedChildIds.has(comp.id) && comp.id !== ROOT_ID) {
					topLevelIds.push(comp.id);
				}
			}

			// Add all components to the map
			for (const comp of pendingComponents) {
				const existing = getComponent(comp.id);
				if (existing) {
					updateComponent(comp.id, comp);
				} else {
					addComponent(comp);
				}
			}

			// Add top-level components to the root's children list
			if (topLevelIds.length > 0 && rootComponent) {
				const rootChildrenData = (
					rootComponent.component as unknown as Record<string, unknown>
				)?.children as Children | undefined;
				const existingChildren =
					rootChildrenData && "explicitList" in rootChildrenData
						? [...rootChildrenData.explicitList]
						: [];

				// Only add IDs that aren't already in the root's children
				const newChildren = [...existingChildren];
				for (const id of topLevelIds) {
					if (!newChildren.includes(id)) {
						newChildren.push(id);
					}
				}

				updateComponent(ROOT_ID, {
					component: {
						...rootComponent.component,
						children: { explicitList: newChildren },
					} as A2UIComponent,
				});
			}

			setPendingComponents([]);
		},
		[
			pendingComponents,
			getComponent,
			updateComponent,
			addComponent,
			setPendingComponents,
			canvasSettings,
			setCanvasSettings,
		],
	);

	const handleDismissComponents = useCallback(() => {
		setPendingComponents([]);
	}, [setPendingComponents]);

	const currentComponents = useMemo(
		() => Array.from(components.values()),
		[components],
	);
	const selectedIds = selection.componentIds;

	// Publish the live widget surface for the global assistant while the builder is mounted.
	useEffect(() => {
		const pageId = actionContext?.pageId ?? currentPageId;
		const kind = pageId ? "page" : "widget";
		const surface: AssistantWidgetSurface = {
			surfaceId,
			kind,
			appId: actionContext?.appId,
			boardId: actionContext?.boardId,
			pageId,
			widgetId: kind === "widget" ? widgetId : undefined,
			currentComponents,
			currentCanvasSettings: canvasSettings,
			selectedComponentIds: selectedIds,
			captureScreenshot,
			applyComponents: handleApplyComponents,
			componentsGenerated: handleComponentsGenerated,
		};
		useAssistantSurface.getState().setWidgetSurface(surface);
		return () => {
			const store = useAssistantSurface.getState();
			if (store.widgetSurface === surface) store.setWidgetSurface(null);
		};
	}, [
		surfaceId,
		widgetId,
		actionContext?.appId,
		actionContext?.boardId,
		actionContext?.pageId,
		currentPageId,
		currentComponents,
		canvasSettings,
		selectedIds,
		captureScreenshot,
		handleApplyComponents,
		handleComponentsGenerated,
	]);

	return (
		<>
			<div
				ref={builderRootRef}
				data-builder-root={builderId}
				tabIndex={-1}
				className={cn(
					"flex min-w-0 flex-col h-full bg-muted/20 overflow-hidden",
					className,
					isDragging && "select-none",
				)}
			>
				{/* Toolbar */}
				<div className="flex min-w-0 items-center gap-1 h-10 px-2 border-b bg-background shrink-0 overflow-x-auto">
					<Toolbar
						onSave={() => {
							const refsRecord = Object.fromEntries(widgetRefs);
							onSave?.(currentComponents, refsRecord);
						}}
						onPreview={() => setMode(mode === "edit" ? "preview" : "edit")}
						pages={actionContext?.pages}
						currentPageId={currentPageId}
						onPageChange={onPageChange}
					/>
					<div className="flex-1" />
					{/* When the host provides the global assistant, the floating FlowPilot bubble is the
					    entry point — only show this in-interface button for the embedded copilot. */}
					{!externalAssistant && (
						<Button
							variant={copilotOpen ? "secondary" : "ghost"}
							size="sm"
							className="h-7 shrink-0 px-2 gap-1.5"
							onClick={() => setCopilotOpen(!copilotOpen)}
						>
							<SparklesIcon className="h-4 w-4" />
							<span className="text-xs">{t("flowpilot", "FlowPilot")}</span>
						</Button>
					)}
				</div>

				{/* Pending components bar */}
				{pendingComponents.length > 0 && (
					<PendingComponentsBar
						components={pendingComponents}
						onApply={handleApplyComponents}
						onDismiss={handleDismissComponents}
					/>
				)}

				{/* Main content */}
				<ResizablePanelGroup
					direction="horizontal"
					className="flex-1 min-h-0 min-w-0 overflow-hidden"
				>
					{/* Left panel - hidden in preview mode */}
					{mode === "edit" && (
						<>
							<ResizablePanel
								defaultSize={20}
								minSize={15}
								maxSize={30}
								className="min-h-0 min-w-0 overflow-hidden"
							>
								<Tabs
									value={leftTab}
									onValueChange={(v) =>
										setLeftTab(v as "palette" | "hierarchy")
									}
									className="h-full flex flex-col min-h-0 min-w-0"
								>
									<TabsList className="w-full justify-start rounded-none border-b bg-transparent px-2 shrink-0">
										<TabsTrigger value="palette" className="gap-1.5">
											<Palette className="h-4 w-4" />
											<span className="hidden sm:inline">
												{t("components", "Components")}
											</span>
										</TabsTrigger>
										<TabsTrigger value="hierarchy" className="gap-1.5">
											<Layers className="h-4 w-4" />
											<span className="hidden sm:inline">
												{t("hierarchy", "Hierarchy")}
											</span>
										</TabsTrigger>
									</TabsList>
									<TabsContent
										value="palette"
										className="flex-1 m-0 min-h-0 overflow-hidden"
									>
										<ComponentPalette className="h-full border-0" />
									</TabsContent>
									<TabsContent
										value="hierarchy"
										className="flex-1 m-0 min-h-0 overflow-hidden"
									>
										<HierarchyTree className="h-full border-0" />
									</TabsContent>
								</Tabs>
							</ResizablePanel>

							<ResizableHandle />
						</>
					)}

					{/* Center: Visual Canvas with live preview */}
					<ResizablePanel
						defaultSize={mode === "preview" ? 100 : copilotOpen ? 40 : 55}
						className="min-h-0 min-w-0 overflow-hidden"
					>
						<div ref={canvasContainerRef} className="h-full w-full">
							{mode === "edit" ? (
								<VisualCanvas surfaceId={surfaceId} />
							) : (
								<ResponsivePreview>
									<BuilderPreview surfaceId={surfaceId} />
								</ResponsivePreview>
							)}
						</div>
					</ResizablePanel>

					{/* Right panel - hidden in preview mode */}
					{mode === "edit" && (
						<>
							<ResizableHandle />

							<ResizablePanel
								defaultSize={copilotOpen ? 40 : 25}
								minSize={20}
								maxSize={50}
								className="min-h-0 min-w-0 overflow-hidden"
							>
								{copilotOpen && !externalAssistant ? (
									<A2UICopilot
										appId={actionContext?.appId}
										currentComponents={currentComponents}
										selectedComponentIds={selectedIds}
										onComponentsGenerated={handleComponentsGenerated}
										onApplyComponents={handleApplyComponents}
										onClose={() => setCopilotOpen(false)}
										className="h-full"
										captureScreenshot={captureScreenshot}
									/>
								) : (
									<Inspector className="h-full border-0" />
								)}
							</ResizablePanel>
						</>
					)}
				</ResizablePanelGroup>

				{/* Mobile FlowPilot Sheet (embedded hosts only) */}
				{!externalAssistant && (
					<Sheet open={copilotOpen} onOpenChange={setCopilotOpen}>
						<SheetContent side="right" className="w-full sm:max-w-md p-0">
							<A2UICopilot
								appId={actionContext?.appId}
								currentComponents={currentComponents}
								selectedComponentIds={selectedIds}
								onComponentsGenerated={handleComponentsGenerated}
								onApplyComponents={handleApplyComponents}
								onClose={() => setCopilotOpen(false)}
								className="h-full"
								captureScreenshot={captureScreenshot}
							/>
						</SheetContent>
					</Sheet>
				)}

				{/* Dev Mode JSON Editor */}
				<DevModePanel />
			</div>
		</>
	);
}

interface PendingComponentsBarProps {
	components: SurfaceComponent[];
	onApply: () => void;
	onDismiss: () => void;
}

function PendingComponentsBar({
	components,
	onApply,
	onDismiss,
}: PendingComponentsBarProps) {
	const { t } = useTranslation("flow");
	return (
		<div className="flex items-center justify-between px-4 py-2 bg-primary/5 border-b border-primary/20 shrink-0">
			<div className="flex items-center gap-2">
				<SparklesIcon className="h-4 w-4 text-primary" />
				<span className="text-sm font-medium">
					{t("countComponents", {
						defaultValue_one: "{{count}} component",
						defaultValue_other: "{{count}} components",
						count: components.length,
					})}{" "}
					{t("readyToApply", "ready to apply")}
				</span>
			</div>
			<div className="flex items-center gap-2">
				<Button
					variant="ghost"
					size="sm"
					onClick={onDismiss}
					className="h-7 px-2 text-muted-foreground hover:text-destructive"
				>
					<XIcon className="h-4 w-4 mr-1" />
					{t("dismiss", "Dismiss")}
				</Button>
				<Button size="sm" onClick={onApply} className="h-7 px-3">
					{t("applyChanges", "Apply Changes")}
				</Button>
			</div>
		</div>
	);
}

// Visual Canvas - shows live preview with drop overlays
function VisualCanvas({ surfaceId }: { surfaceId: string }) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const {
		components,
		selection,
		selectComponent,
		addComponent,
		updateComponent,
		canvasSettings,
		addWidgetRef,
		widgetRefs,
		actionContext,
	} = useBuilder();
	const { activeId } = useBuilderDnd();
	const isDragging = activeId !== null;
	const canvasRef = useRef<HTMLDivElement>(null);
	const canvasId = useId();
	// Components carry storage paths and resolve their own artwork, so the canvas
	// renders them as they are. Only the background has no component to do that.
	const { src: canvasBackgroundImage } = useAssetSource(
		actionContext?.appId,
		canvasSettings.backgroundImage,
	);
	const backgroundClass = isBackgroundClass(canvasSettings.backgroundColor)
		? canvasSettings.backgroundColor
		: undefined;

	// Memoize to prevent unnecessary re-renders when drag state changes
	const surface: Surface = useMemo(
		() => ({
			id: surfaceId,
			rootComponentId: ROOT_ID,
			components: Object.fromEntries(components),
			canvasSettings: canvasSettings.customCss
				? { customCss: canvasSettings.customCss }
				: undefined,
		}),
		[surfaceId, components, canvasSettings.customCss],
	);

	const handleMessage = useCallback((message: A2UIClientMessage) => {
		console.log("Canvas action:", message);
	}, []);

	// Helper to insert a widget instance - copies widget components into page
	const insertWidgetInstance = useCallback(
		async (
			widgetItem: WidgetDragData,
			parentId: string,
			insertIndex?: number,
		) => {
			const { appId, widgetId } = widgetItem;

			// Get parent BEFORE adding components to avoid stale closure
			const parent = components.get(parentId);
			if (!parent) return;

			// Fetch widget data
			let widget: IWidget;

			try {
				widget = await backend.widgetState.getWidget(appId, widgetId);
			} catch (err) {
				console.error("Failed to fetch widget:", err);
				return;
			}

			if (!widget.components?.length || !widget.rootComponentId) {
				console.warn("Widget has no components");
				return;
			}

			// Determine actual root component ID - prefer 'root', then stored value, then first component
			const componentIds = new Set(widget.components.map((c) => c.id));
			const effectiveRootId = componentIds.has("root")
				? "root"
				: componentIds.has(widget.rootComponentId)
					? widget.rootComponentId
					: (widget.components[0]?.id ?? widget.rootComponentId);

			// Create a unique instance ID
			const instanceId = `widget-${widgetId}-${Date.now()}`;
			const widgetInstanceComponentId = `widgetInstance-${instanceId}`;

			// Store the widget definition in refs
			addWidgetRef(instanceId, {
				id: widget.id,
				name: widget.name,
				description: widget.description,
				rootComponentId: effectiveRootId,
				components: widget.components,
				dataModel: widget.dataModel,
				customizationOptions: widget.customizationOptions,
				exposedProps: widget.exposedProps,
				actions: widget.actions,
				tags: widget.tags ?? [],
				catalogId: widget.catalogId,
				thumbnail: widget.thumbnail,
				version: widget.version,
				createdAt: widget.createdAt,
				updatedAt: widget.updatedAt,
			});

			// Create a widgetInstance component that references the widget in refs
			const widgetInstanceComponent: SurfaceComponent = {
				id: widgetInstanceComponentId,
				component: {
					type: "widgetInstance",
					instanceId,
					widgetId,
					appId,
					exposedPropValues: {},
					actionBindings: {},
				} as A2UIComponent,
			};

			// Add the widget instance component
			addComponent(widgetInstanceComponent);

			// Add to parent's children (using captured parent)
			const parentChildren = (
				parent.component as unknown as Record<string, unknown>
			)?.children as Children | undefined;
			const existingChildren =
				parentChildren && "explicitList" in parentChildren
					? [...parentChildren.explicitList]
					: [];

			if (insertIndex !== undefined) {
				existingChildren.splice(insertIndex, 0, widgetInstanceComponentId);
			} else {
				existingChildren.push(widgetInstanceComponentId);
			}

			updateComponent(parentId, {
				component: {
					...parent.component,
					children: { explicitList: existingChildren },
				} as A2UIComponent,
			});
		},
		[
			backend.widgetState,
			components,
			addComponent,
			updateComponent,
			addWidgetRef,
		],
	);

	// Root-level drop target using @dnd-kit
	const { setNodeRef: setDropRef, isOver } = useDroppable({
		id: "canvas-root-drop",
		data: {
			type: "drop-zone",
			parentId: ROOT_ID,
			index: (() => {
				const root = components.get(ROOT_ID);
				if (!root) return 0;
				const childrenData = (
					root.component as unknown as Record<string, unknown>
				).children as Children | undefined;
				return childrenData && "explicitList" in childrenData
					? childrenData.explicitList.length
					: 0;
			})(),
		},
	});

	const handleCanvasClick = useCallback(
		(e: React.MouseEvent) => {
			// Only deselect if clicking the canvas background itself
			if (
				e.target === e.currentTarget ||
				(e.target as HTMLElement).dataset.canvasBackground
			) {
				selectComponent(ROOT_ID, false);
			}
		},
		[selectComponent],
	);

	return (
		<div
			className={cn(
				"h-full min-w-0 flex flex-col bg-muted/30 overflow-hidden",
				isDragging && "select-none",
			)}
			style={{ userSelect: isDragging ? "none" : undefined }}
		>
			{/* Custom CSS injection (scoped and sanitized) */}
			<ScopedCustomCss
				css={canvasSettings.customCss}
				scopeSelector={`[data-canvas-id="${canvasId}"]`}
			/>

			{/* Canvas header with breadcrumb */}
			<div className="flex items-center gap-2 px-3 py-2 border-b bg-background text-xs text-muted-foreground shrink-0">
				<span>{t("canvas", "Canvas")}</span>
				{selection.componentIds.length > 0 &&
					selection.componentIds[0] !== ROOT_ID && (
						<>
							<ChevronRight className="h-3 w-3" />
							<span
								className="min-w-0 truncate text-foreground font-medium"
								title={selection.componentIds[0]}
							>
								{components.get(selection.componentIds[0])?.component.type}
							</span>
						</>
					)}
			</div>

			{/* Canvas area with interactive BuilderRenderer */}
			<div
				ref={(node) => {
					setDropRef(node);
					if (canvasRef)
						(
							canvasRef as React.MutableRefObject<HTMLDivElement | null>
						).current = node;
				}}
				onClick={handleCanvasClick}
				onKeyDown={(e) => e.key === "Escape" && handleCanvasClick(e as never)}
				data-canvas-background="true"
				className={cn(
					"flex-1 overflow-auto p-4 min-w-0 min-h-0",
					isOver && "bg-primary/5",
					isDragging && "select-none",
				)}
				style={{ userSelect: isDragging ? "none" : undefined }}
			>
				<div
					data-canvas-id={canvasId}
					className={cn(
						"min-h-full rounded-lg border shadow-sm relative",
						backgroundClass,
					)}
					style={{
						backgroundColor: backgroundClass
							? undefined
							: canvasSettings.backgroundColor,
						backgroundImage: canvasBackgroundImage
							? `url(${canvasBackgroundImage})`
							: undefined,
						padding: canvasSettings.padding,
					}}
					data-canvas-background="true"
				>
					{/* Editor controls use the rendered elements without adding layout boxes. */}
					<BuilderRenderer surface={surface} className="w-full min-h-full" />

					{/* Empty state */}
					{components.size <= 1 && (
						<div
							className={cn(
								"absolute inset-4 flex items-center justify-center border-2 border-dashed rounded-lg transition-colors pointer-events-none",
								isOver
									? "border-primary bg-primary/10"
									: "border-muted-foreground/30",
							)}
						>
							<div className="text-center text-muted-foreground">
								<Plus className="h-8 w-8 mx-auto mb-2 opacity-50" />
								<p className="text-sm">
									{t(
										"dropComponentsHereToStartBuilding",
										"Drop components here to start building",
									)}
								</p>
							</div>
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

interface BuilderPreviewProps {
	surfaceId: string;
}

function BuilderPreview({ surfaceId }: BuilderPreviewProps) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const { components, canvasSettings, actionContext, widgetRefs } =
		useBuilder();
	const effectiveSurfaceId = actionContext?.pageId ?? surfaceId;
	const previewCanvasId = useId();
	const [previewSurface, setPreviewSurface] = useState<Surface | null>(null);
	// Canvas styling the preview is showing right now: the builder's own settings
	// until a running workflow overrides them with a setCanvasSettings message.
	const [liveCanvasSettings, setLiveCanvasSettings] = useState(canvasSettings);
	useEffect(() => setLiveCanvasSettings(canvasSettings), [canvasSettings]);
	const { src: previewBackgroundImage } = useAssetSource(
		actionContext?.appId,
		liveCanvasSettings.backgroundImage,
	);
	const backgroundClass = isBackgroundClass(liveCanvasSettings.backgroundColor)
		? liveCanvasSettings.backgroundColor
		: undefined;
	const loadEventExecutedRef = useRef<string | null>(null);
	// Keep a ref to components to avoid stale closure in handleA2UIMessage
	const componentsRef = useRef(components);
	componentsRef.current = components;

	// Bridge edit-mode BuilderContext state into a real Surface (with an empty
	// data model present) so runtime messages apply and $.path bindings resolve
	// in the preview exactly like the runtime page.
	const builderSurface: Surface = useMemo(
		() => ({
			id: effectiveSurfaceId,
			rootComponentId: ROOT_ID,
			components: Object.fromEntries(components),
			dataModel: [],
		}),
		[effectiveSurfaceId, components],
	);

	// A running workflow's surface wins over the builder's own once it exists.
	const logicalSurface = previewSurface ?? builderSurface;

	// Don't pass canvasSettings to A2UIRenderer — BuilderPreview handles
	// CSS injection and canvas styling at the outer level to avoid double
	// scoping and inline-style conflicts.
	const surface: Surface = useMemo(
		() => ({ ...logicalSurface, canvasSettings: undefined }),
		[logicalSurface],
	);

	const handleMessage = useCallback((message: A2UIClientMessage) => {
		console.log("Preview action:", message);
	}, []);

	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			// Canvas styling is handled by the outer div via liveCanvasSettings,
			// so keep it out of the surface reducer.
			if (message.type === "setCanvasSettings") {
				if (message.surfaceId !== effectiveSurfaceId) return;
				setLiveCanvasSettings((prev) => {
					const filtered = Object.fromEntries(
						Object.entries(message.canvasSettings).filter(([, v]) => v != null),
					);
					return { ...prev, ...filtered };
				});
				return;
			}
			setPreviewSurface((prev) => {
				const base = prev ?? {
					id: effectiveSurfaceId,
					rootComponentId: ROOT_ID,
					components: Object.fromEntries(componentsRef.current),
					dataModel: [],
				};
				const nextSurface = applyA2UIMessage(base, message);
				// A no-op message (unknown component / wrong surface) must not flip
				// previewSurface from null to a frozen snapshot; keep tracking live
				// builder edits until a message actually changes the surface.
				return prev === null && nextSurface === base ? null : nextSurface;
			});
		},
		[effectiveSurfaceId],
	);

	// Reads componentsRef so live builder edits never re-trigger the lifecycle effects.
	// The preview always refreshes the demand: the flow graph changes without a signal.
	const collectPreviewElements = useCallback(
		(appId: string, boardId: string) =>
			collectRunElements({
				backend,
				appId,
				boardId,
				surfaceId: effectiveSurfaceId,
				components: Object.fromEntries(componentsRef.current),
				storedValues: {},
				refresh: true,
			}),
		[backend, effectiveSurfaceId],
	);

	const elementSource = useCallback(
		(): ElementSource => ({
			surfaceId: effectiveSurfaceId,
			components: Object.fromEntries(componentsRef.current),
			storedValues: {},
		}),
		[effectiveSurfaceId],
	);

	// Execute onLoad event when entering preview mode
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			const { appId, boardId, pageId, onLoadEventId } = actionContext || {};

			if (!onLoadEventId || !appId || !boardId) return;

			// Prevent duplicate execution
			const executionKey = `preview:${pageId}:${onLoadEventId}`;
			if (loadEventExecutedRef.current === executionKey) return;
			loadEventExecutedRef.current = executionKey;

			try {
				const builderElements = await collectPreviewElements(appId, boardId);

				const payload = {
					id: onLoadEventId,
					payload: {
						_elements: builderElements,
						_elements_mode: "demand",
						_route: "/preview",
						_query_params: {},
						_page_id: pageId,
						_event_type: "onLoad",
						_preview_mode: true,
					},
				};

				// Use execution service if available (checks runtime variables)
				const execFn =
					executionService?.executeBoard ?? backend.boardState.executeBoard;
				await execFn(appId, boardId, payload, false, undefined, (events) => {
					for (const evt of events) {
						if (evt.event_type === "a2ui") {
							if (handleWidgetQueryMessage(evt.payload)) {
								continue;
							}
							if (handleElementsRequestMessage(evt.payload, elementSource)) {
								continue;
							}
							handleA2UIMessage(evt.payload as A2UIServerMessage);
						}
					}
				});
			} catch (e) {
				console.error("[BuilderPreview] Failed to execute onLoad event:", e);
			}
		};

		executeOnLoadEvent();
	}, [
		actionContext,
		backend.boardState,
		executionService,
		handleA2UIMessage,
		collectPreviewElements,
		elementSource,
	]);

	// Execute onInterval event at configured time intervals (preview mode)
	useEffect(() => {
		const { appId, boardId, pageId, onIntervalEventId, onIntervalSeconds } =
			actionContext || {};

		if (
			!onIntervalEventId ||
			!appId ||
			!boardId ||
			!onIntervalSeconds ||
			onIntervalSeconds <= 0
		)
			return;

		const intervalMs = onIntervalSeconds * 1000;

		const intervalId = setInterval(async () => {
			try {
				const builderElements = await collectPreviewElements(appId, boardId);

				const payload = {
					id: onIntervalEventId,
					payload: {
						_elements: builderElements,
						_elements_mode: "demand",
						_route: "/preview",
						_query_params: {},
						_page_id: pageId,
						_event_type: "onInterval",
						_preview_mode: true,
						_interval_seconds: onIntervalSeconds,
					},
				};

				// Use execution service if available (checks runtime variables)
				const execFn =
					executionService?.executeBoard ?? backend.boardState.executeBoard;
				await execFn(appId, boardId, payload, false, undefined, (events) => {
					for (const evt of events) {
						if (evt.event_type === "a2ui") {
							if (handleWidgetQueryMessage(evt.payload)) {
								continue;
							}
							if (handleElementsRequestMessage(evt.payload, elementSource)) {
								continue;
							}
							handleA2UIMessage(evt.payload as A2UIServerMessage);
						}
					}
				});
			} catch (e) {
				console.error(
					"[BuilderPreview] Failed to execute onInterval event:",
					e,
				);
			}
		}, intervalMs);

		return () => clearInterval(intervalId);
	}, [
		actionContext,
		backend.boardState,
		executionService,
		handleA2UIMessage,
		collectPreviewElements,
		elementSource,
	]);

	return (
		<div
			data-canvas-id={previewCanvasId}
			className={cn("h-full w-full overflow-auto", backgroundClass)}
			style={{
				backgroundColor: backgroundClass
					? undefined
					: liveCanvasSettings.backgroundColor,
				backgroundImage: previewBackgroundImage
					? `url(${previewBackgroundImage})`
					: undefined,
				padding: liveCanvasSettings.padding,
			}}
		>
			{/* Custom CSS injection (scoped and sanitized) */}
			<ScopedCustomCss
				css={liveCanvasSettings.customCss}
				scopeSelector={`[data-canvas-id="${previewCanvasId}"]`}
			/>
			<A2UIRenderer
				surface={surface}
				widgetRefs={Object.fromEntries(widgetRefs)}
				onMessage={handleMessage}
				onA2UIMessage={handleA2UIMessage}
				className="min-h-full w-full"
				appId={actionContext?.appId}
				boardId={actionContext?.boardId}
				eventId={actionContext?.eventId}
				isPreviewMode={true}
			/>
		</div>
	);
}
