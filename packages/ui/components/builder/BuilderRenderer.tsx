"use client";

import { useDraggable, useDroppable } from "@dnd-kit/core";
import {
	ClipboardPaste,
	Copy,
	GripVertical,
	Scissors,
	Sparkles,
	Trash2,
} from "lucide-react";
import {
	type ReactNode,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { ActionProvider } from "../a2ui/ActionHandler";
import {
	type ComponentProps,
	getComponentRenderer,
} from "../a2ui/ComponentRegistry";
import { DataProvider } from "../a2ui/DataContext";
import { WidgetRefsProvider } from "../a2ui/WidgetRefsContext";
import type {
	A2UIClientMessage,
	A2UIComponent,
	Children,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { useBuilder } from "./BuilderContext";
import {
	COMPONENT_MOVE_TYPE,
	type ComponentMoveData,
	type DropData,
	useBuilderDnd,
} from "./BuilderDndContext";
import { CONTAINER_TYPES, ROOT_ID } from "./WidgetBuilder";

// ============================================================================
// Types
// ============================================================================

interface BuilderRendererProps {
	surface: Surface;
	className?: string;
}

// ============================================================================
// Drop Indicator - Shows where items can be dropped
// ============================================================================

interface DropIndicatorProps {
	parentId: string;
	index: number;
	orientation: "horizontal" | "vertical";
	isActive?: boolean;
}

const DropIndicator = memo(function DropIndicator({
	parentId,
	index,
	orientation,
	isActive = false,
}: DropIndicatorProps) {
	const dropId = `drop-${parentId}-${index}`;

	const { setNodeRef, isOver } = useDroppable({
		id: dropId,
		data: {
			type: "drop-zone",
			parentId,
			index,
		} satisfies DropData,
	});

	const showIndicator = isOver || isActive;

	return (
		<div
			ref={setNodeRef}
			data-drop-indicator={`${parentId}-${index}`}
			className={cn(
				"shrink-0 transition-all duration-100",
				orientation === "horizontal" ? "w-1 self-stretch" : "h-1 w-full",
				showIndicator && "z-50",
			)}
		>
			<div
				className={cn(
					"transition-all duration-100 rounded-full",
					orientation === "horizontal"
						? "h-full w-0.5 mx-auto"
						: "w-full h-0.5 my-auto",
					showIndicator ? "bg-primary scale-y-100" : "bg-transparent scale-y-0",
					showIndicator && (orientation === "horizontal" ? "w-1" : "h-1"),
				)}
			/>
		</div>
	);
});

// ============================================================================
// Cursor-tracking Container Drop Zone
// Calculates nearest insertion point based on cursor position
// ============================================================================

interface ContainerDropZoneProps {
	parentId: string;
	childIds: string[];
	orientation: "horizontal" | "vertical";
	containerRef: React.RefObject<HTMLDivElement>;
}

function ContainerDropZone({
	parentId,
	childIds,
	orientation,
	containerRef,
}: ContainerDropZoneProps) {
	const [nearestIndex, setNearestIndex] = useState<number>(0);
	const [indicatorPosition, setIndicatorPosition] = useState<number>(0);
	const { activeId } = useBuilderDnd();
	const isDragging = activeId !== null;

	// Track mouse position to calculate nearest drop index
	useEffect(() => {
		if (!isDragging || !containerRef.current) return;

		const handleMouseMove = (e: MouseEvent) => {
			const container = containerRef.current;
			if (!container) return;

			// Get only direct children that are builder components
			const allChildElements = container.querySelectorAll(
				"[data-builder-component]",
			);
			// Filter to only direct children
			const childElements = Array.from(allChildElements).filter(
				(el) =>
					el.parentElement?.closest("[data-builder-component]") === container,
			);

			if (childElements.length === 0) {
				setNearestIndex(0);
				setIndicatorPosition(0);
				return;
			}

			const containerRect = container.getBoundingClientRect();
			const mousePos = orientation === "horizontal" ? e.clientX : e.clientY;
			let bestIndex = childIds.length;
			let bestPosition =
				orientation === "horizontal"
					? containerRect.width
					: containerRect.height;

			// Check each child to find the best insertion point
			childElements.forEach((child, i) => {
				const rect = child.getBoundingClientRect();
				const childStart = orientation === "horizontal" ? rect.left : rect.top;
				const childEnd =
					orientation === "horizontal" ? rect.right : rect.bottom;
				const childMid = (childStart + childEnd) / 2;
				const containerStart =
					orientation === "horizontal" ? containerRect.left : containerRect.top;

				if (mousePos < childMid) {
					// Insert before this child
					if (
						i < bestIndex ||
						(i === bestIndex && childStart - containerStart < bestPosition)
					) {
						bestIndex = i;
						bestPosition = childStart - containerStart;
					}
				} else if (i === childElements.length - 1) {
					// After last child
					bestIndex = i + 1;
					bestPosition = childEnd - containerStart;
				}
			});

			setNearestIndex(bestIndex);
			setIndicatorPosition(bestPosition);
		};

		window.addEventListener("mousemove", handleMouseMove);
		// Initial calculation
		handleMouseMove(new MouseEvent("mousemove", { clientX: 0, clientY: 0 }));

		return () => window.removeEventListener("mousemove", handleMouseMove);
	}, [isDragging, orientation, childIds.length, containerRef]);

	// Register as droppable with the calculated index
	const { setNodeRef, isOver } = useDroppable({
		id: `container-zone-${parentId}`,
		data: {
			type: "drop-zone",
			parentId,
			index: nearestIndex,
		} satisfies DropData,
	});

	if (!isDragging) return null;

	return (
		<div ref={setNodeRef} className="absolute inset-0 z-30 pointer-events-auto">
			{/* Visual drop indicator line */}
			{isOver && (
				<div
					className={cn(
						"absolute bg-primary transition-all duration-75 rounded-full",
						orientation === "horizontal"
							? "w-1 top-1 bottom-1"
							: "h-1 left-1 right-1",
					)}
					style={{
						[orientation === "horizontal" ? "left" : "top"]:
							`${indicatorPosition}px`,
						transform:
							orientation === "horizontal"
								? "translateX(-50%)"
								: "translateY(-50%)",
					}}
				/>
			)}
		</div>
	);
}

// ============================================================================
// Empty Container Drop Zone
// ============================================================================

interface EmptyDropZoneProps {
	parentId: string;
}

const EmptyDropZone = memo(function EmptyDropZone({
	parentId,
}: EmptyDropZoneProps) {
	const dropId = `empty-${parentId}`;

	const { setNodeRef, isOver } = useDroppable({
		id: dropId,
		data: {
			type: "drop-zone",
			parentId,
			index: 0,
		} satisfies DropData,
	});

	return (
		<div
			ref={setNodeRef}
			className={cn(
				"flex-1 min-h-16 min-w-16 flex items-center justify-center",
				"border-2 border-dashed rounded-lg transition-all duration-200",
				isOver
					? "border-primary bg-primary/15 scale-[1.01] shadow-inner"
					: "border-muted-foreground/20 bg-muted/30",
			)}
		>
			<span
				className={cn(
					"text-xs select-none transition-colors",
					isOver ? "text-primary font-medium" : "text-muted-foreground/50",
				)}
			>
				{isOver ? "Release to drop" : "Drop here"}
			</span>
		</div>
	);
});

// ============================================================================
// Selection Toolbar - Appears above selected elements
// ============================================================================

interface SelectionToolbarProps {
	componentType: string;
	isRoot: boolean;
	onDelete: () => void;
	onCopy: () => void;
	onCut: () => void;
	onPaste: () => void;
	onOptimize?: () => void;
	dragHandleRef: (node: HTMLElement | null) => void;
	dragAttributes: React.HTMLAttributes<HTMLButtonElement>;
	dragListeners: React.DOMAttributes<HTMLButtonElement> | undefined;
}

const SelectionToolbar = memo(function SelectionToolbar({
	componentType,
	isRoot,
	onDelete,
	onCopy,
	onCut,
	onPaste,
	onOptimize,
	dragHandleRef,
	dragAttributes,
	dragListeners,
}: SelectionToolbarProps) {
	return (
		<div
			className="absolute -top-9 left-0 z-100 flex items-center gap-1 px-1.5 py-1 bg-primary text-primary-foreground rounded-md text-xs shadow-lg pointer-events-auto border border-primary-foreground/10"
			onClick={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			{/* Drag Handle - only for non-root */}
			{!isRoot && (
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							ref={dragHandleRef}
							{...dragAttributes}
							{...dragListeners}
							className="p-1.5 hover:bg-white/20 rounded-md cursor-grab active:cursor-grabbing touch-none transition-colors"
						>
							<GripVertical className="h-3.5 w-3.5" />
						</button>
					</TooltipTrigger>
					<TooltipContent side="top">Drag to move</TooltipContent>
				</Tooltip>
			)}

			{/* Component Type Label */}
			<span className="px-2 py-0.5 font-medium capitalize select-none bg-white/10 rounded">
				{componentType}
			</span>

			<div className="w-px h-5 bg-white/20 mx-1" />

			{/* Copy */}
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						onClick={onCopy}
						className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
					>
						<Copy className="h-3.5 w-3.5" />
					</button>
				</TooltipTrigger>
				<TooltipContent side="top">Copy (⌘C)</TooltipContent>
			</Tooltip>

			{/* Cut - only for non-root */}
			{!isRoot && (
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={onCut}
							className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
						>
							<Scissors className="h-3.5 w-3.5" />
						</button>
					</TooltipTrigger>
					<TooltipContent side="top">Cut (⌘X)</TooltipContent>
				</Tooltip>
			)}

			{/* Paste */}
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						onClick={onPaste}
						className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
					>
						<ClipboardPaste className="h-3.5 w-3.5" />
					</button>
				</TooltipTrigger>
				<TooltipContent side="top">Paste (⌘V)</TooltipContent>
			</Tooltip>

			{/* Optimize with FlowPilot */}
			{onOptimize && (
				<>
					<div className="w-px h-5 bg-white/20 mx-1" />
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={onOptimize}
								className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
							>
								<Sparkles className="h-3.5 w-3.5" />
							</button>
						</TooltipTrigger>
						<TooltipContent side="top">Optimize with FlowPilot</TooltipContent>
					</Tooltip>
				</>
			)}

			{/* Delete - only for non-root */}
			{!isRoot && (
				<>
					<div className="w-px h-5 bg-white/20 mx-1" />
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={onDelete}
								className="p-1.5 hover:bg-red-500/30 rounded-md transition-colors text-red-200 hover:text-red-100"
							>
								<Trash2 className="h-3.5 w-3.5" />
							</button>
						</TooltipTrigger>
						<TooltipContent side="top">Delete (⌫)</TooltipContent>
					</Tooltip>
				</>
			)}
		</div>
	);
});

// ============================================================================
// Selection Overlay - Shows selection state without affecting layout
// ============================================================================

interface SelectionOverlayProps {
	componentId: string;
	componentType: string;
	isSelected: boolean;
	isRoot: boolean;
	isHovered: boolean;
	isDraggingThis: boolean;
	onDelete: () => void;
	onCopy: () => void;
	onCut: () => void;
	onPaste: () => void;
	dragHandleRef: (node: HTMLElement | null) => void;
	dragAttributes: React.HTMLAttributes<HTMLButtonElement>;
	dragListeners: React.DOMAttributes<HTMLButtonElement> | undefined;
}

function SelectionOverlay({
	componentType,
	isSelected,
	isRoot,
	isHovered,
	isDraggingThis,
	onDelete,
	onCopy,
	onCut,
	onPaste,
	dragHandleRef,
	dragAttributes,
	dragListeners,
}: SelectionOverlayProps) {
	if (isRoot && !isSelected) return null;
	if (!isSelected && !isHovered) return null;

	return (
		<>
			{/* Selection/hover outline */}
			<div
				className={cn(
					"absolute inset-0 pointer-events-none rounded transition-all duration-150 z-40",
					isDraggingThis && "opacity-30",
					isSelected && !isRoot && "border-2 border-dotted border-foreground",
					!isSelected &&
						isHovered &&
						!isRoot &&
						"border border-dotted border-foreground/40",
				)}
			/>

			{/* Toolbar for selected items */}
			{isSelected && (
				<SelectionToolbar
					componentType={componentType}
					isRoot={isRoot}
					onDelete={onDelete}
					onCopy={onCopy}
					onCut={onCut}
					onPaste={onPaste}
					dragHandleRef={dragHandleRef}
					dragAttributes={dragAttributes}
					dragListeners={dragListeners}
				/>
			)}
		</>
	);
}

// ============================================================================
// Builder Component - Wraps each A2UI component with builder functionality
// ============================================================================

interface BuilderComponentProps {
	componentId: string;
	surfaceComponent: SurfaceComponent;
	surfaceId: string;
	allComponents: Map<string, SurfaceComponent>;
	renderChild: (childId: string) => ReactNode;
}

function BuilderComponent({
	componentId,
	surfaceComponent,
	surfaceId,
	allComponents,
	renderChild: parentRenderChild,
}: BuilderComponentProps) {
	const containerRef = useRef<HTMLDivElement>(null);
	const [isHovered, setIsHovered] = useState(false);
	const { component, style } = surfaceComponent;

	const {
		selection,
		selectComponent,
		updateComponent,
		deleteComponents,
		copy,
		cut,
		paste,
		isComponentHidden,
		components: builderComponents,
	} = useBuilder();

	const { activeId } = useBuilderDnd();
	const isDragging = activeId !== null;

	// Component metadata
	const isContainer = component ? CONTAINER_TYPES.has(component.type) : false;
	const isRoot = componentId === ROOT_ID;
	const isSelected = selection.componentIds.includes(componentId);
	const isHidden = component ? isComponentHidden(componentId) : false;

	// Get children for containers
	const childrenData = component
		? ((component as unknown as Record<string, unknown>).children as
				| Children
				| undefined)
		: undefined;
	const childIds = useMemo(
		() =>
			childrenData && "explicitList" in childrenData
				? childrenData.explicitList
				: [],
		[childrenData],
	);

	// Determine layout orientation for drop indicators
	const isHorizontalLayout = component?.type === "row";

	// Find parent component
	const findParentId = useCallback((): string | null => {
		for (const [id, comp] of builderComponents) {
			if (!comp.component) continue;
			const compChildren = (
				comp.component as unknown as Record<string, unknown>
			).children as Children | undefined;
			const children =
				compChildren && "explicitList" in compChildren
					? compChildren.explicitList
					: undefined;
			if (children?.includes(componentId)) {
				return id;
			}
		}
		return null;
	}, [builderComponents, componentId]);

	// Draggable setup
	const {
		attributes: dragAttributes,
		listeners: dragListeners,
		setNodeRef: setDragRef,
		isDragging: isThisDragging,
	} = useDraggable({
		id: `move-${componentId}`,
		disabled: isRoot,
		data: {
			type: COMPONENT_MOVE_TYPE,
			componentId,
			currentParentId: findParentId(),
		} satisfies ComponentMoveData,
	});

	// Droppable setup for containers
	const { setNodeRef: setDropRef, isOver: isOverContainer } = useDroppable({
		id: `container-${componentId}`,
		disabled: !isContainer,
		data: {
			type: "container",
			parentId: componentId,
			isContainer: true,
		} satisfies DropData,
	});

	// Click handler - select component
	const handleClick = useCallback(
		(e: React.MouseEvent) => {
			e.stopPropagation();
			selectComponent(componentId, e.shiftKey || e.metaKey);
		},
		[componentId, selectComponent],
	);

	// Delete handler
	const handleDelete = useCallback(() => {
		if (isRoot) return;
		const parentId = findParentId();
		if (parentId) {
			const parent = builderComponents.get(parentId);
			if (parent?.component) {
				const parentChildren = (
					parent.component as unknown as Record<string, unknown>
				).children as Children | undefined;
				const children =
					parentChildren && "explicitList" in parentChildren
						? parentChildren.explicitList
						: [];
				updateComponent(parentId, {
					component: {
						...parent.component,
						children: {
							explicitList: children.filter((id: string) => id !== componentId),
						},
					} as A2UIComponent,
				});
			}
		}
		deleteComponents([componentId]);
	}, [
		componentId,
		builderComponents,
		updateComponent,
		deleteComponents,
		isRoot,
		findParentId,
	]);

	// Copy/Cut/Paste handlers
	const handleCopy = useCallback(() => copy(), [copy]);
	const handleCut = useCallback(() => cut(), [cut]);
	const handlePaste = useCallback(
		() => paste(componentId),
		[paste, componentId],
	);

	// Get renderer
	const Renderer = component ? getComponentRenderer(component.type) : null;

	// Early returns
	if (!component || !Renderer) return null;
	if (isHidden) return null;

	// Custom renderChild for containers - no inline drop indicators needed
	// The ContainerDropZone overlay handles cursor-tracking drop position
	const renderChildForContainer = (childId: string): ReactNode => {
		const childComp =
			allComponents.get(childId) ?? builderComponents.get(childId);
		if (!childComp) return null;

		return (
			<BuilderComponent
				key={childId}
				componentId={childId}
				surfaceComponent={childComp}
				surfaceId={surfaceId}
				allComponents={allComponents}
				renderChild={parentRenderChild}
			/>
		);
	};

	// Modified renderChild for containers
	const modifiedRenderChild = isContainer
		? renderChildForContainer
		: parentRenderChild;

	// Build component props
	const componentProps: ComponentProps = {
		component,
		componentId,
		surfaceId,
		style: style ?? component.style,
		onAction: () => {},
		renderChild: modifiedRenderChild,
	};

	// For the root component, we need it to take full height
	if (isRoot) {
		return (
			<div
				ref={(node) => {
					containerRef.current = node;
					setDropRef(node);
				}}
				onClick={handleClick}
				onMouseEnter={() => setIsHovered(true)}
				onMouseLeave={() => setIsHovered(false)}
				className={cn(
					"relative h-full w-full",
					isOverContainer && isDragging && "bg-primary/5",
				)}
				data-builder-component={componentId}
				data-component-type={component.type}
			>
				{/* Cursor-tracking drop zone for this container */}
				{isDragging && childIds.length > 0 && (
					<ContainerDropZone
						parentId={componentId}
						childIds={childIds}
						orientation={isHorizontalLayout ? "horizontal" : "vertical"}
						containerRef={containerRef as React.RefObject<HTMLDivElement>}
					/>
				)}

				<SelectionOverlay
					componentId={componentId}
					componentType={component.type}
					isSelected={isSelected}
					isRoot={isRoot}
					isHovered={isHovered}
					isDraggingThis={isThisDragging}
					onDelete={handleDelete}
					onCopy={handleCopy}
					onCut={handleCut}
					onPaste={handlePaste}
					dragHandleRef={setDragRef}
					dragAttributes={
						dragAttributes as React.HTMLAttributes<HTMLButtonElement>
					}
					dragListeners={
						dragListeners as React.DOMAttributes<HTMLButtonElement> | undefined
					}
				/>

				<Renderer {...componentProps} />

				{/* Empty container state */}
				{childIds.length === 0 && (
					<div className="absolute inset-0 flex items-center justify-center pointer-events-none">
						{isDragging ? (
							<div className="pointer-events-auto w-full h-full p-2">
								<EmptyDropZone parentId={componentId} />
							</div>
						) : (
							<span className="text-xs text-muted-foreground/50 select-none">
								Empty {component.type}
							</span>
						)}
					</div>
				)}
			</div>
		);
	}

	// For non-root components - wrapper inherits flex/grid sizing
	return (
		<div
			ref={(node) => {
				containerRef.current = node;
				if (isContainer) setDropRef(node);
			}}
			onClick={handleClick}
			onMouseEnter={() => setIsHovered(true)}
			onMouseLeave={() => setIsHovered(false)}
			className={cn(
				"relative min-w-0",
				// Flex items should grow/shrink properly
				"flex-1",
				isThisDragging && "opacity-30",
				isContainer &&
					isOverContainer &&
					isDragging &&
					"bg-primary/5 ring-2 ring-primary/30 ring-inset",
			)}
			data-builder-component={componentId}
			data-component-type={component.type}
		>
			{/* Cursor-tracking drop zone for containers */}
			{isContainer && isDragging && childIds.length > 0 && (
				<ContainerDropZone
					parentId={componentId}
					childIds={childIds}
					orientation={isHorizontalLayout ? "horizontal" : "vertical"}
					containerRef={containerRef as React.RefObject<HTMLDivElement>}
				/>
			)}

			<SelectionOverlay
				componentId={componentId}
				componentType={component.type}
				isSelected={isSelected}
				isRoot={isRoot}
				isHovered={isHovered}
				isDraggingThis={isThisDragging}
				onDelete={handleDelete}
				onCopy={handleCopy}
				onCut={handleCut}
				onPaste={handlePaste}
				dragHandleRef={setDragRef}
				dragAttributes={
					dragAttributes as React.HTMLAttributes<HTMLButtonElement>
				}
				dragListeners={
					dragListeners as React.DOMAttributes<HTMLButtonElement> | undefined
				}
			/>

			<Renderer {...componentProps} />

			{/* Empty container state */}
			{isContainer && childIds.length === 0 && (
				<div className="absolute inset-0 flex items-center justify-center pointer-events-none">
					{isDragging ? (
						<div className="pointer-events-auto w-full h-full p-2">
							<EmptyDropZone parentId={componentId} />
						</div>
					) : (
						<span className="text-xs text-muted-foreground/50 select-none">
							Empty {component.type}
						</span>
					)}
				</div>
			)}
		</div>
	);
}

// ============================================================================
// Main BuilderRenderer Component
// ============================================================================

export function BuilderRenderer({ surface, className }: BuilderRendererProps) {
	const { actionContext, widgetRefs } = useBuilder();

	// Build components map
	const allComponents = useMemo(() => {
		const map = new Map<string, SurfaceComponent>();
		for (const [id, comp] of Object.entries(surface.components ?? {})) {
			map.set(id, comp);
		}
		return map;
	}, [surface.components]);

	// Action handler (intercepted in builder)
	const handleAction = useCallback((message: A2UIClientMessage) => {
		console.log("[BuilderRenderer] Action intercepted:", message);
	}, []);

	// Render child function
	const renderChild = useCallback(
		(childId: string): ReactNode => {
			const comp = allComponents.get(childId);
			if (!comp) return null;
			return (
				<BuilderComponent
					key={childId}
					componentId={childId}
					surfaceComponent={comp}
					surfaceId={surface.id}
					allComponents={allComponents}
					renderChild={renderChild}
				/>
			);
		},
		[allComponents, surface.id],
	);

	// Get root component
	const rootComponent = surface.rootComponentId
		? allComponents.get(surface.rootComponentId)
		: null;

	if (!rootComponent) {
		return (
			<div
				className={cn(
					"flex items-center justify-center h-full text-muted-foreground",
					className,
				)}
			>
				No content to display
			</div>
		);
	}

	return (
		<DataProvider initialData={[]}>
			<WidgetRefsProvider widgetRefs={widgetRefs}>
				<ActionProvider
					onAction={handleAction}
					surfaceId={surface.id}
					appId={actionContext?.appId}
					components={Object.fromEntries(allComponents)}
				>
					<div className={cn("min-h-full w-full", className)}>
						<BuilderComponent
							componentId={surface.rootComponentId}
							surfaceComponent={rootComponent}
							surfaceId={surface.id}
							allComponents={allComponents}
							renderChild={renderChild}
						/>
					</div>
				</ActionProvider>
			</WidgetRefsProvider>
		</DataProvider>
	);
}

export default BuilderRenderer;
