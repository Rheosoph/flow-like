import { useDroppable } from "@dnd-kit/core";
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
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib";
import { createSanitizedStyleProps, safeScopedCss } from "../../lib/css-utils";
import {
	presignCanvasSettings,
	presignPageAssets,
} from "../../lib/presign-assets";
import { useBackend } from "../../state/backend-state";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type { IWidget } from "../../state/backend-state/widget-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import type {
	A2UIClientMessage,
	A2UIComponent,
	A2UIServerMessage,
	Children,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
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
		boardVersion?: [number, number, number];
		pages?: { id: string; name: string; boardId?: string }[];
		workflowEvents?: { nodeId: string; name: string }[];
		widgetActions?: { id: string; label: string; description?: string }[];
	};
	/** Current page ID for the page switcher */
	currentPageId?: string;
	/** Called when user switches to a different page */
	onPageChange?: (pageId: string) => void;
}

export function WidgetBuilder({
	className,
	initialComponents = [],
	initialWidgetRefs,
	surfaceId = "builder-surface",
	onSave,
	onExport,
	onChange,
	initialCanvasSettings,
	onCanvasSettingsChange,
	actionContext,
	currentPageId,
	onPageChange,
}: WidgetBuilderProps) {
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
			<WidgetBuilderWithDnd
				className={className}
				surfaceId={surfaceId}
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
			/>
		</BuilderProvider>
	);
}

interface WidgetBuilderContentProps {
	className?: string;
	surfaceId: string;
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
}: WidgetBuilderContentProps) {
	const {
		components,
		selection,
		addComponent,
		updateComponent,
		getComponent,
		widgetRefs,
		actionContext,
		setCanvasSettings,
	} = useBuilder();
	const { activeId } = useBuilderDnd();
	const isDragging = activeId !== null;

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
			canvasSettings?: {
				backgroundColor?: string;
				padding?: string;
				customCss?: string;
			},
		) => {
			if (pendingComponents.length === 0) return;

			// Apply canvas settings if provided
			if (canvasSettings) {
				setCanvasSettings(canvasSettings);
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
			setCanvasSettings,
		],
	);

	const handleDismissComponents = useCallback(() => {
		setPendingComponents([]);
	}, [setPendingComponents]);

	const currentComponents = Array.from(components.values());
	const selectedIds = selection.componentIds;

	return (
		<>
			<div
				className={cn(
					"flex flex-col h-full bg-muted/20 overflow-hidden",
					className,
					isDragging && "select-none",
				)}
			>
				{/* Toolbar */}
				<div className="flex items-center gap-1 h-10 px-2 border-b bg-background shrink-0">
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
					<Button
						variant={copilotOpen ? "secondary" : "ghost"}
						size="sm"
						className="h-7 px-2 gap-1.5"
						onClick={() => setCopilotOpen(!copilotOpen)}
					>
						<SparklesIcon className="h-4 w-4" />
						<span className="text-xs">FlowPilot</span>
					</Button>
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
									className="h-full flex flex-col min-h-0"
								>
									<TabsList className="w-full justify-start rounded-none border-b bg-transparent px-2 shrink-0">
										<TabsTrigger value="palette" className="gap-1.5">
											<Palette className="h-4 w-4" />
											<span className="hidden sm:inline">Components</span>
										</TabsTrigger>
										<TabsTrigger value="hierarchy" className="gap-1.5">
											<Layers className="h-4 w-4" />
											<span className="hidden sm:inline">Hierarchy</span>
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
								{copilotOpen ? (
									<A2UICopilot
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

				{/* Mobile FlowPilot Sheet */}
				<Sheet open={copilotOpen} onOpenChange={setCopilotOpen}>
					<SheetContent side="right" className="w-full sm:max-w-md p-0">
						<A2UICopilot
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
	return (
		<div className="flex items-center justify-between px-4 py-2 bg-primary/5 border-b border-primary/20 shrink-0">
			<div className="flex items-center gap-2">
				<SparklesIcon className="h-4 w-4 text-primary" />
				<span className="text-sm font-medium">
					{components.length} component{components.length !== 1 ? "s" : ""}{" "}
					ready to apply
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
					Dismiss
				</Button>
				<Button size="sm" onClick={onApply} className="h-7 px-3">
					Apply Changes
				</Button>
			</div>
		</div>
	);
}

// Visual Canvas - shows live preview with drop overlays
function VisualCanvas({ surfaceId }: { surfaceId: string }) {
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
	const [presignedComponents, setPresignedComponents] = useState<Map<
		string,
		SurfaceComponent
	> | null>(null);
	const [presignedCanvasSettings, setPresignedCanvasSettings] =
		useState(canvasSettings);

	// Presign assets in components for preview rendering
	useEffect(() => {
		const presignAssets = async () => {
			const appId = actionContext?.appId;
			if (!appId) {
				setPresignedComponents(null);
				return;
			}

			const componentsArray = Array.from(components.entries()).map(
				([id, comp]) => ({ ...comp, id }),
			);

			try {
				const presigned = await presignPageAssets(
					appId,
					componentsArray,
					backend.storageState,
				);
				const presignedMap = new Map<string, SurfaceComponent>();
				for (const comp of presigned) {
					presignedMap.set(comp.id, comp);
				}
				setPresignedComponents(presignedMap);
			} catch (err) {
				console.warn("[VisualCanvas] Failed to presign assets:", err);
				setPresignedComponents(null);
			}
		};

		presignAssets();
	}, [components, actionContext?.appId, backend.storageState]);

	// Presign canvas background image
	useEffect(() => {
		const presignCanvas = async () => {
			const appId = actionContext?.appId;
			if (!appId) {
				setPresignedCanvasSettings(canvasSettings);
				return;
			}

			try {
				const presigned = await presignCanvasSettings(
					appId,
					canvasSettings,
					backend.storageState,
				);
				setPresignedCanvasSettings(presigned);
			} catch (err) {
				console.warn("[VisualCanvas] Failed to presign canvas settings:", err);
				setPresignedCanvasSettings(canvasSettings);
			}
		};

		presignCanvas();
	}, [canvasSettings, actionContext?.appId, backend.storageState]);

	// Build the surface for rendering - use presigned components if available
	// Memoize to prevent unnecessary re-renders when drag state changes
	const surface: Surface = useMemo(
		() => ({
			id: surfaceId,
			rootComponentId: ROOT_ID,
			components: Object.fromEntries(presignedComponents ?? components),
			canvasSettings: presignedCanvasSettings.customCss
				? { customCss: presignedCanvasSettings.customCss }
				: undefined,
		}),
		[surfaceId, presignedComponents, components, presignedCanvasSettings.customCss],
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
				"h-full flex flex-col bg-muted/30 overflow-hidden",
				isDragging && "select-none",
			)}
			style={{ userSelect: isDragging ? "none" : undefined }}
		>
			{/* Custom CSS injection (scoped and sanitized) */}
			{presignedCanvasSettings.customCss && (
				<style
					{...createSanitizedStyleProps(
						safeScopedCss(
							presignedCanvasSettings.customCss,
							`[data-canvas-id="${canvasId}"]`,
						),
					)}
				/>
			)}

			{/* Canvas header with breadcrumb */}
			<div className="flex items-center gap-2 px-3 py-2 border-b bg-background text-xs text-muted-foreground shrink-0">
				<span>Canvas</span>
				{selection.componentIds.length > 0 &&
					selection.componentIds[0] !== ROOT_ID && (
						<>
							<ChevronRight className="h-3 w-3" />
							<span className="text-foreground font-medium">
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
					className="h-full min-h-full rounded-lg border shadow-sm relative"
					style={{
						backgroundColor: presignedCanvasSettings.backgroundColor,
						backgroundImage: presignedCanvasSettings.backgroundImage
							? `url(${presignedCanvasSettings.backgroundImage})`
							: undefined,
						padding: presignedCanvasSettings.padding,
					}}
					data-canvas-background="true"
				>
					{/* Interactive BuilderRenderer - wraps each component */}
					<BuilderRenderer surface={surface} className="w-full h-full" />

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
									Drop components here to start building
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
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const { components, canvasSettings, actionContext, widgetRefs } =
		useBuilder();
	const previewCanvasId = useId();
	const [previewComponents, setPreviewComponents] = useState<Map<
		string,
		SurfaceComponent
	> | null>(null);
	const [presignedComponents, setPresignedComponents] = useState<Map<
		string,
		SurfaceComponent
	> | null>(null);
	const [presignedCanvasSettings, setPresignedCanvasSettings] =
		useState(canvasSettings);
	const loadEventExecutedRef = useRef<string | null>(null);
	// Keep a ref to components to avoid stale closure in handleA2UIMessage
	const componentsRef = useRef(components);
	componentsRef.current = components;

	// Presign assets in components when they change
	useEffect(() => {
		const presignAssets = async () => {
			const appId = actionContext?.appId;
			if (!appId) {
				setPresignedComponents(null);
				return;
			}

			const componentsArray = Array.from(
				(previewComponents ?? components).entries(),
			).map(([id, comp]) => ({ ...comp, id }));

			try {
				const presigned = await presignPageAssets(
					appId,
					componentsArray,
					backend.storageState,
				);
				const presignedMap = new Map<string, SurfaceComponent>();
				for (const comp of presigned) {
					presignedMap.set(comp.id, comp);
				}
				setPresignedComponents(presignedMap);
			} catch (err) {
				console.warn("[BuilderPreview] Failed to presign assets:", err);
				setPresignedComponents(null);
			}
		};

		presignAssets();
	}, [
		components,
		previewComponents,
		actionContext?.appId,
		backend.storageState,
	]);

	// Presign canvas background image
	useEffect(() => {
		const presignCanvas = async () => {
			const appId = actionContext?.appId;
			if (!appId) {
				setPresignedCanvasSettings(canvasSettings);
				return;
			}

			try {
				const presigned = await presignCanvasSettings(
					appId,
					canvasSettings,
					backend.storageState,
				);
				setPresignedCanvasSettings(presigned);
			} catch (err) {
				console.warn(
					"[BuilderPreview] Failed to presign canvas settings:",
					err,
				);
				setPresignedCanvasSettings(canvasSettings);
			}
		};

		presignCanvas();
	}, [canvasSettings, actionContext?.appId, backend.storageState]);

	// Use presigned components if available, otherwise fall back to preview or builder components
	const activeComponents =
		presignedComponents ?? previewComponents ?? components;

	const surface: Surface = useMemo(
		() => ({
			id: surfaceId,
			rootComponentId: ROOT_ID,
			components: Object.fromEntries(activeComponents),
			canvasSettings: presignedCanvasSettings.customCss
				? { customCss: presignedCanvasSettings.customCss }
				: undefined,
		}),
		[surfaceId, activeComponents, presignedCanvasSettings.customCss],
	);

	const handleMessage = useCallback((message: A2UIClientMessage) => {
		console.log("Preview action:", message);
	}, []);

	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			if (message.type === "setCanvasSettings") {
				if (message.surfaceId !== surfaceId) return;

				setPresignedCanvasSettings((prev) => ({
					...prev,
					...message.canvasSettings,
				}));
				return;
			}

			if (message.type !== "upsertElement") return;

			const { element_id: elementId, value } = message;
			if (!elementId) return;

			const [msgSurfaceId, componentId] = elementId.includes("/")
				? elementId.split("/", 2)
				: [surfaceId, elementId];

			if (msgSurfaceId !== surfaceId) return;

			setPreviewComponents((prev) => {
				// Use ref to always get latest components, avoiding stale closure
				const current = prev ?? new Map(componentsRef.current);
				const component = current.get(componentId);
				if (!component) {
					console.warn(
						"[BuilderPreview] Component not found:",
						componentId,
						"Available:",
						Array.from(current.keys()),
					);
					return prev;
				}

				const updateValue = value as Record<string, unknown>;
				const updateType = updateValue?.type as string;
				let updatedComponent: SurfaceComponent = { ...component };

				switch (updateType) {
					case "setText": {
						const text = updateValue.text as string;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								content: text,
								text: text,
								label: text,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setValue": {
						const val = updateValue.value as string;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								value: val,
								defaultValue: val,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setStyle": {
						const newStyle = updateValue.style as Partial<
							SurfaceComponent["style"]
						>;
						updatedComponent = {
							...component,
							style: {
								...component.style,
								...newStyle,
							},
						};
						break;
					}
					case "setVisibility": {
						const visible = updateValue.visible as boolean;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								hidden: !visible,
							} as unknown as SurfaceComponent["component"],
							style: {
								...component.style,
								opacity: visible ? undefined : 0,
							},
						};
						break;
					}
					case "setDisabled": {
						const disabled = updateValue.disabled as boolean;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								disabled,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setLoading": {
						const loading = updateValue.loading as boolean;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								loading,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setPlaceholder": {
						const placeholder = updateValue.placeholder as string;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								placeholder,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setChecked": {
						const checked = updateValue.checked as boolean;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								checked,
								value: checked,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setSpeakerName": {
						const name = updateValue.name as string;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								speakerName: name,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setSpeakerPortrait": {
						const portraitId = updateValue.portraitId as string;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								speakerPortraitId: portraitId,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setTypewriter": {
						const enabled = updateValue.enabled as boolean;
						const speed = updateValue.speed as number | undefined;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								typewriter: enabled,
								...(speed !== undefined && { typewriterSpeed: speed }),
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					case "setGeoMapViewport": {
						const viewport = updateValue.viewport as
							| { literalJson?: string }
							| undefined;
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								viewport,
							} as unknown as SurfaceComponent["component"],
						};
						break;
					}
					default: {
						const componentData = component.component as unknown as Record<
							string,
							unknown
						>;
						updatedComponent = {
							...component,
							component: {
								...componentData,
								...updateValue,
							} as unknown as SurfaceComponent["component"],
						};
					}
				}

				const newMap = new Map(current);
				newMap.set(componentId, updatedComponent);
				return newMap;
			});
		},
		[surfaceId],
	); // Removed components - using componentsRef instead

	// Convert components map to elements object for the workflow payload
	// Uses componentsRef to avoid dependency on components changing (which would cause infinite loops)
	const getElementsFromComponents = useCallback(() => {
		const elements: Record<string, unknown> = {};
		const currentComponents = componentsRef.current;
		for (const [componentId, surfaceComponent] of currentComponents.entries()) {
			const elementId = `${surfaceId}/${componentId}`;
			elements[elementId] = {
				...surfaceComponent,
				__element_id: elementId,
			};
		}
		return elements;
	}, [surfaceId]); // Only depends on surfaceId which doesn't change during preview

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
				// Get elements from current components (for GetElement to work)
				const builderElements = getElementsFromComponents();

				const payload = {
					id: onLoadEventId,
					payload: {
						_elements: builderElements,
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
		getElementsFromComponents,
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
				const builderElements = getElementsFromComponents();

				const payload = {
					id: onIntervalEventId,
					payload: {
						_elements: builderElements,
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
		getElementsFromComponents,
	]);

	return (
		<div
			data-canvas-id={previewCanvasId}
			className="h-full w-full overflow-auto"
			style={{
				backgroundColor: presignedCanvasSettings.backgroundColor,
				backgroundImage: presignedCanvasSettings.backgroundImage
					? `url(${presignedCanvasSettings.backgroundImage})`
					: undefined,
				padding: presignedCanvasSettings.padding,
			}}
		>
			{/* Custom CSS injection (scoped and sanitized) */}
			{presignedCanvasSettings.customCss && (
				<style
					{...createSanitizedStyleProps(
						safeScopedCss(
							presignedCanvasSettings.customCss,
							`[data-canvas-id="${previewCanvasId}"]`,
						),
					)}
				/>
			)}
			<A2UIRenderer
				surface={surface}
				widgetRefs={Object.fromEntries(widgetRefs)}
				onMessage={handleMessage}
				onA2UIMessage={handleA2UIMessage}
				className="min-h-full w-full"
				appId={actionContext?.appId}
				boardId={actionContext?.boardId}
				isPreviewMode={true}
			/>
		</div>
	);
}
