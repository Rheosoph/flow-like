"use client";

import { SigmaContainer, useRegisterEvents, useSigma } from "@react-sigma/core";
import "@react-sigma/core/lib/style.css";
import {
	DEFAULT_EDGE_CURVATURE,
	EdgeCurvedArrowProgram,
	indexParallelEdgesIndex,
} from "@sigma/edge-curve";
import { createNodeBorderProgram } from "@sigma/node-border";
import { createNodeImageProgram } from "@sigma/node-image";
import Graph from "graphology";
import forceAtlas2, {
	type ForceAtlas2Settings,
} from "graphology-layout-forceatlas2";
import ForceAtlas2WorkerLayout from "graphology-layout-forceatlas2/worker";
import {
	LoaderCircle,
	Maximize,
	Network,
	RotateCcw,
	ZoomIn,
	ZoomOut,
} from "lucide-react";
import {
	startTransition,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	EdgeArrowProgram,
	NodeCircleProgram,
	createNodeCompoundProgram,
} from "sigma/rendering";
import type {
	LabelStyle,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import { getParallelEdgeRenderAttributes } from "./edge-rendering";
import {
	type ConnectivityPartition,
	type LayoutPosition as GraphPosition,
	computeSeedSpread,
	createAnchoredPosition,
	createDeterministicPosition,
	defaultRelaxIterations,
	getLayoutBounds,
	packNodesOnGrid,
	partitionByConnectivity,
	placeDetachedNodes,
	relaxOverlaps,
} from "./graph-layout";
import { getIconDataUri } from "./icon-svg";
import { drawNodeHover, drawNodeLabel } from "./label-renderer";
import { getGraphTheme, invalidateGraphTheme } from "./theme-colors";

const IconNodeProgram = createNodeCompoundProgram([
	createNodeBorderProgram({
		borders: [
			{
				size: { value: 0.1, mode: "relative" },
				color: { attribute: "borderColor" },
			},
		],
	}),
	createNodeImageProgram({
		padding: 0.3,
		keepWithinCircle: true,
	}),
]);

const LARGE_THRESHOLD = 2000;
const HUGE_THRESHOLD = 10000;
const WORKER_LAYOUT_NODE_THRESHOLD = 1200;
const WORKER_LAYOUT_EDGE_THRESHOLD = 4000;
const NODE_PROGRESS_WEIGHT = 0.3;
const EDGE_PROGRESS_WEIGHT = 0.3;
const SIZE_PROGRESS_WEIGHT = 0.1;
const LAYOUT_PROGRESS_WEIGHT = 0.3;
const EXPANSION_OVERLAY_DELAY_MS = 400;
const PRESERVE_RELAX_ITERATIONS = 8;
const LOADING_BAR_DELAYS_MS = [0, 120, 240] as const;

type ForceAtlas2WorkerInstance = {
	start: () => void;
	stop: () => void;
	kill: () => void;
};

type ForceAtlas2WorkerConstructor = new (
	graph: Graph,
	params?: { settings?: ForceAtlas2Settings },
) => ForceAtlas2WorkerInstance;

const ForceAtlas2Worker =
	ForceAtlas2WorkerLayout as unknown as ForceAtlas2WorkerConstructor;

export interface GraphCanvasProps {
	data: SubgraphResult | null;
	loading?: boolean;
	selectedNodeId?: string | null;
	selectedEdgeKey?: string | null;
	highlightedNodeIds?: Set<string>;
	highlightedEdgeIds?: Set<string>;
	hiddenLabels?: Set<string>;
	onNodeClick?: (nodeId: string) => void;
	onNodeShiftClick?: (nodeId: string, label: string) => void;
	onEdgeClick?: (edgeKey: string) => void;
	onStageClick?: () => void;
	className?: string;
}

function getDefaultNodeColor(): string {
	const t = getGraphTheme();
	return t.isDark ? "#94a3b8" : "#64748b";
}

function getDefaultEdgeColor(): string {
	const t = getGraphTheme();
	return t.isDark ? "#64748b" : "#94a3b8";
}

function styleToNodeColor(style?: LabelStyle): string {
	return style?.color ?? getDefaultNodeColor();
}

function styleToEdgeColor(style?: LabelStyle): string {
	return style?.color ?? getDefaultEdgeColor();
}

function colorToHex(color: string): string {
	if (color.startsWith("#")) return color;
	if (color.startsWith("rgba") || color.startsWith("rgb")) {
		const m = color.match(/[\d.]+/g);
		if (m && m.length >= 3) {
			const r = Number.parseInt(m[0]).toString(16).padStart(2, "0");
			const g = Number.parseInt(m[1]).toString(16).padStart(2, "0");
			const b = Number.parseInt(m[2]).toString(16).padStart(2, "0");
			return `#${r}${g}${b}`;
		}
	}
	return colorToHex(getDefaultEdgeColor());
}

interface ColumnRange {
	min: number;
	max: number;
}

function toFiniteNumber(value: unknown): number | undefined {
	const num = typeof value === "number" ? value : Number(value);
	return Number.isFinite(num) ? num : undefined;
}

function computeColumnRanges(
	nodes: readonly SubgraphNode[],
): Map<string, ColumnRange> {
	const ranges = new Map<string, ColumnRange>();
	for (const node of nodes) {
		const size = node.style?.size;
		if (size?.mode !== "by-column" || !size.column) continue;
		const value = toFiniteNumber(node.props?.[size.column]);
		if (value === undefined) continue;
		const existing = ranges.get(size.column);
		if (!existing) {
			ranges.set(size.column, { min: value, max: value });
		} else {
			existing.min = Math.min(existing.min, value);
			existing.max = Math.max(existing.max, value);
		}
	}
	return ranges;
}

function styleToNodeSize(
	style?: LabelStyle,
	degree?: number,
	columnRanges?: ReadonlyMap<string, ColumnRange>,
	props?: Record<string, unknown>,
): number {
	if (!style?.size) return 10;
	const { mode } = style.size;
	if (mode === "fixed") return Math.max(8, style.size.value ?? 10);
	if (mode === "by-degree" && degree !== undefined) {
		const min = style.size.min ?? 8;
		const max = style.size.max ?? 28;
		return Math.min(max, min + degree * 1.5);
	}
	if (mode === "by-column" && style.size.column) {
		const min = style.size.min ?? 8;
		const max = style.size.max ?? 28;
		const value = toFiniteNumber(props?.[style.size.column]);
		const range = columnRanges?.get(style.size.column);
		if (value === undefined || !range || range.max <= range.min) {
			return (min + max) / 2;
		}
		const ratio = (value - range.min) / (range.max - range.min);
		return min + ratio * (max - min);
	}
	return 10;
}

function hexToRgba(hex: string, alpha: number): string {
	const r = Number.parseInt(hex.slice(1, 3), 16);
	const g = Number.parseInt(hex.slice(3, 5), 16);
	const b = Number.parseInt(hex.slice(5, 7), 16);
	return `rgba(${r},${g},${b},${alpha})`;
}

function getBaseEdgeAlpha(nodeCount: number): number {
	if (nodeCount >= HUGE_THRESHOLD) return 0.08;
	if (nodeCount >= LARGE_THRESHOLD) return 0.2;
	return 0.3;
}

const CONTEXT_DIM_EDGE_SIZE = 0.75;
const CONTEXT_DIM_EDGE_ALPHA = 0.08;
const CONTEXT_DIM_NODE_AMOUNT = 0.82;

function dimTowardBackground(color: string): string {
	const theme = getGraphTheme();
	const [bgR, bgG, bgB] = theme.bgRgb;
	const hex = colorToHex(color);
	const r = Number.parseInt(hex.slice(1, 3), 16);
	const g = Number.parseInt(hex.slice(3, 5), 16);
	const b = Number.parseInt(hex.slice(5, 7), 16);
	const mix = (channel: number, target: number) =>
		Math.round(channel + (target - channel) * CONTEXT_DIM_NODE_AMOUNT);
	return `rgb(${mix(r, bgR)},${mix(g, bgG)},${mix(b, bgB)})`;
}

function getNodeChunkSize(nodeCount: number): number {
	if (nodeCount >= HUGE_THRESHOLD) return 300;
	if (nodeCount >= LARGE_THRESHOLD) return 600;
	return 1200;
}

function getEdgeChunkSize(edgeCount: number): number {
	if (edgeCount >= 100000) return 750;
	if (edgeCount >= 25000) return 1500;
	if (edgeCount >= 5000) return 3000;
	return 6000;
}

function getLayoutBatchIterations(nodeCount: number): number {
	if (nodeCount >= HUGE_THRESHOLD) return 6;
	if (nodeCount >= LARGE_THRESHOLD) return 10;
	return 24;
}

function shouldUseWorkerLayout(nodeCount: number, edgeCount: number): boolean {
	return (
		nodeCount >= WORKER_LAYOUT_NODE_THRESHOLD ||
		edgeCount >= WORKER_LAYOUT_EDGE_THRESHOLD
	);
}

function getWorkerLayoutDuration(nodeCount: number, edgeCount: number): number {
	const density = edgeCount / Math.max(1, nodeCount);
	if (nodeCount >= HUGE_THRESHOLD) return 1400;
	if (nodeCount >= LARGE_THRESHOLD || density > 4) return 1800;
	return 1200;
}

function waitForNextFrame(): Promise<void> {
	return new Promise((resolve) => {
		if (typeof window === "undefined") {
			setTimeout(resolve, 0);
			return;
		}
		window.requestAnimationFrame(() => resolve());
	});
}

async function processInChunks<T>(
	items: readonly T[],
	chunkSize: number,
	processItem: (item: T) => void,
	onProgress: (progress: number) => void,
	isCancelled: () => boolean,
): Promise<boolean> {
	if (items.length === 0) {
		onProgress(1);
		return !isCancelled();
	}

	for (let start = 0; start < items.length; start += chunkSize) {
		if (isCancelled()) return false;

		const end = Math.min(items.length, start + chunkSize);
		for (let index = start; index < end; index += 1) {
			processItem(items[index]);
		}

		onProgress(end / items.length);
		if (end < items.length) {
			await waitForNextFrame();
		}
	}

	return !isCancelled();
}

function getFA2Settings(
	nodeCount: number,
	edgeCount: number,
): ForceAtlas2Settings {
	const density = edgeCount / Math.max(1, nodeCount);
	const isHuge = nodeCount >= HUGE_THRESHOLD;
	const isLarge = nodeCount >= LARGE_THRESHOLD;
	const isDense = density > 3;
	const isSparse = density < 1.2;

	if (isHuge) {
		return {
			gravity: 0.05,
			scalingRatio: 20,
			slowDown: 2,
			barnesHutOptimize: true,
			barnesHutTheta: 0.8,
			strongGravityMode: false,
			linLogMode: true,
			edgeWeightInfluence: 0,
			adjustSizes: false,
		};
	}

	if (isLarge) {
		return {
			gravity: 0.2,
			scalingRatio: 12,
			slowDown: 3,
			barnesHutOptimize: true,
			barnesHutTheta: 0.6,
			strongGravityMode: false,
			linLogMode: true,
			edgeWeightInfluence: 0,
			adjustSizes: false,
		};
	}

	// `strongGravityMode` pulls harder the further out a node sits, so on sparse
	// graphs it wins against repulsion and collapses everything into one disc.
	// Plain gravity is a constant inward force: it still keeps components from
	// drifting off, but lets `scalingRatio` decide the spacing.
	return {
		gravity: isDense ? 0.5 : isSparse ? 0.6 : 1,
		scalingRatio: isDense ? 8 : isSparse ? 24 : 14,
		slowDown: isDense ? 5 : 8,
		barnesHutOptimize: nodeCount > 200,
		barnesHutTheta: 0.5,
		strongGravityMode: false,
		linLogMode: true,
		edgeWeightInfluence: isDense ? 0 : 1,
		adjustSizes: true,
	};
}

function getRelaxBatchIterations(nodeCount: number): number {
	if (nodeCount >= HUGE_THRESHOLD) return 2;
	if (nodeCount >= LARGE_THRESHOLD) return 4;
	return 12;
}

/**
 * Guarantees the spacing the simulation only approximates, then parks detached
 * nodes beside the core. Shared by the inline and worker layout paths so both
 * finish in the same readable state.
 */
async function finishLayoutAsync(
	graph: Graph,
	partition: ConnectivityPartition,
	isCancelled: () => boolean,
	updateProgress?: (progress: number, detail: string) => void,
) {
	const { connected, isolated } = partition;
	const totalIterations = defaultRelaxIterations(connected.length);
	const batchIterations = getRelaxBatchIterations(connected.length);
	let completed = 0;

	while (completed < totalIterations) {
		if (isCancelled()) return;

		const batch = Math.min(batchIterations, totalIterations - completed);
		const performed = relaxOverlaps(graph, connected, { iterations: batch });
		completed += batch;

		updateProgress?.(
			completed / totalIterations,
			"Separating overlapping nodes.",
		);

		// A pass that resolved nothing means the set is already clean.
		if (performed < batch) break;
		if (completed < totalIterations) await waitForNextFrame();
	}

	if (isCancelled()) return;
	placeDetachedNodes(graph, isolated, getLayoutBounds(graph, connected));
}

async function applyLayoutAsync(
	graph: Graph,
	partition: ConnectivityPartition,
	updateProgress: (progress: number, detail: string) => void,
	isCancelled: () => boolean,
) {
	if (graph.order === 0) {
		updateProgress(1, "No nodes to arrange.");
		return;
	}

	const nodeCount = graph.order;
	const edgeCount = graph.size;
	// Detached nodes carry no structure, so density is measured against the part
	// of the graph the simulation is actually solving.
	const settings = getFA2Settings(partition.connected.length, edgeCount);

	let totalIterations: number;
	if (nodeCount >= HUGE_THRESHOLD) totalIterations = 80;
	else if (nodeCount >= LARGE_THRESHOLD) totalIterations = 150;
	else totalIterations = Math.min(800, Math.max(200, nodeCount * 4));

	const batchIterations = getLayoutBatchIterations(nodeCount);
	let completedIterations = 0;

	while (completedIterations < totalIterations) {
		if (isCancelled()) return;

		const batch = Math.min(
			batchIterations,
			totalIterations - completedIterations,
		);
		forceAtlas2.assign(graph, { iterations: batch, settings });
		completedIterations += batch;

		updateProgress(
			(completedIterations / totalIterations) * 0.85,
			`${completedIterations.toLocaleString()} / ${totalIterations.toLocaleString()} layout passes complete.`,
		);

		if (completedIterations < totalIterations) {
			await waitForNextFrame();
		}
	}

	await finishLayoutAsync(graph, partition, isCancelled, (progress, detail) => {
		updateProgress(0.85 + progress * 0.15, detail);
	});
}

interface GraphPreparationState {
	phase: "idle" | "building" | "layout" | "ready";
	title: string;
	detail: string;
	progress: number;
	nodeCount: number;
	edgeCount: number;
}

interface GraphLoadingOverlayState {
	title: string;
	detail: string;
	progress: number;
	nodeCount: number;
	edgeCount: number;
}

interface GraphBuildOptions {
	previousPositions: ReadonlyMap<string, GraphPosition>;
	anchorNodeId?: string | null;
	forceLayout?: boolean;
}

interface GraphBuildResult {
	graph: Graph;
	shouldRunWorkerLayout: boolean;
}

const IDLE_PREPARATION_STATE: GraphPreparationState = {
	phase: "idle",
	title: "",
	detail: "",
	progress: 0,
	nodeCount: 0,
	edgeCount: 0,
};

function buildNeighborLookup(data: SubgraphResult): Map<string, string[]> {
	const neighborLookup = new Map<string, string[]>();

	for (const edge of data.edges) {
		const sourceNeighbors = neighborLookup.get(edge.source) ?? [];
		sourceNeighbors.push(edge.target);
		neighborLookup.set(edge.source, sourceNeighbors);

		const targetNeighbors = neighborLookup.get(edge.target) ?? [];
		targetNeighbors.push(edge.source);
		neighborLookup.set(edge.target, targetNeighbors);
	}

	return neighborLookup;
}

function getNodePosition(graph: Graph, nodeId: string): GraphPosition | null {
	if (!graph.hasNode(nodeId)) return null;

	const x = graph.getNodeAttribute(nodeId, "x");
	const y = graph.getNodeAttribute(nodeId, "y");

	if (
		typeof x !== "number" ||
		!Number.isFinite(x) ||
		typeof y !== "number" ||
		!Number.isFinite(y)
	) {
		return null;
	}

	return { x, y };
}

function snapshotGraphPositions(
	graph: Graph | null,
): Map<string, GraphPosition> {
	const positions = new Map<string, GraphPosition>();
	if (!graph) return positions;

	graph.forEachNode((nodeId) => {
		const position = getNodePosition(graph, nodeId);
		if (position) {
			positions.set(nodeId, position);
		}
	});

	return positions;
}

function resolveInitialNodePosition({
	nodeId,
	previousPositions,
	assignedPositions,
	neighborLookup,
	anchorNodeId,
	seedSpread,
	anchorSpread,
}: {
	nodeId: string;
	previousPositions: ReadonlyMap<string, GraphPosition>;
	assignedPositions: ReadonlyMap<string, GraphPosition>;
	neighborLookup: ReadonlyMap<string, string[]>;
	anchorNodeId?: string | null;
	seedSpread: number;
	anchorSpread: number;
}): GraphPosition {
	const existingPosition = previousPositions.get(nodeId);
	if (existingPosition) return existingPosition;

	const candidateAnchorIds = [
		...(anchorNodeId ? [anchorNodeId] : []),
		...(neighborLookup.get(nodeId) ?? []),
	];

	for (const candidateId of candidateAnchorIds) {
		const anchorPosition =
			assignedPositions.get(candidateId) ?? previousPositions.get(candidateId);
		if (anchorPosition) {
			return createAnchoredPosition(anchorPosition, nodeId, anchorSpread);
		}
	}

	return createDeterministicPosition(nodeId, seedSpread);
}

async function buildGraphAsync(
	data: SubgraphResult,
	updateState: (state: GraphPreparationState) => void,
	isCancelled: () => boolean,
	{ previousPositions, anchorNodeId, forceLayout = false }: GraphBuildOptions,
): Promise<GraphBuildResult | null> {
	const graph = new Graph({ multi: true, type: "directed" });
	const nodeCount = data.nodes.length;
	const edgeCount = data.edges.length;
	const isHuge = nodeCount >= HUGE_THRESHOLD;
	const isLarge = nodeCount >= LARGE_THRESHOLD;
	const preservedNodeCount = data.nodes.reduce(
		(count, node) => count + Number(previousPositions.has(node.id)),
		0,
	);
	const preserveLayout =
		!forceLayout &&
		preservedNodeCount > 0 &&
		preservedNodeCount * 5 >= nodeCount;
	const nodeIds = new Set<string>();
	const degreeMap = new Map<string, number>();
	const assignedPositions = new Map<string, GraphPosition>();
	const neighborLookup = buildNeighborLookup(data);
	const columnRanges = computeColumnRanges(data.nodes);
	const seedSpread = computeSeedSpread(nodeCount);
	// Newly expanded nodes land in a small ring around the node they came from,
	// close enough to read as related but clear of it.
	const anchorSpread = computeSeedSpread(12);

	const publish = (
		progress: number,
		title: string,
		detail: string,
		phase: GraphPreparationState["phase"] = "building",
	) => {
		updateState({
			phase,
			title,
			detail,
			progress,
			nodeCount,
			edgeCount,
		});
	};

	publish(
		0.02,
		"Preparing graph scene",
		"Scheduling graph work so the page stays responsive.",
	);

	const nodesBuilt = await processInChunks(
		data.nodes,
		getNodeChunkSize(nodeCount),
		(node) => {
			if (nodeIds.has(node.id)) return;

			const nodeColor = styleToNodeColor(node.style);
			const position = resolveInitialNodePosition({
				nodeId: node.id,
				previousPositions,
				assignedPositions,
				neighborLookup,
				anchorNodeId,
				seedSpread,
				anchorSpread,
			});
			const attrs: Record<string, unknown> = {
				label: node.caption ?? node.id,
				subtitle: node.label,
				size: 10,
				color: nodeColor,
				x: position.x,
				y: position.y,
				nodeLabel: node.label,
				originalColor: nodeColor,
				borderColor: nodeColor,
				usesDefaultColor: !node.style?.color,
			};

			if (!isLarge) {
				attrs.image = getIconDataUri(node.style?.icon ?? "database");
				attrs.type = "bordered-image";
				attrs.props = node.props;
			}

			graph.addNode(node.id, attrs);
			nodeIds.add(node.id);
			assignedPositions.set(node.id, position);
			degreeMap.set(node.id, 0);
		},
		(fraction) => {
			publish(
				NODE_PROGRESS_WEIGHT * fraction,
				"Staging nodes",
				`${Math.round(fraction * 100)}% of node metadata ready.`,
			);
		},
		isCancelled,
	);

	if (!nodesBuilt || isCancelled()) return null;

	const edgesBuilt = await processInChunks(
		data.edges,
		getEdgeChunkSize(edgeCount),
		(edge) => {
			if (!nodeIds.has(edge.source) || !nodeIds.has(edge.target)) return;

			const edgeHex = colorToHex(styleToEdgeColor(edge.style));
			const edgeWidth = edge.style?.width ?? 1;
			graph.addEdge(edge.source, edge.target, {
				label: edge.label,
				size: (isHuge ? 0.3 : isLarge ? 0.6 : 1) * edgeWidth,
				color: hexToRgba(edgeHex, getBaseEdgeAlpha(nodeCount)),
				originalColor: edgeHex,
				type: "arrow",
				edgeId: edge.id,
				forceLabel: false,
				usesDefaultColor: !edge.style?.color,
			});

			degreeMap.set(edge.source, (degreeMap.get(edge.source) ?? 0) + 1);
			degreeMap.set(edge.target, (degreeMap.get(edge.target) ?? 0) + 1);
		},
		(fraction) => {
			publish(
				NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT * fraction,
				"Linking connections",
				`${Math.round(fraction * 100)}% of edges connected.`,
			);
		},
		isCancelled,
	);

	if (!edgesBuilt || isCancelled()) return null;

	if (!isLarge) {
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT,
			"Optimizing connections",
			"Resolving parallel edges for clearer paths.",
		);
		indexParallelEdgesIndex(graph);
		graph.forEachEdge((edge, edgeAttrs) => {
			const parallelIndex = edgeAttrs.parallelIndex as
				| number
				| null
				| undefined;
			const parallelMaxIndex = edgeAttrs.parallelMaxIndex as
				| number
				| null
				| undefined;
			graph.mergeEdgeAttributes(
				edge,
				getParallelEdgeRenderAttributes(
					parallelIndex,
					parallelMaxIndex,
					DEFAULT_EDGE_CURVATURE,
				),
			);
		});
		if (isCancelled()) return null;
		await waitForNextFrame();
	}

	const density = graph.size / Math.max(1, graph.order);
	const sized = await processInChunks(
		data.nodes,
		getNodeChunkSize(nodeCount),
		(node) => {
			if (!graph.hasNode(node.id)) return;

			const baseSize = styleToNodeSize(
				node.style,
				degreeMap.get(node.id),
				columnRanges,
				node.props,
			);
			let scaledSize = baseSize;
			if (isHuge) scaledSize = Math.max(3, baseSize * 0.4);
			else if (density > 4) scaledSize = baseSize * 0.85;
			graph.setNodeAttribute(node.id, "size", scaledSize);
		},
		(fraction) => {
			publish(
				NODE_PROGRESS_WEIGHT +
					EDGE_PROGRESS_WEIGHT +
					SIZE_PROGRESS_WEIGHT * fraction,
				"Balancing node sizes",
				"Scaling nodes for readability.",
			);
		},
		isCancelled,
	);

	if (!sized || isCancelled()) return null;

	const partition = partitionByConnectivity(graph);

	if (preserveLayout) {
		// Expansions must not reshuffle the view, so only the stacking that the
		// anchored seeding introduced gets nudged apart.
		relaxOverlaps(graph, Array.from(nodeIds), {
			iterations: PRESERVE_RELAX_ITERATIONS,
		});
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT,
			"Keeping layout stable",
			"Reusing the current node positions while adding new connections.",
			"ready",
		);
		return {
			graph,
			shouldRunWorkerLayout: false,
		};
	}

	// A force layout has nothing to solve without edges: every node would just
	// fall into the same gravity well. A grid is exact, instant and readable.
	if (partition.connected.length <= 1) {
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT,
			"Arranging nodes",
			"No connections to lay out — placing nodes on a grid.",
			"layout",
		);
		packNodesOnGrid(graph, [...partition.connected, ...partition.isolated]);
		publish(1, "Graph ready", "Rendering interactive view.", "ready");
		return {
			graph,
			shouldRunWorkerLayout: false,
		};
	}

	if (shouldUseWorkerLayout(partition.connected.length, edgeCount)) {
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT,
			"Preparing worker layout",
			"Rendering the graph now and refining positions in the background.",
			"ready",
		);
		return {
			graph,
			shouldRunWorkerLayout: true,
		};
	}

	await applyLayoutAsync(
		graph,
		partition,
		(progress, detail) => {
			publish(
				NODE_PROGRESS_WEIGHT +
					EDGE_PROGRESS_WEIGHT +
					SIZE_PROGRESS_WEIGHT +
					LAYOUT_PROGRESS_WEIGHT * progress,
				"Running layout",
				detail,
				"layout",
			);
		},
		isCancelled,
	);

	if (isCancelled()) return null;

	publish(1, "Graph ready", "Rendering interactive view.", "ready");
	return {
		graph,
		shouldRunWorkerLayout: false,
	};
}

interface HighlightState {
	hoveredNode: string | null;
	hoveredEdge: string | null;
	selectedNodeId: string | null;
	selectedEdgeKey: string | null;
	highlightedNodeIds: Set<string> | undefined;
	highlightedEdgeIds: Set<string> | undefined;
	hiddenLabels: Set<string> | undefined;
	neighborSet: Set<string> | null;
	connectedEdgeSet: Set<string> | null;
}

function computeNeighborSets(
	graph: Graph,
	activeNode: string | null,
): { neighborSet: Set<string> | null; connectedEdgeSet: Set<string> | null } {
	if (!activeNode || !graph.hasNode(activeNode))
		return { neighborSet: null, connectedEdgeSet: null };
	const neighborSet = new Set<string>([activeNode]);
	for (const neighbor of graph.neighbors(activeNode)) {
		neighborSet.add(neighbor);
	}
	const connectedEdgeSet = new Set<string>();
	for (const edge of graph.edges(activeNode)) {
		connectedEdgeSet.add(edge);
	}
	return { neighborSet, connectedEdgeSet };
}

function formatOverlayMetric(value: number): string {
	return value > 0 ? value.toLocaleString() : "-";
}

function GraphCanvasLoadingOverlay({
	title,
	detail,
	progress,
	nodeCount,
	edgeCount,
	dimmed,
}: GraphLoadingOverlayState & { dimmed: boolean }) {
	const progressPercent = Math.max(
		8,
		Math.min(100, Math.round(progress * 100)),
	);

	return (
		<div
			className={`absolute inset-0 z-20 flex items-center justify-center ${
				dimmed ? "bg-background/80 backdrop-blur-sm" : "bg-background"
			}`}
		>
			<div className="relative mx-4 w-full max-w-lg overflow-hidden rounded-3xl border bg-card/95 p-6 shadow-2xl">
				<div className="absolute -left-10 -top-10 h-32 w-32 rounded-full bg-primary/10 blur-3xl" />
				<div className="absolute -bottom-14 -right-10 h-40 w-40 rounded-full bg-accent/40 blur-3xl" />
				<div className="relative space-y-5">
					<div className="flex items-start gap-4">
						<div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border bg-background/80 text-primary shadow-sm">
							<Network className="h-5 w-5" />
						</div>
						<div className="min-w-0 flex-1">
							<div className="flex items-center gap-2">
								<p className="text-sm font-semibold">{title}</p>
								<LoaderCircle className="h-4 w-4 animate-spin text-primary" />
							</div>
							<p className="mt-1 text-xs leading-5 text-muted-foreground">
								{detail}
							</p>
						</div>
					</div>

					<div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
						<div className="rounded-2xl border bg-background/70 p-3">
							<p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
								Nodes
							</p>
							<p className="mt-1 text-lg font-semibold">
								{formatOverlayMetric(nodeCount)}
							</p>
						</div>
						<div className="rounded-2xl border bg-background/70 p-3">
							<p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
								Edges
							</p>
							<p className="mt-1 text-lg font-semibold">
								{formatOverlayMetric(edgeCount)}
							</p>
						</div>
						<div className="col-span-2 rounded-2xl border bg-background/70 p-3 sm:col-span-1">
							<p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
								Progress
							</p>
							<p className="mt-1 text-lg font-semibold">{progressPercent}%</p>
						</div>
					</div>

					<div className="space-y-2">
						<div className="flex items-center justify-between text-xs text-muted-foreground">
							<span>Preparing interactive canvas</span>
							<span>{progressPercent}%</span>
						</div>
						<div className="h-2 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full rounded-full bg-primary transition-[width] duration-300 ease-out"
								style={{ width: `${progressPercent}%` }}
							/>
						</div>
					</div>

					<div className="grid grid-cols-3 gap-2">
						{LOADING_BAR_DELAYS_MS.map((delayMs) => (
							<div
								key={`loading-bar-${delayMs}`}
								className="rounded-2xl border bg-background/60 px-3 py-2"
							>
								<div className="h-2 w-12 rounded bg-muted/80" />
								<div
									className="mt-3 h-2 rounded bg-muted animate-pulse"
									style={{ animationDelay: `${delayMs}ms` }}
								/>
							</div>
						))}
					</div>
				</div>
			</div>
		</div>
	);
}

function GraphEvents({
	onNodeClick,
	onNodeShiftClick,
	onEdgeClick,
	onStageClick,
	highlightRef,
	graph,
}: {
	onNodeClick?: (nodeId: string) => void;
	onNodeShiftClick?: (nodeId: string, label: string) => void;
	onEdgeClick?: (edgeKey: string) => void;
	onStageClick?: () => void;
	highlightRef: React.MutableRefObject<HighlightState>;
	graph: Graph;
}) {
	const sigma = useSigma();
	const registerEvents = useRegisterEvents();

	useEffect(() => {
		registerEvents({
			enterNode: ({ node }) => {
				highlightRef.current.hoveredNode = node;
				highlightRef.current.hoveredEdge = null;
				const activeNode = highlightRef.current.selectedNodeId ?? node;
				const sets = computeNeighborSets(graph, activeNode);
				highlightRef.current.neighborSet = sets.neighborSet;
				highlightRef.current.connectedEdgeSet = sets.connectedEdgeSet;
				sigma.refresh({ skipIndexation: true });
			},
			leaveNode: () => {
				highlightRef.current.hoveredNode = null;
				const activeNode = highlightRef.current.selectedNodeId;
				const sets = computeNeighborSets(graph, activeNode);
				highlightRef.current.neighborSet = sets.neighborSet;
				highlightRef.current.connectedEdgeSet = sets.connectedEdgeSet;
				sigma.refresh({ skipIndexation: true });
			},
			enterEdge: ({ edge }) => {
				highlightRef.current.hoveredEdge = edge;
				sigma.refresh({ skipIndexation: true });
			},
			leaveEdge: () => {
				highlightRef.current.hoveredEdge = null;
				sigma.refresh({ skipIndexation: true });
			},
			clickNode: ({ node, event }) => {
				if (event.original.shiftKey) {
					const label = graph.getNodeAttribute(node, "nodeLabel") as string;
					onNodeShiftClick?.(node, label);
				} else {
					onNodeClick?.(node);
				}
			},
			clickEdge: ({ edge }) => {
				const edgeId = graph.getEdgeAttribute(edge, "edgeId") as
					| string
					| undefined;
				if (edgeId) {
					onEdgeClick?.(edgeId);
				}
			},
			clickStage: () => onStageClick?.(),
		});
	}, [
		registerEvents,
		sigma,
		graph,
		onNodeClick,
		onNodeShiftClick,
		onEdgeClick,
		onStageClick,
		highlightRef,
	]);

	return null;
}

function SigmaRefresher({ refreshKey }: { refreshKey: string }) {
	const sigma = useSigma();
	useEffect(() => {
		void refreshKey;
		try {
			sigma.refresh();
		} catch {
			// WebGL context may be lost
		}
	}, [sigma, refreshKey]);
	return null;
}

function SigmaWorkerLayout({
	enabled,
	graphRevision,
	nodeCount,
	edgeCount,
	onRunningChange,
}: {
	enabled: boolean;
	graphRevision: number;
	nodeCount: number;
	edgeCount: number;
	onRunningChange?: (running: boolean) => void;
}) {
	const sigma = useSigma();
	const layoutSettings = useMemo(
		() => getFA2Settings(nodeCount, edgeCount),
		[nodeCount, edgeCount],
	);
	const duration = useMemo(
		() => getWorkerLayoutDuration(nodeCount, edgeCount),
		[nodeCount, edgeCount],
	);

	useEffect(() => {
		void graphRevision;
		onRunningChange?.(false);

		if (!enabled || nodeCount === 0) {
			onRunningChange?.(false);
			return;
		}

		const currentGraph = sigma.getGraph();
		if (currentGraph.order === 0) {
			return;
		}

		const layout = new ForceAtlas2Worker(currentGraph, {
			settings: layoutSettings,
		});
		let disposed = false;
		let frameId = 0;

		const refresh = () => {
			if (disposed) return;
			try {
				sigma.refresh({ skipIndexation: true });
			} catch {
				// WebGL context may be lost during background layout updates
			}
			frameId = window.requestAnimationFrame(refresh);
		};

		try {
			layout.start();
			onRunningChange?.(true);
			frameId = window.requestAnimationFrame(refresh);
		} catch {
			try {
				layout.kill();
			} catch {
				// Layout worker may already be disposed
			}
			return;
		}

		const timeoutId = window.setTimeout(() => {
			if (disposed) return;
			window.cancelAnimationFrame(frameId);
			try {
				layout.stop();
			} catch {
				// Layout worker may already be disposed
			}
			onRunningChange?.(false);
			// The worker is stopped on a timer, not on convergence, so the graph it
			// leaves behind still overlaps. Settle it before the final paint.
			const partition = partitionByConnectivity(currentGraph);
			relaxOverlaps(currentGraph, partition.connected, {
				iterations: defaultRelaxIterations(partition.connected.length),
			});
			placeDetachedNodes(
				currentGraph,
				partition.isolated,
				getLayoutBounds(currentGraph, partition.connected),
			);
			try {
				sigma.refresh({ skipIndexation: true });
			} catch {
				// WebGL context may be lost during final refresh
			}
		}, duration);

		return () => {
			disposed = true;
			window.clearTimeout(timeoutId);
			window.cancelAnimationFrame(frameId);
			try {
				layout.stop();
			} catch {
				// Layout worker may already be disposed
			}
			try {
				layout.kill();
			} catch {
				// Layout worker may already be disposed
			}
			onRunningChange?.(false);
			try {
				sigma.refresh({ skipIndexation: true });
			} catch {
				// WebGL context may be lost during cleanup
			}
		};
	}, [
		duration,
		enabled,
		graphRevision,
		layoutSettings,
		nodeCount,
		onRunningChange,
		sigma,
	]);

	return null;
}

function SigmaControls({
	onResetLayout,
	disabled,
}: { onResetLayout: () => void; disabled?: boolean }) {
	const sigma = useSigma();
	const buttonClassName = `h-8 w-8 flex items-center justify-center rounded transition-colors ${
		disabled ? "cursor-not-allowed opacity-50" : "hover:bg-accent"
	}`;

	const handleZoomIn = useCallback(() => {
		const camera = sigma.getCamera();
		camera.animatedZoom({ duration: 200 });
	}, [sigma]);

	const handleZoomOut = useCallback(() => {
		const camera = sigma.getCamera();
		camera.animatedUnzoom({ duration: 200 });
	}, [sigma]);

	const handleFitView = useCallback(() => {
		const camera = sigma.getCamera();
		camera.animatedReset({ duration: 300 });
	}, [sigma]);

	return (
		<div className="absolute top-3 right-3 z-10 flex flex-col gap-1 bg-background/80 backdrop-blur-sm rounded-lg border p-1 shadow-sm">
			<button
				type="button"
				className={buttonClassName}
				onClick={handleZoomIn}
				title="Zoom in"
				disabled={disabled}
			>
				<ZoomIn className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={handleZoomOut}
				title="Zoom out"
				disabled={disabled}
			>
				<ZoomOut className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={handleFitView}
				title="Fit to view"
				disabled={disabled}
			>
				<Maximize className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={onResetLayout}
				title="Re-run layout"
				disabled={disabled}
			>
				<RotateCcw className="h-4 w-4" />
			</button>
		</div>
	);
}

export function GraphCanvas({
	data,
	loading,
	selectedNodeId,
	selectedEdgeKey,
	highlightedNodeIds,
	highlightedEdgeIds,
	hiddenLabels,
	onNodeClick,
	onNodeShiftClick,
	onEdgeClick,
	onStageClick,
	className,
}: GraphCanvasProps) {
	const [themeTick, setThemeTick] = useState(0);
	const [layoutRunKey, setLayoutRunKey] = useState(0);
	const [graph, setGraph] = useState<Graph | null>(null);
	const [graphRevision, setGraphRevision] = useState(0);
	const [shouldRunWorkerLayout, setShouldRunWorkerLayout] = useState(false);
	const [preparationState, setPreparationState] =
		useState<GraphPreparationState>(IDLE_PREPARATION_STATE);
	const [showOverlay, setShowOverlay] = useState(false);
	const [isWorkerLayoutRunning, setIsWorkerLayoutRunning] = useState(false);
	const preparedDataRef = useRef<SubgraphResult | null>(null);
	const graphRef = useRef<Graph | null>(graph);
	const loadingRef = useRef(loading);
	const selectedNodeIdRef = useRef<string | null>(selectedNodeId ?? null);
	const forceLayoutRef = useRef(false);
	const lastPaletteKeyRef = useRef<string>("");

	graphRef.current = graph;
	loadingRef.current = loading;
	selectedNodeIdRef.current = selectedNodeId ?? null;

	useEffect(() => {
		const paletteKey = () => {
			const t = getGraphTheme();
			return `${t.bgRgb.join(",")}|${t.fgRgb.join(",")}|${t.isDark}`;
		};

		lastPaletteKeyRef.current = paletteKey();

		const observer = new MutationObserver(() => {
			invalidateGraphTheme();
			const nextKey = paletteKey();
			if (nextKey === lastPaletteKeyRef.current) return;
			lastPaletteKeyRef.current = nextKey;
			setThemeTick((current) => current + 1);
		});

		observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "data-theme", "style"],
		});

		return () => observer.disconnect();
	}, []);

	useEffect(() => {
		void layoutRunKey;
		const nextData = data;
		const previousPositions = snapshotGraphPositions(graphRef.current);
		const forceLayout = forceLayoutRef.current;
		forceLayoutRef.current = false;
		let cancelled = false;

		if (
			!nextData ||
			(nextData.nodes.length === 0 && nextData.edges.length === 0)
		) {
			if (!loadingRef.current) {
				preparedDataRef.current = null;
				setGraph(null);
				setShouldRunWorkerLayout(false);
				setIsWorkerLayoutRunning(false);
				setPreparationState(IDLE_PREPARATION_STATE);
			}
			return () => {
				cancelled = true;
			};
		}

		setPreparationState({
			phase: "building",
			title: "Preparing graph scene",
			detail: "Scheduling graph work so the page stays responsive.",
			progress: 0.01,
			nodeCount: nextData.nodes.length,
			edgeCount: nextData.edges.length,
		});

		void (async () => {
			await waitForNextFrame();
			if (cancelled) return;

			const buildResult = await buildGraphAsync(
				nextData,
				(nextState) => {
					if (!cancelled) {
						setPreparationState(nextState);
					}
				},
				() => cancelled,
				{
					previousPositions,
					anchorNodeId: selectedNodeIdRef.current,
					forceLayout,
				},
			);

			if (!buildResult || cancelled) return;

			preparedDataRef.current = nextData;
			startTransition(() => {
				setGraph(buildResult.graph);
				setGraphRevision((current) => current + 1);
				setShouldRunWorkerLayout(buildResult.shouldRunWorkerLayout);
				setIsWorkerLayoutRunning(false);
				setPreparationState({
					phase: "ready",
					title: "Graph ready",
					detail: "Interactive view updated.",
					progress: 1,
					nodeCount: nextData.nodes.length,
					edgeCount: nextData.edges.length,
				});
			});
		})();

		return () => {
			cancelled = true;
		};
	}, [data, layoutRunKey]);

	const theme = useMemo(() => {
		void themeTick;
		return getGraphTheme();
	}, [themeTick]);
	const hasRenderableData = Boolean(
		data && (data.nodes.length > 0 || data.edges.length > 0),
	);
	const preparedForCurrentData = Boolean(
		graph && preparedDataRef.current === data,
	);
	const useWorkerLayout = Boolean(
		graph &&
			shouldRunWorkerLayout &&
			shouldUseWorkerLayout(graph.order, graph.size),
	);

	const overlayState = useMemo<GraphLoadingOverlayState | null>(() => {
		const nodeCount = data?.nodes.length ?? preparationState.nodeCount;
		const edgeCount = data?.edges.length ?? preparationState.edgeCount;

		if (loading) {
			return {
				title: graph ? "Refreshing graph snapshot" : "Loading graph snapshot",
				detail: graph
					? "Keeping the current view visible while new graph data arrives."
					: "Fetching nodes and connections from the database.",
				progress: graph ? 0.16 : 0.08,
				nodeCount,
				edgeCount,
			};
		}

		if (
			preparationState.phase === "ready" &&
			!preparedForCurrentData &&
			hasRenderableData
		) {
			return {
				title: "Preparing graph scene",
				detail: "Updating the canvas without blocking the page.",
				progress: 0.01,
				nodeCount,
				edgeCount,
			};
		}

		if (preparationState.phase === "idle" && hasRenderableData) {
			return {
				title: "Preparing graph scene",
				detail: "Scheduling graph work so the page stays responsive.",
				progress: 0.01,
				nodeCount,
				edgeCount,
			};
		}

		if (
			preparationState.phase === "idle" ||
			preparationState.phase === "ready"
		) {
			return null;
		}

		return {
			title: preparationState.title,
			detail: preparationState.detail,
			progress: preparationState.progress,
			nodeCount: preparationState.nodeCount,
			edgeCount: preparationState.edgeCount,
		};
	}, [
		data,
		graph,
		hasRenderableData,
		loading,
		preparationState,
		preparedForCurrentData,
	]);

	const isBusy = overlayState !== null;
	const hasExistingGraph = Boolean(graph);
	// While a graph is already on screen, expansions and rebuilds must not dim
	// the whole canvas immediately — only after sustained loading.
	const shouldDelayOverlay = isBusy && hasExistingGraph;

	useEffect(() => {
		if (!isBusy) {
			setShowOverlay(false);
			return;
		}

		if (!shouldDelayOverlay) {
			setShowOverlay(true);
			return;
		}

		const timeoutId = window.setTimeout(() => {
			setShowOverlay(true);
		}, EXPANSION_OVERLAY_DELAY_MS);

		return () => {
			window.clearTimeout(timeoutId);
		};
	}, [isBusy, shouldDelayOverlay]);

	useEffect(() => {
		if (!graph) return;
		void themeTick;

		invalidateGraphTheme();

		const nodeColor = getDefaultNodeColor();
		const edgeHex = colorToHex(getDefaultEdgeColor());
		const edgeAlpha = getBaseEdgeAlpha(graph.order);

		graph.forEachNode((node) => {
			if (!graph.getNodeAttribute(node, "usesDefaultColor")) return;
			graph.setNodeAttribute(node, "color", nodeColor);
			graph.setNodeAttribute(node, "originalColor", nodeColor);
			graph.setNodeAttribute(node, "borderColor", nodeColor);
		});

		graph.forEachEdge((edge) => {
			if (!graph.getEdgeAttribute(edge, "usesDefaultColor")) return;
			graph.setEdgeAttribute(edge, "originalColor", edgeHex);
			graph.setEdgeAttribute(edge, "color", hexToRgba(edgeHex, edgeAlpha));
		});
	}, [graph, themeTick]);

	const handleResetLayout = useCallback(() => {
		if (!data || isBusy) return;
		const nextData = data;
		forceLayoutRef.current = true;

		setPreparationState({
			phase: "layout",
			title: "Restarting layout",
			detail: "Generating a fresh layout without blocking the page.",
			progress: 0.01,
			nodeCount: nextData.nodes.length,
			edgeCount: nextData.edges.length,
		});
		setLayoutRunKey((current) => current + 1);
	}, [data, isBusy]);

	const highlightRef = useRef<HighlightState>({
		hoveredNode: null,
		hoveredEdge: null,
		selectedNodeId: selectedNodeId ?? null,
		selectedEdgeKey: selectedEdgeKey ?? null,
		highlightedNodeIds,
		highlightedEdgeIds,
		hiddenLabels,
		neighborSet: null,
		connectedEdgeSet: null,
	});

	// Keep ref in sync with props (no re-render)
	highlightRef.current.selectedNodeId = selectedNodeId ?? null;
	highlightRef.current.selectedEdgeKey = selectedEdgeKey ?? null;
	highlightRef.current.highlightedNodeIds = highlightedNodeIds;
	highlightRef.current.highlightedEdgeIds = highlightedEdgeIds;
	highlightRef.current.hiddenLabels = hiddenLabels;

	// Recompute neighbor sets when selectedNodeId changes
	useEffect(() => {
		if (!graph) {
			highlightRef.current.neighborSet = null;
			highlightRef.current.connectedEdgeSet = null;
			return;
		}

		const activeNode = selectedNodeId ?? highlightRef.current.hoveredNode;
		const sets = computeNeighborSets(graph, activeNode);
		highlightRef.current.neighborSet = sets.neighborSet;
		highlightRef.current.connectedEdgeSet = sets.connectedEdgeSet;
	}, [selectedNodeId, graph]);

	// Force sigma refresh when visibility/highlight props change
	const sigmaRefreshKey = `${hiddenLabels ? [...hiddenLabels].join(",") : ""}_${highlightedNodeIds ? [...highlightedNodeIds].join(",") : ""}_${highlightedEdgeIds ? [...highlightedEdgeIds].join(",") : ""}_${selectedEdgeKey ?? ""}_${themeTick}`;

	// Stable reducers — read all dynamic state from ref
	const nodeReducer = useCallback(
		(node: string, attrs: Record<string, unknown>) => {
			const res = { ...attrs };
			const hl = highlightRef.current;

			if (!graph || !graph.hasNode(node)) return res;

			const nodeLabel = graph.getNodeAttribute(node, "nodeLabel") as string;
			const origColor = graph.getNodeAttribute(node, "originalColor") as string;

			if (hl.hiddenLabels?.has(nodeLabel)) {
				res.hidden = true;
				return res;
			}

			if (hl.highlightedNodeIds && hl.highlightedNodeIds.size > 0) {
				if (!hl.highlightedNodeIds.has(node)) {
					const dim = dimTowardBackground(origColor);
					res.color = dim;
					res.borderColor = dim;
					res.label = "";
					res.zIndex = 0;
					return res;
				}
				res.zIndex = 2;
				res.color = origColor;
				res.borderColor = origColor;
				return res;
			}

			const activeNode = hl.selectedNodeId ?? hl.hoveredNode;
			if (hl.neighborSet && activeNode) {
				if (node === activeNode) {
					res.zIndex = 3;
					res.highlighted = true;
					res.forceLabel = true;
					res.color = origColor;
					res.borderColor = origColor;
					const s = (res.size as number) ?? 10;
					res.size = s * 1.15;
				} else if (hl.neighborSet.has(node)) {
					res.zIndex = 2;
					res.color = origColor;
					res.borderColor = origColor;
				} else {
					const dim = dimTowardBackground(origColor);
					res.color = dim;
					res.borderColor = dim;
					res.label = "";
					res.zIndex = 0;
				}
			}
			return res;
		},
		[graph],
	);

	const edgeReducer = useCallback(
		(edge: string, attrs: Record<string, unknown>) => {
			const res = { ...attrs };
			const hl = highlightRef.current;

			if (!graph || !graph.hasEdge(edge)) return res;

			const [src, tgt] = graph.extremities(edge);
			const srcLabel = graph.getNodeAttribute(src, "nodeLabel") as string;
			const tgtLabel = graph.getNodeAttribute(tgt, "nodeLabel") as string;
			const storedLabel = res.label as string | undefined;
			if (
				hl.hiddenLabels?.has(srcLabel) ||
				hl.hiddenLabels?.has(tgtLabel) ||
				(storedLabel && hl.hiddenLabels?.has(storedLabel))
			) {
				res.hidden = true;
				return res;
			}

			const origColor = graph.getEdgeAttribute(edge, "originalColor") as string;
			const isHoveredEdge = hl.hoveredEdge === edge;

			if (hl.highlightedNodeIds && hl.highlightedNodeIds.size > 0) {
				const edgeId = graph.getEdgeAttribute(edge, "edgeId") as
					| string
					| undefined;
				const isHighlighted = hl.highlightedEdgeIds
					? Boolean(edgeId && hl.highlightedEdgeIds.has(edgeId))
					: hl.highlightedNodeIds.has(src) || hl.highlightedNodeIds.has(tgt);
				if (!isHighlighted) {
					res.color = hexToRgba(origColor, CONTEXT_DIM_EDGE_ALPHA);
					res.size = CONTEXT_DIM_EDGE_SIZE;
					res.zIndex = 0;
					res.label = undefined;
					res.forceLabel = false;
					return res;
				}
				res.color = hexToRgba(origColor, 0.5);
				res.size = 1;
				res.forceLabel = true;
				return res;
			}

			if (hl.neighborSet) {
				if (hl.connectedEdgeSet?.has(edge)) {
					const srcNodeColor = graph.getNodeAttribute(
						src,
						"originalColor",
					) as string;
					res.color = hexToRgba(srcNodeColor, 0.7);
					res.size = 1.5;
					res.zIndex = 1;
					res.label = undefined;
					res.forceLabel = false;
				} else {
					res.color = hexToRgba(origColor, CONTEXT_DIM_EDGE_ALPHA);
					res.size = CONTEXT_DIM_EDGE_SIZE;
					res.zIndex = 0;
					res.label = undefined;
					res.forceLabel = false;
				}
				if (isHoveredEdge) {
					res.label = storedLabel;
					res.forceLabel = true;
					res.size = 2;
					res.color = hexToRgba(origColor, 0.9);
					res.zIndex = 2;
				}
			} else if (isHoveredEdge) {
				res.forceLabel = true;
				res.size = 1.5;
				res.color = hexToRgba(origColor, 0.7);
				res.zIndex = 1;
			}

			if (hl.selectedEdgeKey) {
				const eid = graph.getEdgeAttribute(edge, "edgeId") as
					| string
					| undefined;
				if (eid === hl.selectedEdgeKey) {
					res.size = 2.5;
					res.zIndex = 2;
					res.color = hexToRgba(origColor, 0.9);
					res.label = storedLabel;
					res.forceLabel = true;
				}
			}

			return res;
		},
		[graph],
	);

	const sigmaSettings = useMemo(() => {
		void themeTick;
		const nodeCount = graph?.order ?? 0;
		const edgeCount = graph?.size ?? 0;
		const isHuge = nodeCount >= HUGE_THRESHOLD;
		const isLarge = nodeCount >= LARGE_THRESHOLD;
		const isDense = edgeCount / Math.max(1, nodeCount) > 3;

		const defaultNode = getDefaultNodeColor();
		const defaultEdgeHex = getDefaultEdgeColor();

		return {
			allowInvalidContainer: true,
			defaultNodeColor: defaultNode,
			defaultEdgeColor: hexToRgba(defaultEdgeHex, getBaseEdgeAlpha(nodeCount)),
			defaultNodeType: isLarge ? "circle" : "bordered-image",
			nodeProgramClasses: {
				"bordered-image": IconNodeProgram,
				circle: NodeCircleProgram,
			},
			defaultEdgeType: "arrow",
			edgeProgramClasses: {
				arrow: EdgeArrowProgram,
				curvedArrow: EdgeCurvedArrowProgram,
			},
			renderEdgeLabels: !isHuge,
			enableEdgeEvents: !isHuge,
			edgeLabelSize: 10,
			labelSize: isHuge ? 10 : isLarge ? 11 : 12,
			labelRenderedSizeThreshold: isHuge ? 18 : isLarge ? 12 : 6,
			labelDensity: isHuge ? 0.07 : isDense ? 0.25 : isLarge ? 0.35 : 0.5,
			labelGridCellSize: isHuge ? 300 : isDense ? 180 : isLarge ? 140 : 100,
			labelFont: "Inter, system-ui, sans-serif",
			labelWeight: "500",
			defaultDrawNodeLabel: drawNodeLabel,
			defaultDrawNodeHover: drawNodeHover,
			zIndex: !isHuge,
			nodeReducer: isHuge ? undefined : nodeReducer,
			edgeReducer: isHuge ? undefined : edgeReducer,
			stagePadding: isLarge ? 60 : 40,
			minCameraRatio: 0.01,
			maxCameraRatio: 20,
			autoRescale: true,
			autoCenter: true,
		};
	}, [graph, nodeReducer, edgeReducer, themeTick]);

	if (!graph && !hasRenderableData && !loading) {
		return (
			<div
				className={`relative flex h-full w-full items-center justify-center text-muted-foreground ${className ?? ""}`}
			>
				No graph data to display
			</div>
		);
	}

	const [bgR, bgG, bgB] = theme.bgRgb;

	return (
		<div className={`relative h-full w-full ${className ?? ""}`}>
			{graph ? (
				<SigmaContainer
					graph={graph}
					className="absolute inset-0"
					style={
						{
							height: "100%",
							width: "100%",
							"--sigma-background-color": `rgb(${bgR},${bgG},${bgB})`,
						} as React.CSSProperties
					}
					settings={sigmaSettings}
				>
					<GraphEvents
						onNodeClick={onNodeClick}
						onNodeShiftClick={onNodeShiftClick}
						onEdgeClick={onEdgeClick}
						onStageClick={onStageClick}
						highlightRef={highlightRef}
						graph={graph}
					/>
					{useWorkerLayout ? (
						<SigmaWorkerLayout
							enabled={useWorkerLayout}
							graphRevision={graphRevision}
							nodeCount={graph.order}
							edgeCount={graph.size}
							onRunningChange={setIsWorkerLayoutRunning}
						/>
					) : null}
					<SigmaRefresher refreshKey={sigmaRefreshKey} />
					<SigmaControls onResetLayout={handleResetLayout} disabled={isBusy} />
				</SigmaContainer>
			) : null}

			{isWorkerLayoutRunning ? (
				<div className="absolute left-3 top-3 z-10 flex items-center gap-2 rounded-full border bg-background/80 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur-sm">
					<LoaderCircle className="h-3.5 w-3.5 animate-spin text-primary" />
					<span>Refining layout</span>
				</div>
			) : null}

			{isBusy && hasExistingGraph && !showOverlay ? (
				<div className="absolute bottom-3 left-3 z-10 flex items-center gap-2 rounded-full border bg-background/80 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur-sm">
					<LoaderCircle className="h-3.5 w-3.5 animate-spin text-primary" />
					<span>{overlayState?.title ?? "Updating graph"}</span>
				</div>
			) : null}

			{showOverlay && overlayState ? (
				<GraphCanvasLoadingOverlay {...overlayState} dimmed={Boolean(graph)} />
			) : null}
		</div>
	);
}
