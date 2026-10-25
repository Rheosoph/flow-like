"use client";

import {
	DndContext,
	type DragEndEvent,
	type DragMoveEvent,
	type DragStartEvent,
	MeasuringStrategy,
	PointerSensor,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import { type WidgetContract, contractDefaults } from "@flow-like/widget-sdk";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type { A2UIComponent, Children, SurfaceComponent } from "../a2ui/types";
import { useBuilder } from "./BuilderContext";
import {
	createBuilderCollisionDetection,
	measureBuilderDroppable,
} from "./builderCollisionDetection";
import { canAcceptComponentChildren } from "./componentTree";
import type { DropRect } from "./dropPlacement";
import { importWidget } from "./import-widget";

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
 * contract so dropping needs no backend round trip. The placed
 * `microWidgetInstance` carries its definition.
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
	/** Insertion marker in viewport coordinates, for the canvas portal. */
	indicator?: DropRect;
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

	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { t } = useTranslation("flow");
	const {
		components,
		addComponent,
		updateComponent,
		insertWidgetInstance,
		actionContext,
		moveComponent,
	} = useBuilder();
	const collisionDetection = useMemo(
		() => createBuilderCollisionDetection(components),
		[components],
	);

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
			setIsDraggingGlobal(true);
		},
		[setIsDraggingGlobal],
	);

	const handleDragOver = useCallback((event: DragMoveEvent) => {
		const collision = event.collisions?.[0];
		setOverId(collision ? String(collision.id) : null);
		setOverData((collision?.data?.dropData as DropData | undefined) ?? null);
	}, []);

	const importWidgetInstance = useCallback(
		async (
			widgetData: WidgetDragData,
			parentId: string,
			insertIndex?: number,
		) => {
			if (!canAcceptComponentChildren(components.get(parentId))) return;
			const targetAppId = actionContext?.appId ?? widgetData.appId;
			try {
				const widget = await importWidget(
					backend.widgetState,
					widgetData.appId,
					widgetData.widgetId,
					targetAppId,
				);
				insertWidgetInstance(widget, targetAppId, parentId, insertIndex);
				if (targetAppId !== widgetData.appId) {
					void invalidate(backend.widgetState.getWidgets, [targetAppId]);
					void invalidate(backend.userState.getUserWidgets, []);
				}
			} catch (error) {
				console.error("Failed to add widget:", error);
				toast.error(
					t("failedToAddWidget", "Failed to add widget to this project"),
				);
			}
		},
		[
			backend.widgetState,
			backend.userState,
			components,
			actionContext?.appId,
			insertWidgetInstance,
			invalidate,
			t,
		],
	);

	// Insert a package-shipped micro widget instance (self-contained component,
	// with its contract and defaults embedded).
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
		async (event: DragEndEvent) => {
			const { active } = event;

			// Reset state
			setActiveId(null);
			setActiveData(null);
			setOverId(null);
			setOverData(null);
			setIsDraggingGlobal(false);

			if (!active.data.current) return;

			const dragData = active.data.current as DragData;
			const dropData = event.collisions?.[0]?.data?.dropData as
				| DropData
				| undefined;

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
				await importWidgetInstance(dragData, parentId, index);
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
				moveComponent(dragData.componentId, parentId, index);
			}
		},
		[
			setIsDraggingGlobal,
			components,
			addComponent,
			updateComponent,
			moveComponent,
			importWidgetInstance,
			insertPackageWidgetInstance,
		],
	);

	const handleDragCancel = useCallback(() => {
		setActiveId(null);
		setActiveData(null);
		setOverId(null);
		setOverData(null);
		setIsDraggingGlobal(false);
	}, [setIsDraggingGlobal]);

	return (
		<DndContext
			sensors={sensors}
			collisionDetection={collisionDetection}
			measuring={{
				droppable: {
					strategy: MeasuringStrategy.Always,
					measure: measureBuilderDroppable,
				},
			}}
			onDragStart={handleDragStart}
			onDragOver={handleDragOver}
			onDragMove={handleDragOver}
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
