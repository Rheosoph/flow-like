"use client";

import {
	type Edge,
	type EdgeChange,
	type InternalNode,
	type Node,
	type NodeChange,
	type OnSelectionChangeFunc,
	type ReactFlowInstance,
	type ReactFlowState,
	type Viewport,
	applyEdgeChanges,
	applyNodeChanges,
	useInternalNode as useLegacyInternalNode,
	useNodes as useLegacyNodes,
	useReactFlow as useLegacyReactFlow,
	useStore as useLegacyStore,
	useStoreApi as useLegacyStoreApi,
	useViewport as useLegacyViewport,
} from "@xyflow/react";
import {
	Position,
	adoptUserNodes,
	getNodesBounds,
	getViewportForBounds,
} from "@xyflow/system";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import { betweenFrames } from "../../lib/board-canvas/frames";

export { useEdgesState, useNodesState } from "@xyflow/react";

export type BoardRuntimeState = ReactFlowState & {
	renderer: "egui" | "react-flow";
	viewportElement: HTMLElement | null;
	overlayElement: HTMLElement | null;
	geometryRevision: number;
	measuredGeometryRevision: number;
	domMeasuredNodeLookup: ReadonlyMap<string, InternalNode>;
	viewportInitialized: boolean;
	onSelectionChange?: OnSelectionChangeFunc;
};
type StateKey = keyof BoardRuntimeState;
type Listener = (state: BoardRuntimeState, previous: BoardRuntimeState) => void;
type StatePatch =
	| Partial<BoardRuntimeState>
	| ((state: BoardRuntimeState) => Partial<BoardRuntimeState>);

export interface BoardRuntimeStore {
	getState(): BoardRuntimeState;
	getInitialState(): BoardRuntimeState;
	setState(patch: StatePatch): void;
	subscribe(listener: Listener): () => void;
	subscribeKeys(
		listener: () => void,
		keys: () => ReadonlySet<StateKey>,
	): () => void;
}

export interface BoardRuntime {
	store: BoardRuntimeStore;
	flow: ReactFlowInstance;
	configure(config: Partial<BoardRuntimeState>): void;
	setViewport(
		viewport: Viewport,
		options?: {
			duration?: number;
			ease?: (t: number) => number;
			notify?: boolean;
		},
	): Promise<boolean>;
	attachLegacy(
		flow: ReactFlowInstance,
		store: ReturnType<typeof useLegacyStoreApi>,
	): () => void;
	destroy(): void;
}

const infiniteExtent: [[number, number], [number, number]] = [
	[Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY],
	[Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY],
];

type HandleBounds = InternalNode["internals"]["handleBounds"];

function sameMeasuredHandles(a: HandleBounds, b: HandleBounds): boolean {
	if (!a || !b) return a === b;
	// Transformed DOM rectangles pick up float noise when a culled editor remounts at a new zoom.
	// Keep subpixel-stable graph coordinates; the tolerance remains below 0.1 CSS px at max zoom.
	const epsilon = 1 / 32;
	for (const type of ["source", "target"] as const) {
		const first = a[type] ?? [];
		const second = b[type] ?? [];
		if (first.length !== second.length) return false;
		for (let index = 0; index < first.length; index++) {
			const left = first[index];
			const right = second[index];
			if (
				left.id !== right.id ||
				left.type !== right.type ||
				left.position !== right.position ||
				Math.abs(left.x - right.x) > epsilon ||
				Math.abs(left.y - right.y) > epsilon ||
				Math.abs(left.width - right.width) > epsilon ||
				Math.abs(left.height - right.height) > epsilon
			)
				return false;
		}
	}
	return true;
}

/** Rescales a camera to `zoom` while keeping the world point under the screen centre fixed. */
export function zoomAboutCenter(
	viewport: Viewport,
	zoom: number,
	width: number,
	height: number,
): Viewport {
	if (zoom === viewport.zoom || !(viewport.zoom > 0)) return viewport;
	const ratio = zoom / viewport.zoom;
	return {
		x: width / 2 - (width / 2 - viewport.x) * ratio,
		y: height / 2 - (height / 2 - viewport.y) * ratio,
		zoom,
	};
}

export function createBoardRuntime(): BoardRuntime {
	const listeners = new Set<Listener>();
	const keyed = new Set<{
		listener: () => void;
		keys: () => ReadonlySet<StateKey>;
	}>();
	let legacy:
		| { flow: ReactFlowInstance; store: ReturnType<typeof useLegacyStoreApi> }
		| undefined;
	let animation:
		| { cancel: () => void; finish: (completed: boolean) => void }
		| undefined;
	let selectedNodeIds: string[] = [];
	let selectedEdgeIds: string[] = [];
	const defaults: Partial<BoardRuntimeState> = {
		renderer: "egui",
		nodes: [],
		edges: [],
		nodeLookup: new Map(),
		edgeLookup: new Map(),
		parentLookup: new Map(),
		transform: [0, 0, 1],
		width: 0,
		height: 0,
		domNode: null,
		viewportElement: null,
		overlayElement: null,
		geometryRevision: 0,
		measuredGeometryRevision: 0,
		domMeasuredNodeLookup: new Map(),
		viewportInitialized: false,
		nodesInitialized: false,
		nodesDraggable: true,
		nodesConnectable: true,
		elementsSelectable: true,
		nodesFocusable: true,
		edgesFocusable: true,
		edgesReconnectable: true,
		snapToGrid: false,
		snapGrid: [15, 15],
		minZoom: 0.02,
		maxZoom: 3,
		nodeOrigin: [0, 0],
		nodeExtent: infiniteExtent,
		translateExtent: infiniteExtent,
		elevateNodesOnSelect: true,
		zIndexMode: "basic",
		paneDragging: false,
		nodesSelectionActive: false,
		userSelectionActive: false,
		onNodesChange: null,
		onEdgesChange: null,
		panZoom: null,
	};
	let state = defaults as BoardRuntimeState;
	const initialState = state;
	const store: BoardRuntimeStore = {
		getState: () => state,
		getInitialState: () => initialState,
		setState(patch) {
			const changes = typeof patch === "function" ? patch(state) : patch;
			const changed = Object.keys(changes).filter(
				(key) => !Object.is(state[key as StateKey], changes[key as StateKey]),
			) as StateKey[];
			if (!changed.length) return;
			const previous = state;
			state = { ...state, ...changes };
			for (const listener of listeners) listener(state, previous);
			for (const subscription of keyed) {
				const keys = subscription.keys();
				if (changed.some((key) => keys.has(key))) subscription.listener();
			}
		},
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		subscribeKeys(listener, keys) {
			const subscription = { listener, keys };
			keyed.add(subscription);
			return () => keyed.delete(subscription);
		},
	};
	const configure: BoardRuntime["configure"] = (config) => {
		const patch = { ...config };
		if (config.nodes && config.nodes !== state.nodes && !config.nodeLookup) {
			const nodeLookup = new Map(state.nodeLookup);
			const parentLookup = new Map();
			adoptUserNodes(config.nodes, nodeLookup, parentLookup, {
				nodeOrigin: state.nodeOrigin,
				nodeExtent: state.nodeExtent,
				elevateNodesOnSelect: state.elevateNodesOnSelect,
				zIndexMode: state.zIndexMode,
			});
			// New board snapshots can omit DOM measurements during a position-only update.
			// Keep their last measured ports until the rich editor measures its next layout.
			for (const [id, measured] of state.domMeasuredNodeLookup) {
				const node = nodeLookup.get(id);
				if (!node || node.handles || node.type !== measured.type) continue;
				if (
					node.measured.width !== undefined &&
					node.measured.height !== undefined &&
					node.internals.handleBounds === measured.internals.handleBounds
				)
					continue;
				nodeLookup.set(id, {
					...node,
					measured: {
						width: node.measured.width ?? measured.measured.width,
						height: node.measured.height ?? measured.measured.height,
					},
					internals: {
						...node.internals,
						handleBounds: measured.internals.handleBounds,
					},
				});
			}
			if (
				[...state.domMeasuredNodeLookup.keys()].some(
					(id) => !nodeLookup.has(id),
				)
			)
				patch.domMeasuredNodeLookup = new Map(
					[...state.domMeasuredNodeLookup].filter(([id]) => nodeLookup.has(id)),
				);
			patch.nodeLookup = nodeLookup;
			patch.parentLookup = parentLookup;
			patch.geometryRevision = state.geometryRevision + 1;
			patch.nodesInitialized = config.nodes.every(
				(node) =>
					node.hidden ||
					!!(
						nodeLookup.get(node.id)?.measured.width &&
						nodeLookup.get(node.id)?.measured.height
					),
			);
		}
		if (config.edges && config.edges !== state.edges && !config.edgeLookup)
			patch.edgeLookup = new Map(config.edges.map((edge) => [edge.id, edge]));
		store.setState(patch);
		if (state.renderer !== "egui" || (!config.nodes && !config.edges)) return;
		const nodes = state.nodes.filter((node) => node.selected);
		const edges = state.edges.filter((edge) => edge.selected);
		const changed =
			nodes.length !== selectedNodeIds.length ||
			edges.length !== selectedEdgeIds.length ||
			nodes.some((node, index) => node.id !== selectedNodeIds[index]) ||
			edges.some((edge, index) => edge.id !== selectedEdgeIds[index]);
		selectedNodeIds = nodes.map((node) => node.id);
		selectedEdgeIds = edges.map((edge) => edge.id);
		if (changed) state.onSelectionChange?.({ nodes, edges });
	};
	const triggerNodeChanges = (changes: NodeChange[]) => {
		if (!changes.length) return;
		configure({ nodes: applyNodeChanges(changes, state.nodes) });
		state.onNodesChange?.(changes);
	};
	const triggerEdgeChanges = (changes: EdgeChange[]) => {
		if (!changes.length) return;
		configure({ edges: applyEdgeChanges(changes, state.edges) });
		state.onEdgesChange?.(changes);
	};
	const viewport = (): Viewport => ({
		x: state.transform[0],
		y: state.transform[1],
		zoom: state.transform[2],
	});
	const cancelAnimation = () => {
		if (!animation) return;
		animation.cancel();
		animation.finish(false);
		animation = undefined;
	};
	const setViewport: BoardRuntime["setViewport"] = async (
		next,
		options = {},
	) => {
		if (legacy) {
			// React Flow's setViewport skips its own zoom limits, and below them it
			// mounts every node of a large board (restored native cameras go to 0.02).
			const { minZoom, maxZoom, width, height } = legacy.store.getState();
			return legacy.flow.setViewport(
				zoomAboutCenter(
					next,
					Math.max(minZoom, Math.min(maxZoom, next.zoom)),
					width,
					height,
				),
				options,
			);
		}
		if (![next.x, next.y, next.zoom].every(Number.isFinite) || next.zoom <= 0)
			return false;
		cancelAnimation();
		const target = {
			...next,
			zoom: Math.max(state.minZoom, Math.min(state.maxZoom, next.zoom)),
		};
		store.setState({ viewportInitialized: true });
		const from = viewport();
		if (from.x === target.x && from.y === target.y && from.zoom === target.zoom)
			return true;
		const notify = options.notify !== false;
		if (notify) state.onMoveStart?.(null, from);
		const apply = (value: Viewport) => {
			store.setState({ transform: [value.x, value.y, value.zoom] });
			if (notify) state.onMove?.(null, value);
		};
		if (!options.duration || typeof requestAnimationFrame === "undefined") {
			apply(target);
			if (notify) state.onMoveEnd?.(null, target);
			return true;
		}
		return new Promise<boolean>((resolve) => {
			const start = performance.now();
			const duration = options.duration ?? 0;
			const current = {
				cancel: () => {},
				finish(completed: boolean) {
					if (notify) state.onMoveEnd?.(null, viewport());
					resolve(completed);
				},
			};
			const tick = (now: number) => {
				if (animation !== current) return;
				const fraction = Math.min(1, (now - start) / duration);
				const progress =
					options.ease?.(fraction) ?? fraction * fraction * (3 - 2 * fraction);
				apply({
					x: from.x + (target.x - from.x) * progress,
					y: from.y + (target.y - from.y) * progress,
					zoom: from.zoom + (target.zoom - from.zoom) * progress,
				});
				// Between frames, so the canvases draw each step with the DOM.
				if (fraction < 1) current.cancel = betweenFrames(tick);
				else {
					animation = undefined;
					current.finish(true);
				}
			};
			animation = current;
			current.cancel = betweenFrames(tick);
		});
	};
	const updateNodeInternals: ReactFlowState["updateNodeInternals"] = (
		updates,
		options,
	) => {
		if (legacy)
			return legacy.store.getState().updateNodeInternals(updates, options);
		const lookup = new Map(state.nodeLookup);
		const measurements = new Map(state.domMeasuredNodeLookup);
		const changes: NodeChange[] = [];
		let dirty = false;
		for (const [id, { nodeElement }] of updates) {
			const node = lookup.get(id);
			if (!node || !nodeElement.isConnected) continue;
			const bounds = nodeElement.getBoundingClientRect();
			const zoom = state.transform[2];
			const measured = {
				width: nodeElement.offsetWidth || bounds.width / zoom,
				height: nodeElement.offsetHeight || bounds.height / zoom,
			};
			if (!measured.width || !measured.height) continue;
			const handles = (type: "source" | "target") =>
				Array.from(
					nodeElement.querySelectorAll<HTMLElement>(
						`.react-flow__handle.${type}`,
					),
				)
					.filter((element) => element.dataset.nodeid === id)
					.map((element) => {
						const rect = element.getBoundingClientRect();
						return {
							id: element.dataset.handleid ?? null,
							nodeId: id,
							type,
							position: (element.dataset.handlepos ??
								(type === "source"
									? Position.Right
									: Position.Left)) as Position,
							x: (rect.left - bounds.left) / zoom,
							y: (rect.top - bounds.top) / zoom,
							width: rect.width / zoom,
							height: rect.height / zoom,
						};
					});
			const handleBounds = {
				source: handles("source"),
				target: handles("target"),
			};
			const resized =
				measured.width !== node.measured.width ||
				measured.height !== node.measured.height;
			const previousMeasurement = measurements.get(id);
			if (
				!resized &&
				sameMeasuredHandles(handleBounds, node.internals.handleBounds) &&
				previousMeasurement &&
				(previousMeasurement.data.node ?? previousMeasurement.data.layer) ===
					(node.data.node ?? node.data.layer)
			)
				continue;
			dirty = true;
			const measuredNode: InternalNode = {
				...node,
				measured,
				internals: {
					...node.internals,
					handleBounds: sameMeasuredHandles(
						handleBounds,
						previousMeasurement?.internals.handleBounds,
					)
						? previousMeasurement?.internals.handleBounds
						: handleBounds,
				},
			};
			lookup.set(id, measuredNode);
			measurements.set(id, measuredNode);
			if (resized)
				changes.push({ id, type: "dimensions", dimensions: measured });
		}
		if (!dirty) return;
		store.setState({
			nodeLookup: lookup,
			domMeasuredNodeLookup: measurements,
			geometryRevision: state.geometryRevision + 1,
			measuredGeometryRevision: state.measuredGeometryRevision + 1,
		});
		if (changes.length) triggerNodeChanges(changes);
	};
	const setNodes: ReactFlowInstance["setNodes"] = (input) => {
		if (legacy) return legacy.flow.setNodes(input);
		const nodes = typeof input === "function" ? input(state.nodes) : input;
		const incoming = new Set(nodes.map((node) => node.id));
		const changes: NodeChange[] = state.nodes
			.filter((node) => !incoming.has(node.id))
			.map((node) => ({ id: node.id, type: "remove" }));
		for (const item of nodes) {
			const previous = state.nodeLookup.get(item.id)?.internals.userNode;
			if (previous === item) continue;
			changes.push(
				previous
					? { id: item.id, type: "replace", item }
					: { type: "add", item },
			);
		}
		configure({ nodes });
		if (changes.length) state.onNodesChange?.(changes);
	};
	const setEdges: ReactFlowInstance["setEdges"] = (input) => {
		if (legacy) return legacy.flow.setEdges(input);
		const edges = typeof input === "function" ? input(state.edges) : input;
		const incoming = new Set(edges.map((edge) => edge.id));
		const changes: EdgeChange[] = state.edges
			.filter((edge) => !incoming.has(edge.id))
			.map((edge) => ({ id: edge.id, type: "remove" }));
		for (const item of edges) {
			const previous = state.edgeLookup.get(item.id);
			if (previous === item) continue;
			changes.push(
				previous
					? { id: item.id, type: "replace", item }
					: { type: "add", item },
			);
		}
		configure({ edges });
		if (changes.length) state.onEdgesChange?.(changes);
	};
	const fitBounds: ReactFlowInstance["fitBounds"] = (bounds, options) => {
		if (legacy) return legacy.flow.fitBounds(bounds, options);
		if (!state.width || !state.height) return Promise.resolve(false);
		return setViewport(
			getViewportForBounds(
				bounds,
				state.width,
				state.height,
				state.minZoom,
				state.maxZoom,
				options?.padding ?? 0.1,
			),
			options,
		);
	};
	const flow: Pick<
		ReactFlowInstance,
		| "getNode"
		| "getNodes"
		| "getEdge"
		| "getEdges"
		| "getInternalNode"
		| "setNodes"
		| "setEdges"
		| "getViewport"
		| "getZoom"
		| "setViewport"
		| "fitBounds"
		| "getNodesBounds"
		| "fitView"
		| "screenToFlowPosition"
		| "flowToScreenPosition"
		| "zoomTo"
		| "zoomIn"
		| "zoomOut"
		| "setCenter"
		| "viewportInitialized"
		| "toObject"
	> = {
		getNode: (id: string) =>
			legacy
				? legacy.flow.getNode(id)
				: state.nodeLookup.get(id)?.internals.userNode,
		getNodes: () => legacy?.flow.getNodes() ?? state.nodes,
		getEdges: () => legacy?.flow.getEdges() ?? state.edges,
		getEdge: (id: string) =>
			legacy ? legacy.flow.getEdge(id) : state.edgeLookup.get(id),
		getInternalNode: (id: string) =>
			legacy ? legacy.flow.getInternalNode(id) : state.nodeLookup.get(id),
		setNodes,
		setEdges,
		getViewport: () => legacy?.flow.getViewport() ?? viewport(),
		getZoom: () => legacy?.flow.getZoom() ?? state.transform[2],
		setViewport,
		fitBounds,
		getNodesBounds: (nodes: (Node | InternalNode | string)[]) =>
			getNodesBounds(nodes, {
				nodeLookup: state.nodeLookup,
				nodeOrigin: state.nodeOrigin,
			}),
		fitView: async (options = {}) => {
			if (legacy) return legacy.flow.fitView(options);
			const ids = options.nodes
				? new Set(options.nodes.map((node) => node.id))
				: undefined;
			const nodes = state.nodes.filter(
				(node) =>
					(options.includeHiddenNodes || !node.hidden) &&
					(!ids || ids.has(node.id)),
			);
			if (!nodes.length || !state.width || !state.height) return false;
			const bounds = getNodesBounds(nodes, {
				nodeLookup: state.nodeLookup,
				nodeOrigin: state.nodeOrigin,
			});
			return setViewport(
				getViewportForBounds(
					bounds,
					state.width,
					state.height,
					options.minZoom ?? state.minZoom,
					options.maxZoom ?? state.maxZoom,
					options.padding ?? 0.1,
				),
				options,
			);
		},
		screenToFlowPosition: (point, options) => {
			if (legacy) return legacy.flow.screenToFlowPosition(point, options);
			const bounds = state.domNode?.getBoundingClientRect();
			const result = {
				x:
					(point.x - (bounds?.left ?? 0) - state.transform[0]) /
					state.transform[2],
				y:
					(point.y - (bounds?.top ?? 0) - state.transform[1]) /
					state.transform[2],
			};
			if (options?.snapToGrid ?? state.snapToGrid) {
				result.x = Math.round(result.x / state.snapGrid[0]) * state.snapGrid[0];
				result.y = Math.round(result.y / state.snapGrid[1]) * state.snapGrid[1];
			}
			return result;
		},
		flowToScreenPosition: (point) => {
			if (legacy) return legacy.flow.flowToScreenPosition(point);
			const bounds = state.domNode?.getBoundingClientRect();
			return {
				x:
					point.x * state.transform[2] +
					state.transform[0] +
					(bounds?.left ?? 0),
				y:
					point.y * state.transform[2] +
					state.transform[1] +
					(bounds?.top ?? 0),
			};
		},
		zoomTo: (zoom, options) => {
			if (legacy) return legacy.flow.zoomTo(zoom, options);
			return setViewport(
				zoomAboutCenter(
					viewport(),
					Math.max(state.minZoom, Math.min(state.maxZoom, zoom)),
					state.width,
					state.height,
				),
				options,
			);
		},
		zoomIn: (options) => flow.zoomTo(state.transform[2] * 1.2, options),
		zoomOut: (options) => flow.zoomTo(state.transform[2] / 1.2, options),
		setCenter: (x, y, options) => {
			const zoom = options?.zoom ?? state.transform[2];
			return setViewport(
				{ x: state.width / 2 - x * zoom, y: state.height / 2 - y * zoom, zoom },
				options,
			);
		},
		get viewportInitialized() {
			return state.viewportInitialized;
		},
		toObject: () => ({
			nodes: state.nodes,
			edges: state.edges,
			viewport: viewport(),
		}),
	};
	Object.assign(state, {
		setNodes: (nodes: Node[]) => configure({ nodes }),
		setEdges: (edges: Edge[]) => configure({ edges }),
		triggerNodeChanges,
		triggerEdgeChanges,
		updateNodeInternals,
		setMinZoom: (minZoom: number) => configure({ minZoom }),
		setMaxZoom: (maxZoom: number) => configure({ maxZoom }),
	});
	return {
		store,
		flow: flow as ReactFlowInstance,
		configure,
		setViewport,
		attachLegacy(instance, legacyStore) {
			cancelAnimation();
			const attached = { flow: instance, store: legacyStore };
			legacy = attached;
			const sync = (next: ReactFlowState, previous?: ReactFlowState) => {
				const geometryChanged =
					!previous ||
					next.transform === previous.transform ||
					next.nodes !== previous.nodes ||
					next.edges !== previous.edges;
				configure({
					renderer: "react-flow",
					nodes: next.nodes,
					edges: next.edges,
					// RF mutates these maps in place; publish fresh identities for keyed facade subscribers.
					nodeLookup: geometryChanged
						? new Map(next.nodeLookup)
						: state.nodeLookup,
					edgeLookup: geometryChanged
						? new Map(next.edgeLookup)
						: state.edgeLookup,
					parentLookup: next.parentLookup,
					transform: next.transform,
					width: next.width,
					height: next.height,
					domNode: next.domNode,
					viewportElement:
						next.domNode?.querySelector<HTMLElement>(".react-flow__viewport") ??
						null,
					overlayElement:
						next.domNode?.querySelector<HTMLElement>(".react-flow__renderer") ??
						next.domNode,
					viewportInitialized:
						state.viewportInitialized || instance.viewportInitialized,
					nodesDraggable: next.nodesDraggable,
					nodesConnectable: next.nodesConnectable,
					elementsSelectable: next.elementsSelectable,
					snapToGrid: next.snapToGrid,
					snapGrid: next.snapGrid,
					geometryRevision: state.geometryRevision + Number(geometryChanged),
				});
			};
			sync(legacyStore.getState());
			const unsubscribe = legacyStore.subscribe(sync);
			return () => {
				unsubscribe();
				if (legacy !== attached) return;
				legacy = undefined;
				configure({
					renderer: "egui",
					nodes: [...state.nodes],
					edges: [...state.edges],
					domNode: null,
				});
			};
		},
		destroy() {
			cancelAnimation();
			listeners.clear();
			keyed.clear();
		},
	};
}

const RuntimeContext = createContext<{
	runtime: BoardRuntime;
	mode?: "egui" | "react-flow";
} | null>(null);

export function BoardRuntimeProvider({ children }: { children: ReactNode }) {
	const [runtime] = useState(createBoardRuntime);
	const value = useMemo(() => ({ runtime }), [runtime]);
	return (
		<RuntimeContext.Provider value={value}>{children}</RuntimeContext.Provider>
	);
}

export function useBoardRuntime(): BoardRuntime | undefined {
	return useContext(RuntimeContext)?.runtime;
}

export function LegacyBoardRuntimeScope({ children }: { children: ReactNode }) {
	return (
		<RuntimeContext.Provider value={null}>{children}</RuntimeContext.Provider>
	);
}

export function LegacyBoardRuntimeBridge({
	children,
}: { children: ReactNode }) {
	const context = useContext(RuntimeContext);
	const runtime = context?.runtime;
	const legacyFlow = useLegacyReactFlow();
	const legacyStore = useLegacyStoreApi();
	useLayoutEffect(
		() => runtime?.attachLegacy(legacyFlow, legacyStore),
		[runtime, legacyFlow, legacyStore],
	);
	const value = useMemo(
		() => (runtime ? { runtime, mode: "react-flow" as const } : null),
		[runtime],
	);
	return (
		<RuntimeContext.Provider value={value}>{children}</RuntimeContext.Provider>
	);
}

function useRuntimeSelector<T>(
	runtime: BoardRuntime,
	selector: (state: BoardRuntimeState) => T,
	equality: (a: T, b: T) => boolean = Object.is,
): T {
	const latest = useRef({ selector, equality });
	latest.current = { selector, equality };
	const cache = useRef<
		| {
				value: T;
				state: BoardRuntimeState;
				selector: typeof selector;
				keys: Set<StateKey>;
		  }
		| undefined
	>(undefined);
	const snapshot = useCallback(() => {
		const state = runtime.store.getState();
		if (
			cache.current?.state === state &&
			cache.current.selector === latest.current.selector
		)
			return cache.current.value;
		const keys = new Set<StateKey>();
		const tracked = new Proxy(state, {
			get(target, property, receiver) {
				if (typeof property === "string") keys.add(property as StateKey);
				return Reflect.get(target, property, receiver);
			},
		});
		const value = latest.current.selector(tracked);
		const previous = cache.current;
		cache.current = {
			value:
				previous && latest.current.equality(previous.value, value)
					? previous.value
					: value,
			state,
			selector: latest.current.selector,
			keys,
		};
		return cache.current.value;
	}, [runtime]);
	const subscribe = useCallback(
		(listener: () => void) =>
			runtime.store.subscribeKeys(
				listener,
				() => cache.current?.keys ?? new Set(),
			),
		[runtime],
	);
	return useSyncExternalStore(subscribe, snapshot, snapshot);
}

// The provider boundary is stable for each mounted consumer. Native consumers never call RF hooks.
export function useReactFlow(): ReactFlowInstance {
	const runtime = useBoardRuntime();
	return runtime ? runtime.flow : useLegacyReactFlow();
}
export function useStoreApi() {
	const runtime = useBoardRuntime();
	return runtime ? runtime.store : useLegacyStoreApi();
}
export function useStore<T>(
	selector: (state: BoardRuntimeState) => T,
	equality?: (a: T, b: T) => boolean,
): T {
	const runtime = useBoardRuntime();
	return runtime
		? useRuntimeSelector(runtime, selector, equality)
		: useLegacyStore(selector as (state: ReactFlowState) => T, equality);
}
export function useNodes(): Node[] {
	const runtime = useBoardRuntime();
	return runtime
		? useRuntimeSelector(runtime, (state) => state.nodes)
		: useLegacyNodes();
}
export function useViewport(): Viewport {
	const runtime = useBoardRuntime();
	if (!runtime) return useLegacyViewport();
	const transform = useRuntimeSelector(runtime, (state) => state.transform);
	return useMemo(
		() => ({ x: transform[0], y: transform[1], zoom: transform[2] }),
		[transform],
	);
}
export function useInternalNode(id: string): InternalNode | undefined {
	const runtime = useBoardRuntime();
	return runtime
		? useRuntimeSelector(runtime, (state) => state.nodeLookup.get(id))
		: useLegacyInternalNode(id);
}
export function useBoardRuntimeMode() {
	const context = useContext(RuntimeContext);
	// Only the bridge subtree has an RF provider. Shared outer overlays use runtime portals.
	return context ? (context.mode ?? "egui") : undefined;
}
