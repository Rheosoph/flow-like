"use client";

import {
	type CollisionDetection,
	DndContext,
	type DragEndEvent,
	type DragOverEvent,
	type DragStartEvent,
	PointerSensor,
	pointerWithin,
	rectIntersection,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import { type WidgetContract, contractDefaults } from "@flow-like/widget-sdk";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useRef,
	useState,
} from "react";
import { useBackend } from "../../state/backend-state";
import type { IWidget } from "../../state/backend-state/widget-state";
import type { A2UIComponent, Children, SurfaceComponent } from "../a2ui/types";
import { useBuilder } from "./BuilderContext";

// Drag item types
export const COMPONENT_DND_TYPE = "a2ui-component";
export const COMPONENT_MOVE_TYPE = "a2ui-component-move";
export const WIDGET_DND_TYPE = "a2ui-widget";
export const PACKAGE_WIDGET_DND_TYPE = "a2ui-package-widget";

export interface ComponentDragData {
	type: typeof COMPONENT_DND_TYPE;
	componentType: string;
}

export interface ComponentMoveData {
	type: typeof COMPONENT_MOVE_TYPE;
	componentId: string;
	currentParentId: string | null;
}

export interface WidgetDragData {
	type: typeof WIDGET_DND_TYPE;
	appId: string;
	widgetId: string;
	components?: SurfaceComponent[];
	rootComponentId?: string;
}

/**
 * A widget shipped by a package added to the app (§6.1). Carries the full
 * contract so dropping needs no backend round-trip — the placed
 * `microWidgetInstance` is self-contained.
 */
export interface PackageWidgetDragData {
	type: typeof PACKAGE_WIDGET_DND_TYPE;
	packageId: string;
	widgetId: string;
	packageVersion: string;
	bundleHash?: string;
	name: string;
	contract: WidgetContract;
}

export type DragData =
	| ComponentDragData
	| ComponentMoveData
	| WidgetDragData
	| PackageWidgetDragData;

export interface DropData {
	type: "container" | "drop-zone";
	parentId: string;
	index?: number;
	isContainer?: boolean;
}

interface BuilderDndContextType {
	activeId: string | null;
	activeData: DragData | null;
	overId: string | null;
	overData: DropData | null;
}

const BuilderDndReactContext = createContext<BuilderDndContextType>({
	activeId: null,
	activeData: null,
	overId: null,
	overData: null,
});

export function useBuilderDnd() {
	return useContext(BuilderDndReactContext);
}

interface BuilderDndProviderProps {
	children: ReactNode;
	setIsDraggingGlobal: (dragging: boolean) => void;
}

// Custom collision detection that prefers the most deeply nested container
const customCollisionDetection: CollisionDetection = (args) => {
	// Get all pointer collisions (cursor is inside these elements)
	const pointerCollisions = pointerWithin(args);

	if (pointerCollisions.length === 0) {
		// Fallback to rect intersection if pointer isn't directly over anything
		return rectIntersection(args);
	}

	// Sort by specificity:
	// 1. Drop zones (between-element indicators) get highest priority
	// 2. Then containers sorted by depth (smaller rect = more nested = higher priority)
	const sorted = [...pointerCollisions].sort((a, b) => {
		const aData = a.data?.droppableContainer?.data?.current as
			| DropData
			| undefined;
		const bData = b.data?.droppableContainer?.data?.current as
			| DropData
			| undefined;

		const aIsDropZone = aData?.type === "drop-zone";
		const bIsDropZone = bData?.type === "drop-zone";

		// Drop zones (explicit insertion points) always win
		if (aIsDropZone && !bIsDropZone) return -1;
		if (!aIsDropZone && bIsDropZone) return 1;

		// For containers, prefer smaller ones (more deeply nested)
		// Smaller area = more specific/nested container
		const aRect = a.data?.droppableContainer?.rect;
		const bRect = b.data?.droppableContainer?.rect;

		if (aRect && bRect) {
			const aArea = (aRect.width ?? 0) * (aRect.height ?? 0);
			const bArea = (bRect.width ?? 0) * (bRect.height ?? 0);
			// Smaller area = higher priority (return negative)
			return aArea - bArea;
		}

		return 0;
	});

	return sorted.length > 0 ? [sorted[0]] : [];
};

// Import these from WidgetBuilder to avoid circular deps
import { createDefaultComponent, getDefaultStyle } from "./componentDefaults";

export function BuilderDndProvider({
	children,
	setIsDraggingGlobal,
}: BuilderDndProviderProps) {
	const [activeId, setActiveId] = useState<string | null>(null);
	const [activeData, setActiveData] = useState<DragData | null>(null);
	const [overId, setOverId] = useState<string | null>(null);
	const [overData, setOverData] = useState<DropData | null>(null);
	const lastDropZoneRef = useRef<DropData | null>(null);

	const backend = useBackend();
	const { components, addComponent, updateComponent, addWidgetRef } =
		useBuilder();

	const pointerSensor = useSensor(PointerSensor, {
		activationConstraint: {
			distance: 8,
		},
	});

	const sensors = useSensors(pointerSensor);

	const handleDragStart = useCallback(
		(event: DragStartEvent) => {
			const { active } = event;
			setActiveId(active.id as string);
			setActiveData(active.data.current as DragData);
			lastDropZoneRef.current = null;
			setIsDraggingGlobal(true);
		},
		[setIsDraggingGlobal],
	);

	const handleDragOver = useCallback((event: DragOverEvent) => {
		const { over } = event;
		const nextOverData = over?.data.current as DropData | null;
		setOverId(over?.id as string | null);
		setOverData(nextOverData);
		if (nextOverData?.type === "drop-zone") {
			lastDropZoneRef.current = nextOverData;
		}
	}, []);

	// Move component from one parent to another
	const moveComponent = useCallback(
		(
			movingId: string,
			fromParentId: string | null,
			toParentId: string,
			toIndex?: number,
		) => {
			const toParent = components.get(toParentId);
			if (!toParent) return;

			const toChildrenData = (
				toParent.component as unknown as Record<string, unknown>
			).children as Children | undefined;
			const toChildren =
				toChildrenData && "explicitList" in toChildrenData
					? [...toChildrenData.explicitList]
					: [];

			if (fromParentId === toParentId) {
				const currentIndex = toChildren.indexOf(movingId);
				if (currentIndex === -1) return;

				toChildren.splice(currentIndex, 1);
				const insertIndex =
					toIndex !== undefined
						? toIndex > currentIndex
							? toIndex - 1
							: toIndex
						: toChildren.length;
				toChildren.splice(insertIndex, 0, movingId);

				updateComponent(toParentId, {
					component: {
						...toParent.component,
						children: { explicitList: toChildren },
					} as A2UIComponent,
				});
			} else {
				if (fromParentId) {
					const fromParent = components.get(fromParentId);
					if (fromParent) {
						const fromChildrenData = (
							fromParent.component as unknown as Record<string, unknown>
						).children as Children | undefined;
						const fromChildren =
							fromChildrenData && "explicitList" in fromChildrenData
								? fromChildrenData.explicitList
								: [];
						updateComponent(fromParentId, {
							component: {
								...fromParent.component,
								children: {
									explicitList: fromChildren.filter((id) => id !== movingId),
								},
							} as A2UIComponent,
						});
					}
				}

				const insertIndex = toIndex !== undefined ? toIndex : toChildren.length;
				toChildren.splice(insertIndex, 0, movingId);

				updateComponent(toParentId, {
					component: {
						...toParent.component,
						children: { explicitList: toChildren },
					} as A2UIComponent,
				});
			}
		},
		[components, updateComponent],
	);

	// Insert widget instance
	const insertWidgetInstance = useCallback(
		async (
			widgetData: WidgetDragData,
			parentId: string,
			insertIndex?: number,
		) => {
			const { appId, widgetId } = widgetData;
			const parent = components.get(parentId);
			if (!parent) return;

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

			const widgetComponentIds = new Set(widget.components.map((c) => c.id));
			const effectiveRootId = widgetComponentIds.has("root")
				? "root"
				: widgetComponentIds.has(widget.rootComponentId)
					? widget.rootComponentId
					: (widget.components[0]?.id ?? widget.rootComponentId);

			const instanceId = `widget-${widgetId}-${Date.now()}`;
			const widgetInstanceComponentId = `widgetInstance-${instanceId}`;

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

			addComponent(widgetInstanceComponent);

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

	// Insert a package-shipped micro widget instance (self-contained component,
	// contract + defaults embedded — no widgetRef, no backend fetch).
	const insertPackageWidgetInstance = useCallback(
		(
			widgetData: PackageWidgetDragData,
			parentId: string,
			insertIndex?: number,
		) => {
			const parent = components.get(parentId);
			if (!parent) return;

			const instanceId = `micro-${widgetData.widgetId}-${Date.now()}`;
			const componentId = `microWidgetInstance-${instanceId}`;

			addComponent({
				id: componentId,
				component: {
					type: "microWidgetInstance",
					instanceId,
					packageId: widgetData.packageId,
					widgetId: widgetData.widgetId,
					packageVersion: widgetData.packageVersion,
					bundleHash: widgetData.bundleHash,
					contract: widgetData.contract,
					props: contractDefaults(widgetData.contract),
					actionBindings: {},
				} as A2UIComponent,
			});

			const parentChildren = (
				parent.component as unknown as Record<string, unknown>
			)?.children as Children | undefined;
			const existingChildren =
				parentChildren && "explicitList" in parentChildren
					? [...parentChildren.explicitList]
					: [];

			if (insertIndex !== undefined) {
				existingChildren.splice(insertIndex, 0, componentId);
			} else {
				existingChildren.push(componentId);
			}

			updateComponent(parentId, {
				component: {
					...parent.component,
					children: { explicitList: existingChildren },
				} as A2UIComponent,
			});
		},
		[components, addComponent, updateComponent],
	);

	const handleDragEnd = useCallback(
		(event: DragEndEvent) => {
			const { active, over } = event;

			// Reset state
			setActiveId(null);
			setActiveData(null);
			setOverId(null);
			setOverData(null);
			setIsDraggingGlobal(false);

			if (!active.data.current) return;

			const dragData = active.data.current as DragData;
			const directDropData = (over?.data.current as DropData | null) ?? null;
			const dropData =
				directDropData?.type === "drop-zone"
					? directDropData
					: (lastDropZoneRef.current ?? directDropData);

			lastDropZoneRef.current = null;

			if (!dropData) return;

			const parentId = dropData.parentId;
			const index = dropData.index;

			const parent = components.get(parentId);
			if (!parent) return;

			const parentChildrenData = (
				parent.component as unknown as Record<string, unknown>
			)?.children as Children | undefined;
			const existingChildren =
				parentChildrenData && "explicitList" in parentChildrenData
					? [...parentChildrenData.explicitList]
					: [];

			if (dragData.type === WIDGET_DND_TYPE) {
				insertWidgetInstance(dragData, parentId, index);
			} else if (dragData.type === PACKAGE_WIDGET_DND_TYPE) {
				insertPackageWidgetInstance(dragData, parentId, index);
			} else if (dragData.type === COMPONENT_DND_TYPE) {
				const newId = `${dragData.componentType}-${Date.now()}`;
				const defaultStyle = getDefaultStyle(dragData.componentType);
				const newComponent: SurfaceComponent = {
					id: newId,
					component: createDefaultComponent(dragData.componentType),
					...(defaultStyle && { style: defaultStyle }),
				};
				addComponent(newComponent);

				if (index !== undefined) {
					existingChildren.splice(index, 0, newId);
				} else {
					existingChildren.push(newId);
				}
				updateComponent(parentId, {
					component: {
						...parent.component,
						children: { explicitList: existingChildren },
					} as A2UIComponent,
				});
			} else if (dragData.type === COMPONENT_MOVE_TYPE) {
				moveComponent(
					dragData.componentId,
					dragData.currentParentId,
					parentId,
					index,
				);
			}
		},
		[
			setIsDraggingGlobal,
			components,
			addComponent,
			updateComponent,
			moveComponent,
			insertWidgetInstance,
			insertPackageWidgetInstance,
		],
	);

	const handleDragCancel = useCallback(() => {
		setActiveId(null);
		setActiveData(null);
		setOverId(null);
		setOverData(null);
		lastDropZoneRef.current = null;
		setIsDraggingGlobal(false);
	}, [setIsDraggingGlobal]);

	return (
		<DndContext
			sensors={sensors}
			collisionDetection={customCollisionDetection}
			onDragStart={handleDragStart}
			onDragOver={handleDragOver}
			onDragEnd={handleDragEnd}
			onDragCancel={handleDragCancel}
		>
			<BuilderDndReactContext.Provider
				value={{ activeId, activeData, overId, overData }}
			>
				{children}
			</BuilderDndReactContext.Provider>
		</DndContext>
	);
}
