import { type Edge, type Node, Position } from "@xyflow/react";
import { DOM_RENDERER_NODE_BUDGET } from "../../lib/board-canvas/dom-visibility";
import {
	type Palette,
	type Scene,
	type SceneNode,
	type ScenePin,
	isNativeCanvasNode,
	sceneFromFlow,
} from "../../lib/board-canvas/scene";

const GEOMETRY_ONLY: Palette = {
	background: [0, 0, 0, 0],
	card: [0, 0, 0, 0],
	foreground: [0, 0, 0, 0],
	muted: [0, 0, 0, 0],
	border: [0, 0, 0, 0],
	primary: [0, 0, 0, 0],
	tertiary: [0, 0, 0, 0],
};
// DOM editors measured up to a third taller than their native rows; overestimating only mounts a node early.
const DOM_HEIGHT_HEADROOM = 1.4;

function hasMeasuredSize(node: Node) {
	return (
		node.measured?.width !== undefined && node.measured.height !== undefined
	);
}

function sceneHandles(pins: readonly ScenePin[]) {
	return pins.map((pin) => ({
		id: pin.id,
		type: pin.direction === "input" ? ("target" as const) : ("source" as const),
		position: pinPosition(pin),
		x: pin.x - 6,
		y: pin.y - 6,
		width: 12,
		height: 12,
	}));
}

/**
 * React Flow mounts every node without handle bounds to measure it, wherever it is,
 * so opening a large board mounted all of it once and OOM-killed small machines.
 * Never-measured nodes get the native geometry instead, so only nodes in view mount.
 * The estimate is half a pixel off any DOM size, so a node's first measurement always
 * replaces it; the measured size then reaches the board through its dimension change.
 */
export function createDomFlowGeometryCache() {
	const seeded = new WeakMap<Node, Node>();
	const seed = (
		node: Node,
		width: number,
		height: number,
		handles: Node["handles"],
	) => {
		seeded.set(node, {
			...node,
			measured: { width: width + 0.5, height: height + 0.5 },
			handles,
		});
	};
	const seedNative = (pending: Node[]) => {
		const byId = new Map(pending.map((node) => [node.id, node]));
		for (const estimate of sceneFromFlow(pending, [], GEOMETRY_ONLY).nodes) {
			const node = byId.get(estimate.id);
			if (node)
				seed(
					node,
					estimate.width,
					estimate.height * DOM_HEIGHT_HEADROOM,
					sceneHandles(estimate.pins),
				);
		}
	};
	const seedFixedSize = (node: Node) => {
		if (
			!seeded.has(node) &&
			typeof node.width === "number" &&
			typeof node.height === "number"
		)
			seed(node, node.width, node.height, []);
	};
	const needsSeed = (node: Node) =>
		!node.hidden && !hasMeasuredSize(node) && !seeded.has(node);
	const estimated = (node: Node) =>
		hasMeasuredSize(node) ? node : (seeded.get(node) ?? node);
	return (nodes: Node[]): Node[] => {
		if (nodes.length <= DOM_RENDERER_NODE_BUDGET) return nodes;
		const pending = nodes.filter(needsSeed);
		if (pending.length) {
			seedNative(pending);
			for (const node of pending) seedFixedSize(node);
		}
		return nodes.map(estimated);
	};
}

/** Native rows are deterministic; selected DOM editor measurements are temporary. */
export function prepareNativeSceneNodes(
	nodes: readonly Node[],
	domNodeIds?: ReadonlySet<string>,
): Node[] {
	return nodes.map((node) =>
		isNativeCanvasNode(node) && !(domNodeIds?.has(node.id) ?? node.selected)
			? { ...node, measured: undefined, width: undefined, height: undefined }
			: node,
	);
}

/** Supply navigation, minimap and helper-line geometry without mounting every node editor. */
export function prepareNativeFlowNodes(
	nodes: readonly Node[],
	scene: Scene,
	hoveredNodeId?: string,
): Node[] {
	const geometry = new Map(scene.nodes.map((node) => [node.id, node]));
	return nodes.map((node) => {
		const projected = geometry.get(node.id);
		if (!projected) return node;
		return projectNode(node, projected, hoveredNodeId === node.id);
	});
}

function pinPosition(pin: ScenePin): Position {
	return pin.kind === "reference"
		? pin.direction === "input"
			? Position.Top
			: Position.Bottom
		: pin.direction === "input"
			? Position.Left
			: Position.Right;
}

function projectNode(node: Node, projected: SceneNode, hovered: boolean): Node {
	const dom = projected.dom ?? node.selected ?? false;
	const data = {
		...node.data,
		__flowNativeDom: dom,
		__flowNativeHovered: hovered,
	};
	// DOM editors own their natural size and Handle measurements, including collapse and editor changes.
	if (dom) return { ...node, data };
	return {
		...node,
		data,
		width: projected.width,
		height: projected.height,
		measured: { width: projected.width, height: projected.height },
		handles: sceneHandles(projected.pins),
		style: {
			...node.style,
			pointerEvents: "all",
			background: "none",
			border: "none",
			boxShadow: "none",
		},
	};
}

function projectEdge(edge: Edge, dom: boolean): Edge {
	return {
		...edge,
		reconnectable: dom ? edge.reconnectable : false,
		data: { ...edge.data, __flowNativeDom: dom },
	};
}

/** Custom SVG paths, labels and animations remain owned by their production edge component. */
export function prepareNativeFlowEdges(
	edges: readonly Edge[],
	scene: Scene,
): Edge[] {
	const geometry = new Map(scene.edges.map((edge) => [edge.id, edge]));
	return edges.map((edge) =>
		projectEdge(edge, geometry.get(edge.id)?.dom !== false),
	);
}

function sameNativeGeometry(node: Node, projected: SceneNode): boolean {
	return (
		node.width === projected.width &&
		node.height === projected.height &&
		node.handles?.length === projected.pins.length &&
		projected.pins.every((pin, index) => {
			const handle = node.handles?.[index];
			return (
				handle?.id === pin.id &&
				handle.type === (pin.direction === "input" ? "target" : "source") &&
				handle.position === pinPosition(pin) &&
				handle.x === pin.x - 6 &&
				handle.y === pin.y - 6
			);
		})
	);
}

/** Reuse the array so the board runtime does not adopt unchanged graph records again. */
function projectArray<T>(
	input: readonly T[],
	previous: T[],
	project: (value: T) => T,
): T[] {
	let next: T[] | undefined = input.length === previous.length ? undefined : [];
	for (let index = 0; index < input.length; index++) {
		const value = project(input[index]);
		if (!next && previous[index] !== value) next = previous.slice(0, index);
		next?.push(value);
	}
	return next ?? previous;
}

/** One cache per mounted canvas; source identities belong to its current board snapshots. */
export function createNativeFlowProjectionCache() {
	let nodeCache = new WeakMap<Node, Node>();
	let edgeCache = new WeakMap<Edge, Edge>();
	let sceneNodes: Scene["nodes"] | undefined;
	let sceneEdges: Scene["edges"] | undefined;
	let geometry = new Map<string, SceneNode>();
	let edgeDom = new Map<string, boolean>();
	let previousNodes: Node[] = [];
	let previousEdges: Edge[] = [];
	return {
		nodes(
			nodes: readonly Node[],
			scene: Scene,
			hoveredNodeId?: string,
		): Node[] {
			if (scene.nodes !== sceneNodes) {
				sceneNodes = scene.nodes;
				geometry = new Map();
				for (const node of scene.nodes) geometry.set(node.id, node);
			}
			previousNodes = projectArray(nodes, previousNodes, (node) => {
				const projected = geometry.get(node.id);
				if (!projected) return node;
				const dom = projected.dom ?? node.selected ?? false;
				const hovered = hoveredNodeId === node.id;
				const previous = nodeCache.get(node);
				if (
					previous &&
					previous.data.__flowNativeDom === dom &&
					previous.data.__flowNativeHovered === hovered &&
					(dom || sameNativeGeometry(previous, projected))
				)
					return previous;
				const value = projectNode(node, projected, hovered);
				nodeCache.set(node, value);
				return value;
			});
			return previousNodes;
		},
		edges(edges: readonly Edge[], scene: Scene): Edge[] {
			if (scene.edges !== sceneEdges) {
				sceneEdges = scene.edges;
				edgeDom = new Map();
				for (const edge of scene.edges)
					edgeDom.set(edge.id, edge.dom !== false);
			}
			previousEdges = projectArray(edges, previousEdges, (edge) => {
				const dom = edgeDom.get(edge.id) ?? true;
				const previous = edgeCache.get(edge);
				if (previous && previous.data?.__flowNativeDom === dom) return previous;
				const value = projectEdge(edge, dom);
				edgeCache.set(edge, value);
				return value;
			});
			return previousEdges;
		},
		clear() {
			nodeCache = new WeakMap();
			edgeCache = new WeakMap();
			sceneNodes = undefined;
			sceneEdges = undefined;
			geometry = new Map();
			edgeDom = new Map();
			previousNodes = [];
			previousEdges = [];
		},
	};
}
