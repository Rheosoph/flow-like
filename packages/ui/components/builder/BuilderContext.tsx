"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type { A2UIComponent, SurfaceComponent } from "../a2ui/types";
import {
	type BuilderClipboard,
	collectClipboard,
	pasteClipboard,
} from "./builderClipboard";
import { moveComponentInTree } from "./componentTree";

export type { BuilderClipboard } from "./builderClipboard";

export interface BuilderSelection {
	componentIds: string[];
	surfaceId?: string;
}

export interface BuilderSnapshot {
	components: SurfaceComponent[];
	widgetRefs: Record<string, IWidgetRef>;
}

export interface TransformState {
	isDragging: boolean;
	isResizing: boolean;
	resizeHandle?: "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";
	dragStart?: { x: number; y: number };
	originalBounds?: { x: number; y: number; width: number; height: number };
}

export interface BuilderHistory {
	past: BuilderSnapshot[];
	present: BuilderSnapshot;
	future: BuilderSnapshot[];
}

export interface CanvasSettings {
	backgroundColor: string;
	backgroundImage?: string;
	padding: string;
	/** Custom CSS to inject into the canvas (scoped to canvas container) */
	customCss?: string;
}

export interface PageInfo {
	id: string;
	name: string;
	boardId?: string;
}

export interface WorkflowEventInfo {
	nodeId: string;
	name: string;
}

export interface ActionContext {
	appId?: string;
	boardId?: string;
	pages?: PageInfo[];
	workflowEvents?: WorkflowEventInfo[];
	/** Widget-level actions that can be triggered by components inside the widget */
	widgetActions?: { id: string; label: string; description?: string }[];
	eventId?: string;
	/** Page behavior hooks for preview mode */
	pageId?: string;
	onLoadEventId?: string;
	onUnloadEventId?: string;
	onIntervalEventId?: string;
	onIntervalSeconds?: number;
}

export interface BuilderContextType {
	// Selection
	selection: BuilderSelection;
	setSelection: (selection: BuilderSelection) => void;
	selectComponent: (componentId: string, multi?: boolean) => void;
	deselectAll: () => void;
	isSelected: (componentId: string) => boolean;

	// Clipboard
	clipboard: BuilderClipboard | null;
	copy: (ids?: string[]) => void;
	cut: (ids?: string[]) => void;
	paste: (parentId?: string) => void;
	duplicate: () => void;

	// Transform
	transform: TransformState;
	setTransform: (state: Partial<TransformState>) => void;

	// History (undo/redo)
	canUndo: boolean;
	canRedo: boolean;
	undo: () => void;
	redo: () => void;
	pushHistory: () => void;

	// Components
	components: Map<string, SurfaceComponent>;
	addComponent: (component: SurfaceComponent, parentId?: string) => void;
	addComponents: (components: SurfaceComponent[]) => void;
	replaceComponents: (components: SurfaceComponent[]) => void;
	updateComponent: (id: string, updates: Partial<SurfaceComponent>) => void;
	deleteComponents: (ids: string[]) => void;
	moveComponent: (id: string, newParentId: string, index?: number) => void;
	getComponent: (id: string) => SurfaceComponent | undefined;

	// Widget refs - widget definitions stored by instance ID
	widgetRefs: Map<string, IWidgetRef>;
	addWidgetRef: (instanceId: string, widget: IWidgetRef) => void;
	getWidgetRef: (instanceId: string) => IWidgetRef | undefined;
	removeWidgetRef: (instanceId: string) => void;

	// Drag state
	isDraggingGlobal: boolean;
	setIsDraggingGlobal: (dragging: boolean) => void;

	// Canvas settings
	canvasSettings: CanvasSettings;
	setCanvasSettings: (settings: Partial<CanvasSettings>) => void;

	// Viewport
	zoom: number;
	setZoom: (zoom: number) => void;
	pan: { x: number; y: number };
	setPan: (pan: { x: number; y: number }) => void;

	// Settings
	showGrid: boolean;
	setShowGrid: (show: boolean) => void;
	snapToGrid: boolean;
	setSnapToGrid: (snap: boolean) => void;
	gridSize: number;
	setGridSize: (size: number) => void;

	// Action context for action editor
	actionContext?: ActionContext;

	// Visibility - hide components in builder preview
	hiddenComponents: Set<string>;
	toggleComponentVisibility: (componentId: string) => void;
	isComponentHidden: (componentId: string) => boolean;
	setComponentHidden: (componentId: string, hidden: boolean) => void;

	// Dev mode - raw JSON editing
	devMode: boolean;
	setDevMode: (devMode: boolean) => void;
	getRawJson: () => string;
	setRawJson: (json: string) => boolean; // returns true if successful
}

const BuilderContext = createContext<BuilderContextType | null>(null);

export function useBuilder() {
	const context = useContext(BuilderContext);
	if (!context) {
		throw new Error("useBuilder must be used within a BuilderProvider");
	}
	return context;
}

export interface BuilderProviderProps {
	children: ReactNode;
	initialComponents?: SurfaceComponent[];
	initialWidgetRefs?: Record<string, IWidgetRef>;
	onChange?: (
		components: SurfaceComponent[],
		widgetRefs: Record<string, IWidgetRef>,
	) => void;
	initialCanvasSettings?: Partial<CanvasSettings>;
	onCanvasSettingsChange?: (settings: CanvasSettings) => void;
	actionContext?: ActionContext;
}

export function BuilderProvider({
	children,
	initialComponents = [],
	initialWidgetRefs = {},
	onChange,
	initialCanvasSettings,
	onCanvasSettingsChange,
	actionContext,
}: BuilderProviderProps) {
	// Selection state
	const [selection, setSelection] = useState<BuilderSelection>({
		componentIds: [],
	});

	// Clipboard state - load from localStorage on mount
	const [clipboard, setClipboardState] = useState<BuilderClipboard | null>(
		null,
	);

	const clipboardRef = useRef(clipboard);
	clipboardRef.current = clipboard;
	const [clipboardSourceId] = useState(createId);

	// Load clipboard from localStorage on mount
	useEffect(() => {
		try {
			const stored = localStorage.getItem("a2ui-clipboard");
			if (stored) {
				const parsed = JSON.parse(stored);
				if (
					Array.isArray(parsed?.components) &&
					parsed.components.every(
						(component: SurfaceComponent) =>
							typeof component?.id === "string" &&
							typeof component.component?.type === "string",
					) &&
					Array.isArray(parsed.rootIds) &&
					parsed.rootIds.every((id: unknown) => typeof id === "string")
				) {
					const restored = { ...parsed, cut: false, sourceId: undefined };
					clipboardRef.current = restored;
					setClipboardState(restored);
				}
			}
		} catch (e) {
			console.warn("Failed to load clipboard from localStorage", e);
		}
	}, []);

	// Wrapper to save clipboard to localStorage
	const setClipboard = useCallback((value: BuilderClipboard | null) => {
		clipboardRef.current = value;
		setClipboardState(value);
		try {
			if (value) {
				localStorage.setItem("a2ui-clipboard", JSON.stringify(value));
			} else {
				localStorage.removeItem("a2ui-clipboard");
			}
		} catch (e) {
			console.warn("Failed to save clipboard to localStorage", e);
		}
	}, []);

	// Transform state
	const [transform, setTransformState] = useState<TransformState>({
		isDragging: false,
		isResizing: false,
	});

	// History state
	const [history, setHistory] = useState<BuilderHistory>({
		past: [],
		present: { components: initialComponents, widgetRefs: initialWidgetRefs },
		future: [],
	});

	// Components map for quick access
	const [componentsMap, setComponentsMap] = useState<
		Map<string, SurfaceComponent>
	>(() => new Map(initialComponents.map((c) => [c.id, c])));

	// Widget refs map - stores widget definitions by instance ID
	const [widgetRefsMap, setWidgetRefsMap] = useState<Map<string, IWidgetRef>>(
		() => new Map(Object.entries(initialWidgetRefs)),
	);

	const componentsRef = useRef(componentsMap);
	componentsRef.current = componentsMap;
	const widgetRefsRef = useRef(widgetRefsMap);
	widgetRefsRef.current = widgetRefsMap;
	const getSnapshot = useCallback(
		(): BuilderSnapshot => ({
			components: Array.from(componentsRef.current.values()),
			widgetRefs: Object.fromEntries(widgetRefsRef.current),
		}),
		[],
	);

	// Track if this is the first render to avoid calling onChange on mount
	const isFirstRender = useRef(true);
	// Store onChange in ref to avoid dependency issues
	const onChangeRef = useRef(onChange);
	onChangeRef.current = onChange;

	// Notify onChange when components or widgetRefs change (not on initial mount)
	useEffect(() => {
		if (isFirstRender.current) {
			isFirstRender.current = false;
			return;
		}
		onChangeRef.current?.(
			Array.from(componentsMap.values()),
			Object.fromEntries(widgetRefsMap),
		);
	}, [componentsMap, widgetRefsMap]);

	// Viewport state
	const [zoom, setZoom] = useState(1);
	const [pan, setPan] = useState({ x: 0, y: 0 });

	// Grid settings
	const [showGrid, setShowGrid] = useState(true);
	const [snapToGrid, setSnapToGrid] = useState(true);
	const [gridSize, setGridSize] = useState(8);

	// Hidden components state (for builder preview only, not persisted)
	const [hiddenComponents, setHiddenComponents] = useState<Set<string>>(
		new Set(),
	);

	// Dev mode state
	const [devMode, setDevMode] = useState(false);

	// Canvas settings - initialize from props if provided
	const [canvasSettings, setCanvasSettingsState] = useState<CanvasSettings>({
		backgroundColor:
			initialCanvasSettings?.backgroundColor ?? "var(--background)",
		backgroundImage: initialCanvasSettings?.backgroundImage,
		padding: initialCanvasSettings?.padding ?? "16px",
		customCss: initialCanvasSettings?.customCss,
	});

	// Global drag state to prevent text selection during drag
	const [isDraggingGlobal, setIsDraggingGlobal] = useState(false);

	// Visibility methods
	const toggleComponentVisibility = useCallback((componentId: string) => {
		setHiddenComponents((prev) => {
			const next = new Set(prev);
			if (next.has(componentId)) {
				next.delete(componentId);
			} else {
				next.add(componentId);
			}
			return next;
		});
	}, []);

	const isComponentHidden = useCallback(
		(componentId: string) => hiddenComponents.has(componentId),
		[hiddenComponents],
	);

	const setComponentHidden = useCallback(
		(componentId: string, hidden: boolean) => {
			setHiddenComponents((prev) => {
				const next = new Set(prev);
				if (hidden) {
					next.add(componentId);
				} else {
					next.delete(componentId);
				}
				return next;
			});
		},
		[],
	);

	// History methods (defined early since other methods depend on pushHistory)
	const pushHistory = useCallback(() => {
		const snapshot = getSnapshot();
		setHistory((prev) => ({
			past: [...prev.past, snapshot],
			present: snapshot,
			future: [],
		}));
	}, [getSnapshot]);

	// Dev mode methods
	const getRawJson = useCallback(() => {
		const data = {
			components: Array.from(componentsMap.values()),
			widgetRefs: Object.fromEntries(widgetRefsMap),
			canvasSettings: canvasSettings,
		};
		return JSON.stringify(data, null, 2);
	}, [componentsMap, widgetRefsMap, canvasSettings]);

	const setRawJson = useCallback(
		(json: string): boolean => {
			try {
				const data = JSON.parse(json);
				if (!data.components || !Array.isArray(data.components)) {
					console.error("Invalid JSON: missing or invalid components array");
					return false;
				}

				// Validate components have required fields
				for (const comp of data.components) {
					if (!comp.id || !comp.component) {
						console.error("Invalid component: missing id or component", comp);
						return false;
					}
				}

				// Update components
				setComponentsMap(
					new Map(data.components.map((c: SurfaceComponent) => [c.id, c])),
				);

				// Update widget refs if present
				if (data.widgetRefs && typeof data.widgetRefs === "object") {
					setWidgetRefsMap(new Map(Object.entries(data.widgetRefs)));
				}

				// Update canvas settings if present - use setCanvasSettings to trigger onCanvasSettingsChange
				if (data.canvasSettings && typeof data.canvasSettings === "object") {
					const newSettings = { ...canvasSettings, ...data.canvasSettings };
					setCanvasSettingsState(newSettings);
					onCanvasSettingsChange?.(newSettings);
				}

				pushHistory();
				return true;
			} catch (e) {
				console.error("Failed to parse JSON:", e);
				return false;
			}
		},
		[pushHistory, canvasSettings, onCanvasSettingsChange],
	);

	const setCanvasSettings = useCallback(
		(settings: Partial<CanvasSettings>) => {
			setCanvasSettingsState((prev) => {
				const newSettings = { ...prev, ...settings };
				onCanvasSettingsChange?.(newSettings);
				return newSettings;
			});
		},
		[onCanvasSettingsChange],
	);

	// Selection methods
	const selectComponent = useCallback((componentId: string, multi = false) => {
		setSelection((prev) => {
			if (multi) {
				const isAlreadySelected = prev.componentIds.includes(componentId);
				return {
					...prev,
					componentIds: isAlreadySelected
						? prev.componentIds.filter((id) => id !== componentId)
						: [...prev.componentIds, componentId],
				};
			}
			return { ...prev, componentIds: [componentId] };
		});
	}, []);

	const deselectAll = useCallback(() => {
		setSelection({ componentIds: [] });
	}, []);

	const isSelected = useCallback(
		(componentId: string) => selection.componentIds.includes(componentId),
		[selection.componentIds],
	);

	// Clipboard snapshots include named content slots and widget definitions.
	const captureClipboard = useCallback(
		(cut: boolean, ids?: string[]) =>
			collectClipboard(
				componentsRef.current,
				widgetRefsRef.current,
				Array.isArray(ids) ? ids : selection.componentIds,
				cut,
				clipboardSourceId,
			),
		[selection.componentIds, clipboardSourceId],
	);

	const copy = useCallback(
		(ids?: string[]) => {
			const value = captureClipboard(false, ids);
			if (value) setClipboard(value);
		},
		[captureClipboard, setClipboard],
	);

	const cut = useCallback(
		(ids?: string[]) => {
			const value = captureClipboard(true, ids);
			if (value) setClipboard(value);
		},
		[captureClipboard, setClipboard],
	);

	const applyClipboard = useCallback(
		(value: BuilderClipboard, parentId?: string, duplicate = false) => {
			const previous = getSnapshot();
			const result = pasteClipboard({
				components: componentsRef.current,
				widgetRefs: widgetRefsRef.current,
				clipboard: value,
				selectionIds: selection.componentIds,
				sourceId: clipboardSourceId,
				parentId,
				duplicate,
			});
			if (!result) return;
			if (result.components !== componentsRef.current) {
				componentsRef.current = result.components;
				widgetRefsRef.current = result.widgetRefs;
				setComponentsMap(result.components);
				setWidgetRefsMap(result.widgetRefs);
				const present = getSnapshot();
				setHistory((history) => ({
					past: [...history.past, previous],
					present,
					future: [],
				}));
			}
			if (result.consumedCut) setClipboard(null);
			setSelection({ componentIds: result.rootIds });
		},
		[getSnapshot, selection.componentIds, setClipboard, clipboardSourceId],
	);

	const paste = useCallback(
		(parentId?: string) => {
			if (clipboardRef.current) applyClipboard(clipboardRef.current, parentId);
		},
		[applyClipboard],
	);

	const duplicate = useCallback(() => {
		const value = captureClipboard(false);
		if (value) applyClipboard(value, undefined, true);
	}, [captureClipboard, applyClipboard]);

	// Transform methods
	const setTransform = useCallback((state: Partial<TransformState>) => {
		setTransformState((prev) => ({ ...prev, ...state }));
	}, []);

	const restoreSnapshot = useCallback((snapshot: BuilderSnapshot) => {
		const components = new Map(
			snapshot.components.map((component) => [component.id, component]),
		);
		const refs = new Map(Object.entries(snapshot.widgetRefs));
		componentsRef.current = components;
		widgetRefsRef.current = refs;
		setComponentsMap(components);
		setWidgetRefsMap(refs);
		setSelection((selection) => ({
			...selection,
			componentIds: selection.componentIds.filter((id) => components.has(id)),
		}));
	}, []);

	const undo = useCallback(() => {
		const newPresent = history.past[history.past.length - 1];
		if (!newPresent) return;
		const current = getSnapshot();
		restoreSnapshot(newPresent);
		setHistory((prev) => ({
			past: prev.past.slice(0, -1),
			present: newPresent,
			future: [current, ...prev.future],
		}));
	}, [history, getSnapshot, restoreSnapshot]);

	const redo = useCallback(() => {
		const newPresent = history.future[0];
		if (!newPresent) return;
		const current = getSnapshot();
		restoreSnapshot(newPresent);
		setHistory((prev) => ({
			past: [...prev.past, current],
			present: newPresent,
			future: prev.future.slice(1),
		}));
	}, [history, getSnapshot, restoreSnapshot]);

	// Component methods
	const addComponent = useCallback(
		(component: SurfaceComponent, parentId?: string) => {
			pushHistory();
			setComponentsMap((prev) => {
				const next = new Map(prev);
				next.set(component.id, component);
				return next;
			});
		},
		[pushHistory],
	);

	// Batch add multiple components at once (single history entry, single state update)
	const addComponents = useCallback(
		(components: SurfaceComponent[]) => {
			if (components.length === 0) return;
			pushHistory();
			setComponentsMap((prev) => {
				const next = new Map(prev);
				for (const comp of components) {
					next.set(comp.id, comp);
				}
				return next;
			});
		},
		[pushHistory],
	);

	const replaceComponents = useCallback(
		(components: SurfaceComponent[]) => {
			pushHistory();
			setComponentsMap(new Map(components.map((comp) => [comp.id, comp])));
		},
		[pushHistory],
	);

	const updateComponent = useCallback(
		(id: string, updates: Partial<SurfaceComponent>) => {
			const newId = updates.id;
			const isIdChange = newId !== undefined && newId !== id;

			setComponentsMap((prev) => {
				const component = prev.get(id);
				if (!component) return prev;
				const next = new Map(prev);

				if (isIdChange) {
					// Remove old entry and add with new key
					next.delete(id);
					next.set(newId, { ...component, ...updates });

					// Update parent references to the old ID
					for (const [parentKey, parentComp] of next) {
						if (!parentComp.component) continue;
						const props = parentComp.component as unknown as Record<
							string,
							unknown
						>;
						let updated = false;
						const updatedProps = { ...props };

						// Update children.explicitList
						if ("children" in props && props.children) {
							const children = props.children as { explicitList?: string[] };
							if (children.explicitList?.includes(id)) {
								updatedProps.children = {
									...children,
									explicitList: children.explicitList.map((cid) =>
										cid === id ? newId : cid,
									),
								};
								updated = true;
							}
						}

						// Update child property
						if ("child" in props && props.child === id) {
							updatedProps.child = newId;
							updated = true;
						}

						// Update entryPointChild property
						if ("entryPointChild" in props && props.entryPointChild === id) {
							updatedProps.entryPointChild = newId;
							updated = true;
						}

						// Update contentChild property
						if ("contentChild" in props && props.contentChild === id) {
							updatedProps.contentChild = newId;
							updated = true;
						}

						if (updated) {
							next.set(parentKey, {
								...parentComp,
								component:
									updatedProps as unknown as typeof parentComp.component,
							});
						}
					}
				} else {
					next.set(id, { ...component, ...updates });
				}

				return next;
			});

			// Update selection if ID changed
			if (isIdChange) {
				setSelection((prev) => ({
					...prev,
					componentIds: prev.componentIds.map((cid) =>
						cid === id ? newId : cid,
					),
				}));
			}
		},
		[],
	);

	const deleteComponents = useCallback(
		(ids: string[]) => {
			const collectDescendants = (
				map: Map<string, SurfaceComponent>,
				componentId: string,
			): string[] => {
				const descendants: string[] = [];
				const component = map.get(componentId);
				if (!component?.component) return descendants;

				const props = component.component as unknown as Record<string, unknown>;
				const childIds: string[] = [];

				if ("children" in props && props.children) {
					const children = props.children as { explicitList?: string[] };
					if (children.explicitList) {
						childIds.push(...children.explicitList);
					}
				}
				if ("child" in props && typeof props.child === "string") {
					childIds.push(props.child);
				}
				if (
					"entryPointChild" in props &&
					typeof props.entryPointChild === "string"
				) {
					childIds.push(props.entryPointChild);
				}
				if ("contentChild" in props && typeof props.contentChild === "string") {
					childIds.push(props.contentChild);
				}

				for (const childId of childIds) {
					descendants.push(childId);
					descendants.push(...collectDescendants(map, childId));
				}

				return descendants;
			};

			const allIdsToDelete = new Set<string>();
			for (const id of ids) {
				allIdsToDelete.add(id);
				for (const descendantId of collectDescendants(componentsMap, id)) {
					allIdsToDelete.add(descendantId);
				}
			}

			pushHistory();
			setComponentsMap((prev) => {
				const next = new Map(prev);

				for (const [componentId, component] of next) {
					if (allIdsToDelete.has(componentId) || !component.component) continue;

					const props = component.component as unknown as Record<
						string,
						unknown
					>;
					const children = props.children as
						| { explicitList?: string[] }
						| undefined;
					if (!children?.explicitList?.some((id) => allIdsToDelete.has(id))) {
						continue;
					}

					next.set(componentId, {
						...component,
						component: {
							...component.component,
							children: {
								...children,
								explicitList: children.explicitList.filter(
									(id) => !allIdsToDelete.has(id),
								),
							},
						} as A2UIComponent,
					});
				}

				for (const id of allIdsToDelete) {
					next.delete(id);
				}

				return next;
			});
			setSelection((prev) => ({
				...prev,
				componentIds: prev.componentIds.filter((id) => !allIdsToDelete.has(id)),
			}));
		},
		[componentsMap, pushHistory],
	);

	const moveComponent = useCallback(
		(id: string, newParentId: string, index?: number) => {
			const next = moveComponentInTree(componentsMap, id, newParentId, index);
			if (next === componentsMap) return;
			const snapshot = getSnapshot();
			setHistory((previous) => ({
				past: [...previous.past, snapshot],
				present: {
					components: Array.from(next.values()),
					widgetRefs: Object.fromEntries(widgetRefsRef.current),
				},
				future: [],
			}));
			setComponentsMap(next);
		},
		[componentsMap, getSnapshot],
	);

	const getComponent = useCallback(
		(id: string) => componentsMap.get(id),
		[componentsMap],
	);

	// Widget ref methods
	const addWidgetRef = useCallback((instanceId: string, widget: IWidgetRef) => {
		setWidgetRefsMap((prev) => {
			const next = new Map(prev);
			next.set(instanceId, widget);
			return next;
		});
	}, []);

	const getWidgetRef = useCallback(
		(instanceId: string) => widgetRefsMap.get(instanceId),
		[widgetRefsMap],
	);

	const removeWidgetRef = useCallback((instanceId: string) => {
		setWidgetRefsMap((prev) => {
			const next = new Map(prev);
			next.delete(instanceId);
			return next;
		});
	}, []);

	const value: BuilderContextType = {
		selection,
		setSelection,
		selectComponent,
		deselectAll,
		isSelected,

		clipboard,
		copy,
		cut,
		paste,
		duplicate,

		transform,
		setTransform,

		canUndo: history.past.length > 0,
		canRedo: history.future.length > 0,
		undo,
		redo,
		pushHistory,

		components: componentsMap,
		addComponent,
		addComponents,
		replaceComponents,
		updateComponent,
		deleteComponents,
		moveComponent,
		getComponent,

		widgetRefs: widgetRefsMap,
		addWidgetRef,
		getWidgetRef,
		removeWidgetRef,

		zoom,
		setZoom,
		pan,
		setPan,

		showGrid,
		setShowGrid,
		snapToGrid,
		setSnapToGrid,
		gridSize,
		setGridSize,

		isDraggingGlobal,
		setIsDraggingGlobal,
		canvasSettings,
		setCanvasSettings,

		hiddenComponents,
		toggleComponentVisibility,
		isComponentHidden,
		setComponentHidden,

		devMode,
		setDevMode,
		getRawJson,
		setRawJson,

		actionContext,
	};

	return (
		<BuilderContext.Provider value={value}>{children}</BuilderContext.Provider>
	);
}
