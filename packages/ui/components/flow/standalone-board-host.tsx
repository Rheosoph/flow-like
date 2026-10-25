"use client";

import {
	type Edge,
	type EdgeProps,
	EdgeText,
	type EdgeTypes,
	type Node,
	type NodeProps,
	Position,
	type ReactFlowProps,
	getBezierPath,
	getSimpleBezierPath,
	getSmoothStepPath,
	getStraightPath,
} from "@xyflow/react";
import {
	type ComponentType,
	type Ref,
	forwardRef,
	memo,
	useCallback,
	useEffect,
	useId,
	useLayoutEffect,
	useMemo,
	useRef,
	useSyncExternalStore,
} from "react";
import {
	equalVisibleIds,
	visibleDomIds,
	warmEditorIds,
} from "../../lib/board-canvas/dom-visibility";
import { useBoardRuntime } from "./board-runtime";
import { BoardNodeIdProvider } from "./board-runtime-primitives";

export type StandaloneBoardHostProps = ReactFlowProps & {
	exporting?: boolean;
};

type RegisterTransform = (
	element: HTMLElement | SVGSVGElement,
	position?: { x: number; y: number },
	promote?: boolean,
) => () => void;

/** Native canvas planes that the rich content interleaves with. */
export const NATIVE_EDGE_PLANE_Z = 18;
export const NATIVE_NODE_PLANE_Z = 20;
const CAMERA_SETTLE_MS = 150;

/**
 * WebKit rasterizes a composited layer at its untransformed size and never re-rasters it
 * for a CSS scale, so a zoomed editor on its own layer blurs. Anything painted above the
 * WebGL edge plane is composited for overlap, so editors between the planes share one
 * untransformed stratum that paints them at device resolution. They get layers of their
 * own only while the camera moves.
 */
const inEditorStratum = (node: Node) =>
	(node.zIndex ?? 0) > NATIVE_EDGE_PLANE_Z &&
	(node.zIndex ?? 0) <= NATIVE_NODE_PLANE_Z;

const cameraTransform = ([x, y, zoom]: readonly number[]) =>
	`translate(${x}px, ${y}px) scale(${zoom})`;

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
	if (typeof ref === "function") ref(value);
	else if (ref) ref.current = value;
}

function useInternalNode(id: string) {
	const runtime = useBoardRuntime();
	if (!runtime) throw new Error("The standalone board needs a board runtime.");
	return useSyncExternalStore(
		runtime.store.subscribe,
		() => runtime.store.getState().nodeLookup.get(id),
		() => runtime.store.getState().nodeLookup.get(id),
	);
}

const RichNode = memo(function RichNode({
	node,
	component: Original,
	observe,
	measure,
	registerTransform,
	props,
}: {
	node: Node;
	component: ComponentType<NodeProps>;
	observe: (element: HTMLDivElement | null, id: string) => void;
	measure: (id: string, element: HTMLDivElement) => void;
	registerTransform: RegisterTransform;
	props: StandaloneBoardHostProps;
}) {
	const runtime = useBoardRuntime();
	const internal = useInternalNode(node.id);
	const ref = useRef<HTMLDivElement | null>(null);
	const position = internal?.internals.positionAbsolute ?? node.position;
	const promote = inEditorStratum(node);
	useLayoutEffect(() => {
		if (ref.current)
			return registerTransform(
				ref.current,
				{ x: position.x, y: position.y },
				promote,
			);
	}, [registerTransform, position.x, position.y, promote]);
	const width = internal?.measured.width ?? node.measured?.width ?? node.width;
	const height =
		internal?.measured.height ?? node.measured?.height ?? node.height;
	const zIndex =
		internal?.internals.z ?? (node.zIndex ?? 0) + (node.selected ? 1000 : 0);
	const draggable = props.nodesDraggable !== false && node.draggable !== false;
	const selectable =
		props.elementsSelectable !== false && node.selectable !== false;
	const connectable =
		props.nodesConnectable !== false && node.connectable !== false;
	const setElement = useCallback(
		(element: HTMLDivElement | null) => {
			ref.current = element;
			observe(element, node.id);
		},
		[observe, node.id],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Same-size pin reorderings still invalidate the record-bound port measurements.
	useLayoutEffect(() => {
		if (ref.current) measure(node.id, ref.current);
	}, [measure, node.id, node.data.node, node.data.layer]);
	useEffect(() => {
		if (node.data.__flowNativeHovered)
			ref.current?.firstElementChild?.dispatchEvent(
				new MouseEvent("mouseover", { bubbles: true }),
			);
	}, [node.data.__flowNativeHovered]);
	const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
		if (event.target !== event.currentTarget || !selectable || !runtime) return;
		if (!["Enter", " ", "Escape"].includes(event.key)) return;
		event.preventDefault();
		const selected = event.key !== "Escape";
		const additive = event.metaKey || event.ctrlKey;
		props.onNodesChange?.(
			runtime.flow.getNodes().flatMap((entry) => {
				const next =
					entry.id === node.id ? selected : additive && !!entry.selected;
				return !!entry.selected === next
					? []
					: [{ id: entry.id, type: "select" as const, selected: next }];
			}),
		);
	};
	return (
		<div
			ref={setElement}
			className={`react-flow__node react-flow__node-${node.type ?? "default"} ${node.selected ? "selected" : ""} ${selectable ? "selectable" : ""} ${draggable ? "draggable nopan" : ""} ${node.dragging ? "dragging" : ""} ${node.className ?? ""}`}
			data-id={node.id}
			data-testid={`rf__node-${node.id}`}
			data-board-rich-node
			style={{
				...node.style,
				position: "absolute",
				left: 0,
				top: 0,
				transformOrigin: "0 0",
				zIndex,
				pointerEvents: "all",
				...(node.width !== undefined ? { width: node.width } : {}),
				...(node.height !== undefined ? { height: node.height } : {}),
			}}
			tabIndex={
				node.focusable === false || props.nodesFocusable === false
					? undefined
					: 0
			}
			role={node.ariaRole ?? "group"}
			aria-roledescription="node"
			aria-label={node.ariaLabel}
			onKeyDown={onKeyDown}
			onMouseEnter={(event) => props.onNodeMouseEnter?.(event, node)}
			onMouseLeave={(event) => props.onNodeMouseLeave?.(event, node)}
			onFocus={(event) => {
				if (!runtime || !event.currentTarget.matches(":focus-visible")) return;
				const state = runtime.store.getState();
				const [x, y, zoom] = state.transform;
				if (
					position.x * zoom + x + (width ?? 0) * zoom < 0 ||
					position.y * zoom + y + (height ?? 0) * zoom < 0 ||
					position.x * zoom + x > state.width ||
					position.y * zoom + y > state.height
				)
					void runtime.flow.setCenter(
						position.x + (width ?? 0) / 2,
						position.y + (height ?? 0) / 2,
						{ zoom },
					);
			}}
			{...node.domAttributes}
		>
			<BoardNodeIdProvider nodeId={node.id}>
				<Original
					id={node.id}
					data={node.data}
					type={node.type ?? "default"}
					positionAbsoluteX={position.x}
					positionAbsoluteY={position.y}
					selected={node.selected ?? false}
					selectable={selectable}
					draggable={draggable}
					deletable={node.deletable ?? true}
					isConnectable={connectable}
					sourcePosition={node.sourcePosition}
					targetPosition={node.targetPosition}
					dragging={node.dragging ?? false}
					dragHandle={node.dragHandle}
					zIndex={zIndex}
					parentId={node.parentId}
					width={width}
					height={height}
				/>
			</BoardNodeIdProvider>
		</div>
	);
});

function DefaultEdge(props: EdgeProps) {
	const [path, labelX, labelY] =
		props.type === "straight"
			? getStraightPath(props)
			: props.type === "simplebezier"
				? getSimpleBezierPath(props)
				: props.type === "step" || props.type === "smoothstep"
					? getSmoothStepPath({
							...props,
							...props.pathOptions,
							borderRadius:
								props.type === "step"
									? 0
									: (props.pathOptions?.borderRadius ?? 5),
						})
					: getBezierPath({ ...props, ...props.pathOptions });
	return (
		<>
			<path
				className="react-flow__edge-path"
				d={path}
				fill="none"
				stroke="var(--foreground)"
				markerStart={props.markerStart}
				markerEnd={props.markerEnd}
				style={props.style}
			/>
			<path
				className="react-flow__edge-interaction"
				d={path}
				fill="none"
				stroke="transparent"
				strokeWidth={props.interactionWidth ?? 20}
			/>
			<EdgeText
				x={labelX}
				y={labelY}
				label={props.label}
				labelStyle={props.labelStyle}
				labelShowBg={props.labelShowBg}
				labelBgStyle={props.labelBgStyle}
				labelBgPadding={props.labelBgPadding}
				labelBgBorderRadius={props.labelBgBorderRadius}
			/>
		</>
	);
}

const RichEdge = memo(function RichEdge({
	edge,
	component: Original = DefaultEdge,
	prefix,
	props,
	registerTransform,
}: {
	edge: Edge;
	component?: EdgeTypes[string];
	prefix: string;
	props: StandaloneBoardHostProps;
	registerTransform: RegisterTransform;
}) {
	const runtime = useBoardRuntime();
	const source = useInternalNode(edge.source);
	const target = useInternalNode(edge.target);
	const releaseTransform = useRef<(() => void) | undefined>(undefined);
	const ref = useCallback(
		(element: SVGSVGElement | null) => {
			releaseTransform.current?.();
			releaseTransform.current = element
				? registerTransform(element)
				: undefined;
		},
		[registerTransform],
	);
	if (!source || !target) return null;
	const locate = (
		node: typeof source,
		id: string | null | undefined,
		type: "source" | "target",
	) => {
		const handle = node.internals.handleBounds?.[type]?.find(
			(item) => !id || item.id === id,
		);
		if (!handle) return undefined;
		return {
			x: node.internals.positionAbsolute.x + handle.x + handle.width / 2,
			y: node.internals.positionAbsolute.y + handle.y + handle.height / 2,
			position: handle.position,
		};
	};
	const a = locate(source, edge.sourceHandle, "source");
	const b = locate(target, edge.targetHandle, "target");
	if (!a || !b) return null;
	const marker = (value: Edge["markerStart"], suffix: string) =>
		typeof value === "string"
			? `url('#${value}')`
			: value
				? `url('#${prefix}-${suffix}')`
				: undefined;
	const zIndex = (edge.zIndex ?? 0) + (edge.selected ? 1000 : 0);
	const focusable = edge.focusable ?? props.edgesFocusable ?? true;
	const onKeyDown = (event: React.KeyboardEvent<SVGGElement>) => {
		if (
			!runtime ||
			event.target !== event.currentTarget ||
			!focusable ||
			props.disableKeyboardA11y ||
			!["Enter", " ", "Escape"].includes(event.key)
		)
			return;
		const state = runtime.store.getState();
		if (!(edge.selectable ?? state.elementsSelectable)) return;
		event.preventDefault();
		event.stopPropagation();
		const selected = event.key !== "Escape";
		const additive = event.metaKey || event.ctrlKey || event.shiftKey;
		state.triggerEdgeChanges(
			state.edges.flatMap((entry) => {
				const next =
					entry.id === edge.id
						? selected
						: selected && !additive
							? false
							: !!entry.selected;
				return !!entry.selected === next
					? []
					: [{ id: entry.id, type: "select" as const, selected: next }];
			}),
		);
		if (selected && !additive)
			state.triggerNodeChanges(
				state.nodes
					.filter((node) => node.selected)
					.map((node) => ({ id: node.id, type: "select", selected: false })),
			);
		if (!selected) event.currentTarget.blur();
	};
	return (
		<svg
			ref={ref}
			style={{
				position: "absolute",
				inset: 0,
				width: "100%",
				height: "100%",
				overflow: "visible",
				pointerEvents: "none",
				transformOrigin: "0 0",
				zIndex,
			}}
		>
			<title>
				{edge.ariaLabel ?? `Connection from ${edge.source} to ${edge.target}`}
			</title>
			<defs>
				{(["start", "end"] as const).map((side) => {
					const value = side === "start" ? edge.markerStart : edge.markerEnd;
					return value && typeof value !== "string" ? (
						<marker
							key={side}
							id={`${prefix}-${side}`}
							markerWidth={value.width ?? 12}
							markerHeight={value.height ?? 12}
							viewBox="-10 -10 20 20"
							refX="0"
							refY="0"
							orient={value.orient ?? "auto-start-reverse"}
							markerUnits={value.markerUnits ?? "strokeWidth"}
						>
							<polyline
								points="-5,-4 0,0 -5,4"
								fill={
									value.type === "arrowclosed"
										? (value.color ?? "var(--foreground)")
										: "none"
								}
								stroke={value.color ?? "var(--foreground)"}
								strokeWidth={value.strokeWidth ?? 1}
							/>
						</marker>
					) : null;
				})}
			</defs>
			<g
				className={`react-flow__edge react-flow__edge-${edge.type ?? "default"} nopan ${edge.selected ? "selected" : ""} ${edge.animated ? "animated" : ""} ${edge.className ?? ""}`}
				data-id={edge.id}
				data-testid={`rf__edge-${edge.id}`}
				style={{ pointerEvents: "visibleStroke" }}
				tabIndex={focusable ? 0 : undefined}
				role={edge.ariaRole ?? "group"}
				aria-roledescription="edge"
				aria-label={
					edge.ariaLabel ?? `Connection from ${edge.source} to ${edge.target}`
				}
				onKeyDown={onKeyDown}
				{...edge.domAttributes}
			>
				<Original
					{...edge}
					type={edge.type ?? "default"}
					data={edge.data}
					sourceX={a.x}
					sourceY={a.y}
					targetX={b.x}
					targetY={b.y}
					sourcePosition={a.position ?? Position.Right}
					targetPosition={b.position ?? Position.Left}
					sourceHandleId={edge.sourceHandle}
					targetHandleId={edge.targetHandle}
					markerStart={marker(edge.markerStart, "start")}
					markerEnd={marker(edge.markerEnd, "end")}
				/>
			</g>
		</svg>
	);
});

function DefaultNode({ data }: NodeProps) {
	return <div>{String(data.label ?? "")}</div>;
}

const noNodes: Node[] = [];

const RichContent = memo(function RichContent({
	nodes,
	edges,
	props,
	observe,
	measure,
	prefix,
	registerTransform,
}: {
	nodes: Node[];
	edges: Edge[];
	props: StandaloneBoardHostProps;
	observe: (element: HTMLDivElement | null, id: string) => void;
	measure: (id: string, element: HTMLDivElement) => void;
	prefix: string;
	registerTransform: RegisterTransform;
}) {
	const runtime = useBoardRuntime();
	if (!runtime) throw new Error("The standalone board needs a board runtime.");
	const mountedIds = useRef<{ nodes: string[]; edges: string[] }>({
		nodes: [],
		edges: [],
	});
	const visibility = useMemo(() => {
		const warm = warmEditorIds(nodes);
		let cached = {
			nodes: nodes.map((node) => node.id),
			edges: edges.map((edge) => edge.id),
		};
		let previousState: ReturnType<typeof runtime.store.getState> | undefined;
		return () => {
			const state = runtime.store.getState();
			if (
				state === previousState ||
				props.exporting ||
				props.onlyRenderVisibleElements === false
			)
				return cached;
			previousState = state;
			const retained = new Set(warm);
			if (typeof document !== "undefined") {
				const focus = document.activeElement?.closest<HTMLElement>(
					".react-flow__node,.react-flow__edge,[data-board-node-id]",
				);
				const id = focus?.dataset.boardNodeId ?? focus?.dataset.id;
				if (id) retained.add(id);
				// Portaled editors must retain their owning component even when the
				// camera moves. Their owner can be outside the current viewport.
				if (
					document.querySelector(
						'[role="dialog"][data-state="open"], [role="menu"][data-state="open"], [data-radix-popper-content-wrapper]',
					)
				)
					for (const id of mountedIds.current.nodes) retained.add(id);
			}
			const next = visibleDomIds({
				nodes,
				edges,
				nodeLookup: state.nodeLookup,
				transform: state.transform,
				width: state.width,
				height: state.height,
				retained,
			});
			if (!equalVisibleIds(cached, next)) cached = next;
			return cached;
		};
	}, [runtime, nodes, edges, props.exporting, props.onlyRenderVisibleElements]);
	const visible = useSyncExternalStore(
		runtime.store.subscribe,
		visibility,
		visibility,
	);
	mountedIds.current = visible;
	const nodeIds = new Set(visible.nodes);
	const edgeIds = new Set(visible.edges);
	const visibleNodes = nodes.filter((node) => nodeIds.has(node.id));
	const richNode = (node: Node) => (
		<RichNode
			key={node.id}
			node={node}
			component={props.nodeTypes?.[node.type ?? "default"] ?? DefaultNode}
			observe={observe}
			measure={measure}
			registerTransform={registerTransform}
			props={props}
		/>
	);
	return (
		<>
			{edges
				.filter((edge) => edgeIds.has(edge.id))
				.map((edge) => (
					<RichEdge
						key={edge.id}
						edge={edge}
						component={props.edgeTypes?.[edge.type ?? "default"]}
						prefix={`${prefix}-${edge.id}`}
						props={props}
						registerTransform={registerTransform}
					/>
				))}
			{visibleNodes.filter((node) => !inEditorStratum(node)).map(richNode)}
			<div
				data-board-editor-stratum
				style={{
					position: "absolute",
					inset: 0,
					pointerEvents: "none",
					zIndex: NATIVE_EDGE_PLANE_Z + 1,
				}}
			>
				{visibleNodes.filter(inEditorStratum).map(richNode)}
				<SelectedGroup registerTransform={registerTransform} />
			</div>
		</>
	);
});

function SelectedGroup({
	registerTransform,
}: { registerTransform: RegisterTransform }) {
	const runtime = useBoardRuntime();
	if (!runtime) return null;
	const nodes = useSyncExternalStore(
		runtime.store.subscribe,
		() =>
			runtime.store.getState().nodesSelectionActive
				? runtime.store.getState().nodes
				: noNodes,
		() => noNodes,
	);
	useSyncExternalStore(
		runtime.store.subscribe,
		() => runtime.store.getState().geometryRevision,
		() => 0,
	);
	const selected = nodes.filter((node) => node.selected && !node.hidden);
	const ref = useRef<HTMLDivElement | null>(null);
	const visible = selected.length > 1;
	useLayoutEffect(() => {
		if (visible && ref.current) return registerTransform(ref.current);
	}, [registerTransform, visible]);
	if (selected.length < 2) return null;
	const bounds = runtime.flow.getNodesBounds(selected);
	return (
		<div
			ref={ref}
			className="react-flow__nodesselection"
			style={{
				position: "absolute",
				inset: 0,
				pointerEvents: "none",
				zIndex: 1000,
				transformOrigin: "0 0",
			}}
		>
			<div
				className="react-flow__nodesselection-rect"
				// biome-ignore lint/a11y/noNoninteractiveTabindex: This group supports keyboard movement through the board controller.
				tabIndex={0}
				role="group"
				aria-label={`${selected.length} selected nodes`}
				style={{
					position: "absolute",
					width: bounds.width,
					height: bounds.height,
					transform: `translate(${bounds.x}px, ${bounds.y}px)`,
					transformOrigin: "0 0",
					pointerEvents: "all",
					border: "1px dotted var(--primary)",
					background: "color-mix(in oklch, var(--primary) 5%, transparent)",
				}}
			/>
		</div>
	);
}

/** Independent camera and rich-content host. The native painter owns ordinary graph pixels. */
export const StandaloneBoardHost = forwardRef<
	HTMLDivElement,
	StandaloneBoardHostProps
>(function StandaloneBoardHost(props, forwardedRef) {
	const runtime = useBoardRuntime();
	if (!runtime) throw new Error("The standalone board needs a board runtime.");
	const root = useRef<HTMLDivElement | null>(null);
	const viewport = useRef<HTMLDivElement | null>(null);
	const worldPortal = useRef<HTMLDivElement | null>(null);
	const overlay = useRef<HTMLDivElement | null>(null);
	const latest = useRef(props);
	latest.current = props;
	const transformTargets = useRef(
		new Map<
			HTMLElement | SVGSVGElement,
			{ position?: { x: number; y: number }; promote: boolean }
		>(),
	);
	const cameraMoving = useRef(false);
	const registerTransform = useCallback<RegisterTransform>(
		(element, position, promote = false) => {
			transformTargets.current.set(element, { position, promote });
			const camera = cameraTransform(
				latest.current.exporting
					? [0, 0, 1]
					: runtime.store.getState().transform,
			);
			element.style.transform = position
				? `${camera} translate(${position.x}px, ${position.y}px)`
				: camera;
			element.style.willChange =
				promote && cameraMoving.current ? "transform" : "";
			return () => {
				transformTargets.current.delete(element);
			};
		},
		[runtime],
	);
	const prefix = useId().replaceAll(":", "");
	const elements = useRef(new Map<string, HTMLDivElement>());
	const observer = useRef<ResizeObserver | undefined>(undefined);
	const pendingMeasurements = useRef(
		new Map<
			string,
			{ id: string; nodeElement: HTMLDivElement; force: boolean }
		>(),
	);
	const measureFrame = useRef(0);
	const queueMeasure = useCallback(
		(id: string, element: HTMLDivElement) => {
			pendingMeasurements.current.set(id, {
				id,
				nodeElement: element,
				force: true,
			});
			if (measureFrame.current) return;
			measureFrame.current = requestAnimationFrame(() => {
				measureFrame.current = 0;
				const updates = new Map(
					[...pendingMeasurements.current].filter(
						([, entry]) => entry.nodeElement.isConnected,
					),
				);
				pendingMeasurements.current.clear();
				if (updates.size)
					runtime.store
						.getState()
						.updateNodeInternals(updates, { triggerFitView: false });
			});
		},
		[runtime],
	);
	const observe = useCallback(
		(element: HTMLDivElement | null, id: string) => {
			const previous = elements.current.get(id);
			if (previous) observer.current?.unobserve(previous);
			if (element) {
				elements.current.set(id, element);
				observer.current?.observe(element);
				queueMeasure(id, element);
			} else elements.current.delete(id);
		},
		[queueMeasure],
	);
	useLayoutEffect(() => {
		runtime.configure({
			nodes: props.nodes,
			edges: props.edges,
			onNodesChange: props.onNodesChange,
			onEdgesChange: props.onEdgesChange,
			onSelectionChange: props.onSelectionChange,
			onNodeDragStart: props.onNodeDragStart,
			onNodeDrag: props.onNodeDrag,
			onNodeDragStop: props.onNodeDragStop,
			onMoveStart: props.onMoveStart,
			onMove: props.onMove,
			onMoveEnd: props.onMoveEnd,
			minZoom: props.minZoom ?? 0.02,
			maxZoom: props.maxZoom ?? 3,
			renderer: "egui",
			domNode: root.current,
			viewportElement: worldPortal.current,
			overlayElement: overlay.current,
		});
	}, [runtime, props]);
	useLayoutEffect(() => {
		runtime.configure({
			nodesDraggable: props.nodesDraggable ?? true,
			nodesConnectable: props.nodesConnectable ?? true,
			elementsSelectable: props.elementsSelectable ?? true,
		});
	}, [
		runtime,
		props.nodesDraggable,
		props.nodesConnectable,
		props.elementsSelectable,
	]);
	useLayoutEffect(() => {
		const element = root.current;
		if (!element) return;
		const measure = () =>
			runtime.configure({
				width: element.clientWidth,
				height: element.clientHeight,
			});
		const sizeObserver = new ResizeObserver(measure);
		sizeObserver.observe(element);
		observer.current = new ResizeObserver((entries) => {
			for (const entry of entries) {
				const target = entry.target as HTMLDivElement;
				if (target.dataset.id) queueMeasure(target.dataset.id, target);
			}
		});
		for (const [id, nodeElement] of elements.current) {
			observer.current.observe(nodeElement);
			queueMeasure(id, nodeElement);
		}
		measure();
		const shouldFit =
			latest.current.fitView && !runtime.store.getState().viewportInitialized;
		latest.current.onInit?.(runtime.flow);
		let fitFrame = 0;
		if (shouldFit)
			fitFrame = requestAnimationFrame(() => {
				fitFrame = requestAnimationFrame(() => {
					if (!runtime.store.getState().viewportInitialized)
						void runtime.flow.fitView(latest.current.fitViewOptions);
				});
			});
		return () => {
			sizeObserver.disconnect();
			observer.current?.disconnect();
			cancelAnimationFrame(fitFrame);
			cancelAnimationFrame(measureFrame.current);
		};
	}, [runtime, queueMeasure]);
	useLayoutEffect(() => {
		let settle: ReturnType<typeof setTimeout> | undefined;
		const setMoving = (moving: boolean) => {
			cameraMoving.current = moving;
			for (const [element, target] of transformTargets.current)
				if (target.promote)
					element.style.willChange = moving ? "transform" : "";
		};
		const sync = () => {
			const camera = cameraTransform(
				props.exporting ? [0, 0, 1] : runtime.store.getState().transform,
			);
			// Inherited CSS variables invalidate every rich editor's descendants on
			// camera changes. Direct transforms keep their computed styles intact.
			for (const [element, { position }] of transformTargets.current)
				element.style.transform = position
					? `${camera} translate(${position.x}px, ${position.y}px)`
					: camera;
			if (worldPortal.current) worldPortal.current.style.transform = camera;
		};
		sync();
		const unsubscribe = runtime.store.subscribe((state, previous) => {
			if (state.transform === previous.transform) return;
			sync();
			if (props.exporting) return;
			if (!cameraMoving.current) setMoving(true);
			clearTimeout(settle);
			settle = setTimeout(() => setMoving(false), CAMERA_SETTLE_MS);
		});
		return () => {
			unsubscribe();
			clearTimeout(settle);
			setMoving(false);
		};
	}, [runtime, props.exporting]);
	useEffect(() => {
		const element = root.current;
		if (!element) return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const wheel = (event: WheelEvent) => {
			if (
				latest.current.exporting ||
				(event.target instanceof Element &&
					event.target.closest(
						".nowheel,[data-board-dom-control],[data-board-canvas-controls]",
					))
			)
				return;
			event.preventDefault();
			const state = runtime.store.getState();
			const current = runtime.flow.getViewport();
			if (!timer) latest.current.onMoveStart?.(event, current);
			clearTimeout(timer);
			const multiplier =
				event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? state.height : 1;
			const zooming = event.ctrlKey || !latest.current.panOnScroll;
			if (zooming) {
				const zoom = Math.max(
					latest.current.minZoom ?? 0.02,
					Math.min(
						latest.current.maxZoom ?? 3,
						current.zoom *
							2 **
								(-event.deltaY *
									(event.deltaMode === 1 ? 0.05 : event.deltaMode ? 1 : 0.002) *
									(event.ctrlKey ? 10 : 1)),
					),
				);
				const bounds = element.getBoundingClientRect();
				const x = event.clientX - bounds.left;
				const y = event.clientY - bounds.top;
				void runtime.setViewport(
					{
						x: x - ((x - current.x) * zoom) / current.zoom,
						y: y - ((y - current.y) * zoom) / current.zoom,
						zoom,
					},
					{ notify: false },
				);
			} else
				void runtime.setViewport(
					{
						...current,
						x: current.x - event.deltaX * multiplier,
						y: current.y - event.deltaY * multiplier,
					},
					{ notify: false },
				);
			latest.current.onMove?.(event, runtime.flow.getViewport());
			timer = setTimeout(() => {
				timer = undefined;
				latest.current.onMoveEnd?.(event, runtime.flow.getViewport());
			}, 120);
		};
		element.addEventListener("wheel", wheel, { passive: false });
		return () => {
			clearTimeout(timer);
			element.removeEventListener("wheel", wheel);
		};
	}, [runtime]);
	const nodes = useMemo(
		() =>
			(props.nodes ?? []).filter(
				(node) =>
					!node.hidden &&
					(props.exporting || node.data.__flowNativeDom !== false),
			),
		[props.nodes, props.exporting],
	);
	const edges = useMemo(
		() =>
			(props.edges ?? []).filter(
				(edge) =>
					!edge.hidden &&
					(props.exporting || edge.data?.__flowNativeDom !== false),
			),
		[props.edges, props.exporting],
	);
	return (
		<div
			ref={(element) => {
				root.current = element;
				assignRef(forwardedRef, element);
			}}
			className={`react-flow ${props.colorMode === "dark" ? "dark" : "light"} ${props.className ?? ""}`}
			data-standalone-board
			data-standalone-board-host
			style={{
				position: "relative",
				width: "100%",
				height: "100%",
				overflow: "hidden",
				touchAction: "none",
				isolation: "isolate",
				...props.style,
			}}
			onContextMenu={props.onContextMenu}
		>
			<div
				className="react-flow__pane"
				style={{ position: "absolute", inset: 0, zIndex: -2147483648 }}
			/>
			<div
				ref={viewport}
				className="react-flow__viewport"
				data-board-world-content
				style={{
					position: "absolute",
					inset: 0,
					pointerEvents: "none",
					transform: "none",
					zIndex: "auto",
				}}
			>
				<RichContent
					nodes={nodes}
					edges={edges}
					props={props}
					observe={observe}
					measure={queueMeasure}
					prefix={prefix}
					registerTransform={registerTransform}
				/>
				<div
					ref={worldPortal}
					className="board-world-overlays"
					style={{
						position: "absolute",
						inset: 0,
						pointerEvents: "none",
						transformOrigin: "0 0",
					}}
				/>
			</div>
			<div
				ref={overlay}
				className="board-screen-overlays"
				data-board-screen-overlays
				style={{
					position: "absolute",
					inset: 0,
					pointerEvents: "none",
					zIndex: "auto",
				}}
			/>
			{props.children}
		</div>
	);
});
