"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
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
	PinOff,
	RotateCcw,
	Square,
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
import type { ClusterModel } from "./graph-clusters";
import {
	type ConnectivityPartition,
	DEFAULT_NODE_SIZE,
	type GraphLayoutMode,
	type LayoutPosition as GraphPosition,
	applyClusterLayout,
	computeLabelExtents,
	computeSeedSpread,
	createAnchoredPosition,
	createDeterministicPosition,
	defaultRelaxIterations,
	getLayoutBounds,
	packNodesOnGrid,
	partitionByConnectivity,
	placeCircularLayout,
	placeDetachedNodes,
	placeHierarchyLayout,
	placeRadialLayout,
	relaxOverlaps,
} from "./graph-layout";
import { loadGraphScene, saveGraphScene } from "./graph-position-store";
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
		// The glyph is a type hint, not the subject. Filling most of the disc made
		// it read as the node itself, which is also what buried the colour that
		// actually distinguishes one object type from another.
		padding: 0.42,
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

/** An explicit arrangement request; a new `seq` applies it to the live scene. */
export interface GraphLayoutCommand {
	mode: GraphLayoutMode;
	seq: number;
	/** Anchor for the radial arrangement; ignored by the other modes. */
	centerNodeId?: string | null;
}

/** Imperative surface the canvas hands its host for pin management. */
export interface GraphCanvasApi {
	pinNode: (nodeId: string, pinned: boolean) => void;
	unpinAll: () => void;
	isPinned: (nodeId: string) => boolean;
}

export interface GraphCanvasProps {
	data: SubgraphResult | null;
	loading?: boolean;
	selectedNodeId?: string | null;
	selectedEdgeKey?: string | null;
	highlightedNodeIds?: Set<string>;
	highlightedEdgeIds?: Set<string>;
	hiddenLabels?: Set<string>;
	/**
	 * Restricts the stage to these nodes. Dimming stops paying off once the
	 * context is large enough to be the problem itself; this removes it instead.
	 */
	visibleNodeIds?: Set<string>;
	/** Groups the nodes into constellations instead of one undifferentiated field. */
	clusters?: ClusterModel | null;
	onNodeClick?: (nodeId: string) => void;
	onNodeShiftClick?: (nodeId: string, label: string) => void;
	/** Double-click, with sigma's double-click zoom suppressed. */
	onNodeDoubleClick?: (nodeId: string) => void;
	/** Right-click; the position is in container-relative viewport pixels. */
	onNodeContextMenu?: (
		nodeId: string,
		position: { x: number; y: number },
	) => void;
	onEdgeClick?: (edgeKey: string) => void;
	onStageClick?: () => void;
	/** Storage key under which positions and pins survive a reload. */
	persistKey?: string;
	/** Applies deterministic layouts to the live scene, animated. */
	layoutCommand?: GraphLayoutCommand | null;
	/** Hands the host the pin API once the canvas is interactive. */
	onCanvasApi?: (api: GraphCanvasApi | null) => void;
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

const NODE_SIZE_MIN = 5;
const NODE_SIZE_MAX = 18;

function styleToNodeSize(
	style?: LabelStyle,
	degree?: number,
	columnRanges?: ReadonlyMap<string, ColumnRange>,
	props?: Record<string, unknown>,
): number {
	if (!style?.size) return DEFAULT_NODE_SIZE;
	const { mode } = style.size;
	if (mode === "fixed")
		return Math.max(NODE_SIZE_MIN, style.size.value ?? DEFAULT_NODE_SIZE);
	if (mode === "by-degree" && degree !== undefined) {
		const min = style.size.min ?? NODE_SIZE_MIN;
		const max = style.size.max ?? NODE_SIZE_MAX;
		return Math.min(max, min + degree * 1.2);
	}
	if (mode === "by-column" && style.size.column) {
		const min = style.size.min ?? NODE_SIZE_MIN;
		const max = style.size.max ?? NODE_SIZE_MAX;
		const value = toFiniteNumber(props?.[style.size.column]);
		const range = columnRanges?.get(style.size.column);
		if (value === undefined || !range || range.max <= range.min) {
			return (min + max) / 2;
		}
		const ratio = (value - range.min) / (range.max - range.min);
		return min + ratio * (max - min);
	}
	return DEFAULT_NODE_SIZE;
}

const HUB_SIZE_MIN = 11;
const HUB_SIZE_MAX = 24;
/**
 * Hub captions exempted from label culling. A forced label bypasses
 * `labelRenderedSizeThreshold` and the label grid entirely, so forcing all 400
 * groups would undo exactly the budget the large tiers exist to enforce. Past
 * this the size encoding and sigma's own culling decide.
 */
const MAX_FORCED_HUB_LABELS = 48;

/**
 * Size carries fan-out, log-scaled so a corpus spanning orders of magnitude
 * still fits one screen. Sigma orders label candidates by size and culls below
 * `labelRenderedSizeThreshold`, so this doubles as the label-priority rule:
 * hub captions survive, member captions drop out.
 */
function hubNodeSize(represented: number): number {
	const scaled = HUB_SIZE_MIN + 1.5 * Math.log2(1 + Math.max(0, represented));
	return Math.min(HUB_SIZE_MAX, Math.max(HUB_SIZE_MIN, scaled));
}

/** Nodes a stage seats comfortably before circles start crowding the edges out. */
const SIZE_FIT_REFERENCE_NODES = 40;
const MIN_FIT_SIZE_SCALE = 0.28;
/** Stage the size ceiling is reckoned against, in CSS pixels. */
const REFERENCE_STAGE_AREA = 1200 * 700;
/** How much of the room a node gets its circle may fill, edge to edge. */
const MAX_NODE_PITCH_SHARE = 0.3;
const MIN_RENDERED_NODE_SIZE = 2;

/**
 * Shrinks nodes as the sample grows, because `autoRescale` fits the whole layout
 * to the stage while `size` stays in screen pixels.
 *
 * Extra spacing in the layout cannot fix this: the auto-fit divides it straight
 * back out, so the pitch a node gets on screen falls as `1/sqrt(nodeCount)`
 * whatever the layout does. Only the pixel size is ours to set, so it follows the
 * same curve — otherwise circles keep their pixels while the gaps between them
 * close, and past a hundred-odd nodes the edges disappear underneath them.
 */
function fitSizeScale(nodeCount: number): number {
	if (nodeCount <= SIZE_FIT_REFERENCE_NODES) return 1;
	return Math.max(
		MIN_FIT_SIZE_SCALE,
		Math.sqrt(SIZE_FIT_REFERENCE_NODES / nodeCount),
	);
}

/**
 * Hard ceiling on a rendered node, as a share of the room the sample leaves it.
 *
 * Scaling alone cannot promise this: an overlay is free to declare `size: 28`,
 * and a fraction of a large number is still large. The pitch — how far apart
 * auto-fit can hold two nodes — is what the reader actually has, so the ceiling
 * is a share of that and the declared size only matters below it.
 */
function maxNodeSize(nodeCount: number): number {
	const pitch = Math.sqrt(REFERENCE_STAGE_AREA / Math.max(1, nodeCount));
	return Math.max(MIN_RENDERED_NODE_SIZE, (pitch * MAX_NODE_PITCH_SHARE) / 2);
}

/**
 * Margin between the label cutoff and the nodes it judges, so a plain node still
 * clears the bar its own size was scaled against rather than landing exactly on it.
 */
const LABEL_THRESHOLD_HEADROOM = 0.8;

function hexToRgba(hex: string, alpha: number): string {
	const r = Number.parseInt(hex.slice(1, 3), 16);
	const g = Number.parseInt(hex.slice(3, 5), 16);
	const b = Number.parseInt(hex.slice(5, 7), 16);
	return `rgba(${r},${g},${b},${alpha})`;
}

function getBaseEdgeAlpha(nodeCount: number): number {
	if (nodeCount >= HUGE_THRESHOLD) return 0.08;
	if (nodeCount >= LARGE_THRESHOLD) return 0.22;
	return 0.5;
}

const CONTEXT_DIM_EDGE_SIZE = 0.75;
const CONTEXT_DIM_EDGE_ALPHA = 0.06;
const CONTEXT_DIM_NODE_AMOUNT = 0.88;
/** Dimmed nodes also shrink, so the focus reads as depth and not just as colour. */
const CONTEXT_DIM_NODE_SCALE = 0.75;

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
	//
	// `outboundAttractionDistribution` divides a hub's attraction across its
	// edges, so satellites spread around it instead of collapsing into it.
	return {
		gravity: isDense ? 0.5 : isSparse ? 0.6 : 1,
		scalingRatio: isDense ? 8 : isSparse ? 24 : 14,
		slowDown: isDense ? 5 : 8,
		barnesHutOptimize: nodeCount > 200,
		barnesHutTheta: 0.5,
		strongGravityMode: false,
		linLogMode: true,
		outboundAttractionDistribution: true,
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
	const labelExtents = computeLabelExtents(graph, connected);
	let completed = 0;

	while (completed < totalIterations) {
		if (isCancelled()) return;

		const batch = Math.min(batchIterations, totalIterations - completed);
		const performed = relaxOverlaps(graph, connected, {
			iterations: batch,
			labelExtents,
		});
		completed += batch;

		updateProgress?.(
			completed / totalIterations,
			i18next.t("separatingOverlappingNodes", "Separating overlapping nodes."),
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
			i18next.t(
				"valVal2LayoutPassesComplete",
				"{{val}} / {{val2}} layout passes complete.",
				{
					val: completedIterations.toLocaleString(),
					val2: totalIterations.toLocaleString(),
				},
			),
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
	clusters?: ClusterModel | null;
	/** Nodes the user placed by hand; they keep their spot through every layout. */
	pinnedIds?: ReadonlySet<string>;
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
	{
		previousPositions,
		anchorNodeId,
		forceLayout = false,
		clusters,
		pinnedIds,
	}: GraphBuildOptions,
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
	// Endpoint labels are stamped onto edges so the edge reducer never has to
	// walk back to the nodes — that walk is what made reducers unaffordable at
	// the large tiers.
	const labelById = new Map(data.nodes.map((node) => [node.id, node.label]));
	const columnRanges = computeColumnRanges(data.nodes);
	// Groups arrive ranked by population, so the biggest hubs keep their captions.
	const forcedHubIds = new Set(
		(clusters?.clusters ?? [])
			.map((cluster) => cluster.hubId)
			.filter((hubId): hubId is string => hubId !== undefined)
			.slice(0, MAX_FORCED_HUB_LABELS),
	);
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
		i18next.t("preparingGraphScene", "Preparing graph scene"),
		i18next.t(
			"schedulingGraphWorkSoThePageStaysResponsive",
			"Scheduling graph work so the page stays responsive.",
		),
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
				size: DEFAULT_NODE_SIZE,
				color: nodeColor,
				x: position.x,
				y: position.y,
				nodeLabel: node.label,
				originalColor: nodeColor,
				borderColor: nodeColor,
				usesDefaultColor: !node.style?.color,
			};

			if (pinnedIds?.has(node.id)) {
				attrs.pinned = true;
				// `fixed` is what the ForceAtlas2 kernels read to skip a node.
				attrs.fixed = true;
			}

			// Written at every graph size: above LARGE_THRESHOLD the reducers are
			// skipped entirely, so raw attributes are all the renderer still sees.
			const assignment = clusters?.byNode.get(node.id);
			if (assignment) {
				attrs.clusterId = assignment.clusterId;
				if (assignment.isHub) {
					attrs.isHub = true;
					attrs.badge = assignment.badge;
					attrs.forceLabel = forcedHubIds.has(node.id);
				}
			}

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
				i18next.t(
					"valOfNodeMetadataReady",
					"{{val}}% of node metadata ready.",
					{ val: Math.round(fraction * 100) },
				),
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
				srcLabel: labelById.get(edge.source),
				tgtLabel: labelById.get(edge.target),
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
				i18next.t("valOfEdgesConnected", "{{val}}% of edges connected.", {
					val: Math.round(fraction * 100),
				}),
			);
		},
		isCancelled,
	);

	if (!edgesBuilt || isCancelled()) return null;

	// Below the huge tier parallel edges get separated into curves; past it the
	// indexation cost buys nothing the faint straight lines would show anyway.
	if (!isHuge) {
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT,
			"Optimizing connections",
			i18next.t(
				"resolvingParallelEdgesForClearerPaths",
				"Resolving parallel edges for clearer paths.",
			),
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
	const fitScale = fitSizeScale(graph.order);
	const sizeCeiling = maxNodeSize(graph.order);
	const sized = await processInChunks(
		data.nodes,
		getNodeChunkSize(nodeCount),
		(node) => {
			if (!graph.hasNode(node.id)) return;

			const assignment = clusters?.byNode.get(node.id);
			const styledSize = styleToNodeSize(
				node.style,
				degreeMap.get(node.id),
				columnRanges,
				node.props,
			);
			// A hub is never smaller than its population suggests, but a user's own
			// sizing can still make it bigger — max() keeps both encodings honest.
			const baseSize = assignment?.isHub
				? Math.max(styledSize, hubNodeSize(assignment.represented))
				: styledSize;

			const scaledSize = Math.max(
				MIN_RENDERED_NODE_SIZE,
				Math.min(baseSize * fitScale * (density > 4 ? 0.85 : 1), sizeCeiling),
			);
			graph.setNodeAttribute(node.id, "size", scaledSize);
		},
		(fraction) => {
			publish(
				NODE_PROGRESS_WEIGHT +
					EDGE_PROGRESS_WEIGHT +
					SIZE_PROGRESS_WEIGHT * fraction,
				i18next.t("balancingNodeSizes", "Balancing node sizes"),
				i18next.t(
					"scalingNodesForReadability",
					"Scaling nodes for readability.",
				),
			);
		},
		isCancelled,
	);

	if (!sized || isCancelled()) return null;

	const partition = partitionByConnectivity(graph);

	if (preserveLayout) {
		// Expansions must not reshuffle the view, so only the stacking that the
		// anchored seeding introduced gets nudged apart.
		const preservedIds = Array.from(nodeIds);
		relaxOverlaps(graph, preservedIds, {
			iterations: PRESERVE_RELAX_ITERATIONS,
			labelExtents: computeLabelExtents(graph, preservedIds),
		});
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT,
			i18next.t("keepingLayoutStable", "Keeping layout stable"),
			`Reusing the current node positions while adding new connections.`,
			"ready",
		);
		return {
			graph,
			shouldRunWorkerLayout: false,
		};
	}

	// Grouping wins over the force layout when we have one: a simulation spreads
	// nodes by connectivity alone, which says nothing about a sample that is
	// mostly one object type or mostly detached.
	if (clusters && clusters.clusters.length > 1) {
		const base =
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT;
		publish(
			base,
			"Grouping nodes",
			i18next.t(
				"arrangingEachGroupAroundItsHub",
				"Arranging each group around its hub.",
			),
			"layout",
		);
		const clusterLabelExtents = computeLabelExtents(graph, [...nodeIds]);
		await applyClusterLayout(graph, clusters.clusters, {
			labelExtents: clusterLabelExtents,
			// On graphs small enough for dense captions the groups also get more
			// clearance and a gentle whole-stage pass against cross-group label hits.
			clusterGap: clusterLabelExtents ? 48 : undefined,
			globalRelaxIterations: clusterLabelExtents ? 10 : 0,
			onProgress: (fraction) => {
				publish(
					base + LAYOUT_PROGRESS_WEIGHT * fraction,
					"Grouping nodes",
					i18next.t("valOfGroupsArranged", "{{val}}% of groups arranged.", {
						val: Math.round(fraction * 100),
					}),
					"layout",
				);
			},
			yieldToFrame: waitForNextFrame,
			isCancelled,
		});
		if (isCancelled()) return null;
		publish(
			1,
			"Graph ready",
			i18next.t("renderingInteractiveView", "Rendering interactive view."),
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
			i18next.t("arrangingNodes", "Arranging nodes"),
			i18next.t(
				"noConnectionsToLayOutPlacingNodesOnAGrid",
				"No connections to lay out — placing nodes on a grid.",
			),
			"layout",
		);
		packNodesOnGrid(graph, [...partition.connected, ...partition.isolated]);
		publish(
			1,
			"Graph ready",
			i18next.t("renderingInteractiveView", "Rendering interactive view."),
			"ready",
		);
		return {
			graph,
			shouldRunWorkerLayout: false,
		};
	}

	if (shouldUseWorkerLayout(partition.connected.length, edgeCount)) {
		publish(
			NODE_PROGRESS_WEIGHT + EDGE_PROGRESS_WEIGHT + SIZE_PROGRESS_WEIGHT,
			i18next.t("preparingWorkerLayout", "Preparing worker layout"),
			i18next.t(
				"renderingTheGraphNowAndRefiningPositionsInTheBackground",
				"Rendering the graph now and refining positions in the background.",
			),
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

	publish(
		1,
		"Graph ready",
		i18next.t("renderingInteractiveView", "Rendering interactive view."),
		"ready",
	);
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
	visibleNodeIds: Set<string> | undefined;
	neighborSet: Set<string> | null;
	connectedEdgeSet: Set<string> | null;
	/** The few neighbours whose captions are worth forcing — see the cap below. */
	labeledNeighborSet: Set<string> | null;
	/** >1 when a filter shrank the visible set: survivors get the freed room. */
	visibleBoost: number;
	visibleSizeCap: number;
}

/**
 * Forcing a caption bypasses both the density grid and the size threshold, so
 * forcing every neighbour of a 200-degree hub is a wall of text the culling
 * exists to prevent. Only the biggest few earn it; the rest still compete in
 * the grid like everyone else.
 */
const MAX_LABELED_NEIGHBORS = 10;

function computeNeighborSets(
	graph: Graph,
	activeNode: string | null,
): {
	neighborSet: Set<string> | null;
	connectedEdgeSet: Set<string> | null;
	labeledNeighborSet: Set<string> | null;
} {
	if (!activeNode || !graph.hasNode(activeNode))
		return {
			neighborSet: null,
			connectedEdgeSet: null,
			labeledNeighborSet: null,
		};
	const neighborSet = new Set<string>([activeNode]);
	for (const neighbor of graph.neighbors(activeNode)) {
		neighborSet.add(neighbor);
	}
	const connectedEdgeSet = new Set<string>();
	for (const edge of graph.edges(activeNode)) {
		connectedEdgeSet.add(edge);
	}

	const labeledNeighborSet = new Set<string>([activeNode]);
	const ranked = [...neighborSet]
		.filter((nodeId) => nodeId !== activeNode)
		.sort((a, b) => {
			const sizeA = (graph.getNodeAttribute(a, "size") as number) ?? 0;
			const sizeB = (graph.getNodeAttribute(b, "size") as number) ?? 0;
			return sizeB - sizeA || (a < b ? -1 : 1);
		});
	for (const nodeId of ranked.slice(0, MAX_LABELED_NEIGHBORS)) {
		labeledNeighborSet.add(nodeId);
	}

	return { neighborSet, connectedEdgeSet, labeledNeighborSet };
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
	const { t } = useTranslation("common");
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
								{t("nodes2", "Nodes")}
							</p>
							<p className="mt-1 text-lg font-semibold">
								{formatOverlayMetric(nodeCount)}
							</p>
						</div>
						<div className="rounded-2xl border bg-background/70 p-3">
							<p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
								{t("edges", "Edges")}
							</p>
							<p className="mt-1 text-lg font-semibold">
								{formatOverlayMetric(edgeCount)}
							</p>
						</div>
						<div className="col-span-2 rounded-2xl border bg-background/70 p-3 sm:col-span-1">
							<p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
								{t("progress2", "Progress")}
							</p>
							<p className="mt-1 text-lg font-semibold">{`${progressPercent}%`}</p>
						</div>
					</div>

					<div className="space-y-2">
						<div className="flex items-center justify-between text-xs text-muted-foreground">
							<span>
								{t(
									"preparingInteractiveCanvas",
									"Preparing interactive canvas",
								)}
							</span>
							<span>{`${progressPercent}%`}</span>
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

/** Viewport pixels a press may wander before it counts as a drag, not a click. */
const DRAG_START_THRESHOLD = 3;

function GraphEvents({
	onNodeClick,
	onNodeShiftClick,
	onNodeDoubleClick,
	onNodeContextMenu,
	onEdgeClick,
	onStageClick,
	onNodeDragged,
	highlightRef,
	graph,
}: {
	onNodeClick?: (nodeId: string) => void;
	onNodeShiftClick?: (nodeId: string, label: string) => void;
	onNodeDoubleClick?: (nodeId: string) => void;
	onNodeContextMenu?: (
		nodeId: string,
		position: { x: number; y: number },
	) => void;
	onEdgeClick?: (edgeKey: string) => void;
	onStageClick?: () => void;
	/** Fires when a drag ends with real movement; the node arrives pinned. */
	onNodeDragged?: (nodeId: string) => void;
	highlightRef: React.MutableRefObject<HighlightState>;
	graph: Graph;
}) {
	const sigma = useSigma();
	const registerEvents = useRegisterEvents();
	const dragRef = useRef<{
		node: string;
		startX: number;
		startY: number;
		moved: boolean;
	} | null>(null);

	// A fresh graph deserves a fresh framing: the custom bbox a drag froze must
	// not survive into the next dataset.
	useEffect(() => {
		void graph;
		try {
			sigma.setCustomBBox(null);
		} catch {
			// The instance may be mid-teardown.
		}
	}, [graph, sigma]);

	useEffect(() => {
		const applyNeighborSets = (activeNode: string | null) => {
			const sets = computeNeighborSets(graph, activeNode);
			highlightRef.current.neighborSet = sets.neighborSet;
			highlightRef.current.connectedEdgeSet = sets.connectedEdgeSet;
			highlightRef.current.labeledNeighborSet = sets.labeledNeighborSet;
		};

		registerEvents({
			enterNode: ({ node }) => {
				highlightRef.current.hoveredNode = node;
				highlightRef.current.hoveredEdge = null;
				applyNeighborSets(highlightRef.current.selectedNodeId ?? node);
				sigma.refresh({ skipIndexation: true });
			},
			leaveNode: () => {
				highlightRef.current.hoveredNode = null;
				applyNeighborSets(highlightRef.current.selectedNodeId);
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
			downNode: ({ node, event }) => {
				const original = event.original;
				if (
					!(original instanceof MouseEvent) ||
					original.button !== 0 ||
					original.shiftKey
				) {
					return;
				}
				dragRef.current = {
					node,
					startX: event.x,
					startY: event.y,
					moved: false,
				};
			},
			mousemovebody: (event) => {
				const drag = dragRef.current;
				if (!drag) return;
				if (
					!drag.moved &&
					Math.hypot(event.x - drag.startX, event.y - drag.startY) <
						DRAG_START_THRESHOLD
				) {
					return;
				}
				// `autoRescale` re-fits the stage to the bbox, so without freezing it a
				// dragged node would drag the whole framing along with it.
				if (!sigma.getCustomBBox()) sigma.setCustomBBox(sigma.getBBox());
				drag.moved = true;
				const position = sigma.viewportToGraph({ x: event.x, y: event.y });
				graph.setNodeAttribute(drag.node, "x", position.x);
				graph.setNodeAttribute(drag.node, "y", position.y);
				event.preventSigmaDefault();
				event.original.preventDefault();
				event.original.stopPropagation();
			},
			mouseup: () => {
				const drag = dragRef.current;
				if (!drag) return;
				dragRef.current = null;
				if (!drag.moved) return;
				// A hand-placed node stays where it was put: pinned for the relax pass,
				// fixed for the force simulation.
				graph.setNodeAttribute(drag.node, "pinned", true);
				graph.setNodeAttribute(drag.node, "fixed", true);
				onNodeDragged?.(drag.node);
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
			doubleClickNode: ({ node, event }) => {
				if (!onNodeDoubleClick) return;
				event.preventSigmaDefault();
				onNodeDoubleClick(node);
			},
			rightClickNode: ({ node, event }) => {
				if (!onNodeContextMenu) return;
				event.preventSigmaDefault();
				event.original.preventDefault();
				onNodeContextMenu(node, { x: event.x, y: event.y });
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
		onNodeDoubleClick,
		onNodeContextMenu,
		onEdgeClick,
		onStageClick,
		onNodeDragged,
		highlightRef,
	]);

	return null;
}

function SigmaRefresher({ refreshKey }: { refreshKey: readonly unknown[] }) {
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

/** How often the background layout is checked for having settled, in ms. */
const WORKER_CONVERGENCE_INTERVAL_MS = 400;
/** Mean per-node movement between checks below which the layout counts as settled. */
const WORKER_CONVERGENCE_EPSILON = 0.35;
/** Consecutive settled checks required — one quiet sample can be a coincidence. */
const WORKER_CONVERGENCE_STREAK = 2;
/** Nodes sampled per check; a spread sample tracks the whole graph closely enough. */
const WORKER_CONVERGENCE_SAMPLE = 150;
/** The old wall-clock stop survives as a backstop, no longer as the stop rule. */
const WORKER_MAX_DURATION_FACTOR = 5;

function SigmaWorkerLayout({
	enabled,
	graphRevision,
	nodeCount,
	edgeCount,
	stopToken,
	onRunningChange,
}: {
	enabled: boolean;
	graphRevision: number;
	nodeCount: number;
	edgeCount: number;
	/** Bumping this asks the running layout to stop and settle now. */
	stopToken: number;
	onRunningChange?: (running: boolean) => void;
}) {
	const sigma = useSigma();
	const stopFnRef = useRef<(() => void) | null>(null);
	const layoutSettings = useMemo(
		() => getFA2Settings(nodeCount, edgeCount),
		[nodeCount, edgeCount],
	);
	const duration = useMemo(
		() => getWorkerLayoutDuration(nodeCount, edgeCount),
		[nodeCount, edgeCount],
	);

	useEffect(() => {
		if (stopToken > 0) stopFnRef.current?.();
	}, [stopToken]);

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
		let finished = false;
		let frameId = 0;
		let intervalId = 0;
		let maxTimeoutId = 0;

		const refreshLoop = () => {
			if (disposed) return;
			try {
				sigma.refresh({ skipIndexation: true });
			} catch {
				// WebGL context may be lost during background layout updates
			}
			frameId = window.requestAnimationFrame(refreshLoop);
		};

		// The layout stops when it has measurably settled instead of on a fixed
		// timer: sampled positions are compared between checks, and a graph whose
		// nodes have stopped moving is done arranging itself. On a big graph the
		// old timer routinely cut the untangling off mid-swing.
		const sampleStep = Math.max(
			1,
			Math.floor(currentGraph.order / WORKER_CONVERGENCE_SAMPLE),
		);
		let previousSample: Float64Array | null = null;
		let settledStreak = 0;
		const startedAt = Date.now();

		const takeSample = (): Float64Array => {
			const sample = new Float64Array(
				(Math.floor((currentGraph.order - 1) / sampleStep) + 1) * 2,
			);
			let cursor = 0;
			let index = 0;
			currentGraph.forEachNode((nodeId) => {
				if (index % sampleStep === 0 && cursor < sample.length) {
					sample[cursor] = currentGraph.getNodeAttribute(nodeId, "x") as number;
					sample[cursor + 1] = currentGraph.getNodeAttribute(
						nodeId,
						"y",
					) as number;
					cursor += 2;
				}
				index += 1;
			});
			return sample;
		};

		const finish = () => {
			if (disposed || finished) return;
			finished = true;
			stopFnRef.current = null;
			window.clearInterval(intervalId);
			window.clearTimeout(maxTimeoutId);
			window.cancelAnimationFrame(frameId);
			try {
				layout.stop();
			} catch {
				// Layout worker may already be disposed
			}
			onRunningChange?.(false);
			// Settle leftover overlap in rAF-sized batches — a synchronous pass over
			// a large graph would drop frames.
			const refresh = () => {
				try {
					sigma.refresh({ skipIndexation: true });
				} catch {
					// WebGL context may be lost during layout updates
				}
			};
			void finishLayoutAsync(
				currentGraph,
				partitionByConnectivity(currentGraph),
				() => disposed,
				refresh,
			).then(() => {
				if (!disposed) refresh();
			});
		};
		stopFnRef.current = finish;

		try {
			layout.start();
			onRunningChange?.(true);
			frameId = window.requestAnimationFrame(refreshLoop);
		} catch {
			try {
				layout.kill();
			} catch {
				// Layout worker may already be disposed
			}
			return;
		}

		intervalId = window.setInterval(() => {
			if (disposed || finished) return;
			const sample = takeSample();
			if (previousSample && previousSample.length === sample.length) {
				let total = 0;
				for (let index = 0; index < sample.length; index += 2) {
					total += Math.hypot(
						sample[index] - previousSample[index],
						sample[index + 1] - previousSample[index + 1],
					);
				}
				const meanMovement = total / Math.max(1, sample.length / 2);
				settledStreak =
					meanMovement < WORKER_CONVERGENCE_EPSILON ? settledStreak + 1 : 0;
				// The minimum runtime keeps a briefly-quiet start from ending the
				// layout before it has begun pulling anything apart.
				if (
					settledStreak >= WORKER_CONVERGENCE_STREAK &&
					Date.now() - startedAt >= duration
				) {
					finish();
					return;
				}
			}
			previousSample = sample;
		}, WORKER_CONVERGENCE_INTERVAL_MS);

		maxTimeoutId = window.setTimeout(
			finish,
			duration * WORKER_MAX_DURATION_FACTOR,
		);

		return () => {
			disposed = true;
			stopFnRef.current = null;
			window.clearInterval(intervalId);
			window.clearTimeout(maxTimeoutId);
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

/** How long an explicit layout change animates. Long enough to track, short enough to not wait for. */
const LAYOUT_ANIMATION_MS = 450;

function easeOutCubic(t: number): number {
	return 1 - (1 - t) ** 3;
}

/**
 * Applies an explicit arrangement to the live scene and animates every node to
 * its new position — the reader keeps their mental map because they can watch
 * the old picture become the new one.
 */
function SigmaLayoutApplier({
	command,
	onApplied,
}: {
	command: GraphLayoutCommand | null;
	onApplied?: () => void;
}) {
	const sigma = useSigma();
	const lastSeqRef = useRef(0);
	const animationRef = useRef(0);

	useEffect(() => {
		if (
			!command ||
			command.mode === "auto" ||
			command.seq === lastSeqRef.current
		)
			return;
		lastSeqRef.current = command.seq;

		const graph = sigma.getGraph();
		if (graph.order === 0) return;

		const before = snapshotGraphPositions(graph);
		const partition = partitionByConnectivity(graph);
		const coreBounds = () => getLayoutBounds(graph, partition.connected);

		switch (command.mode) {
			case "grid":
				packNodesOnGrid(graph, [...partition.connected, ...partition.isolated]);
				break;
			case "circular":
				placeCircularLayout(graph, partition.connected);
				placeDetachedNodes(graph, partition.isolated, coreBounds());
				break;
			case "radial":
				placeRadialLayout(graph, partition.connected, {
					centerId: command.centerNodeId ?? null,
				});
				placeDetachedNodes(graph, partition.isolated, coreBounds());
				break;
			case "hierarchy":
				placeHierarchyLayout(graph, partition.connected);
				placeDetachedNodes(graph, partition.isolated, coreBounds());
				break;
			case "force": {
				// A synchronous run with a bounded budget: an explicit command should
				// land as one movement, not as a background simmer.
				const iterations =
					graph.order <= 300 ? 300 : graph.order <= 1500 ? 150 : 60;
				forceAtlas2.assign(graph, {
					iterations,
					settings: getFA2Settings(partition.connected.length, graph.size),
				});
				relaxOverlaps(graph, partition.connected, {
					labelExtents: computeLabelExtents(graph, partition.connected),
				});
				placeDetachedNodes(graph, partition.isolated, coreBounds());
				break;
			}
		}

		if (
			command.mode === "circular" ||
			command.mode === "radial" ||
			command.mode === "hierarchy"
		) {
			const labelExtents = computeLabelExtents(graph, partition.connected);
			if (labelExtents) {
				relaxOverlaps(graph, partition.connected, {
					iterations: 6,
					labelExtents,
				});
			}
		}

		const after = snapshotGraphPositions(graph);
		for (const [nodeId, position] of before) {
			if (!graph.hasNode(nodeId)) continue;
			graph.setNodeAttribute(nodeId, "x", position.x);
			graph.setNodeAttribute(nodeId, "y", position.y);
		}
		try {
			// A drag may have frozen the framing; a new arrangement needs a new fit.
			sigma.setCustomBBox(null);
		} catch {
			// The instance may be mid-teardown.
		}

		const startedAt = performance.now();
		window.cancelAnimationFrame(animationRef.current);
		const step = (now: number) => {
			const t = Math.min(1, (now - startedAt) / LAYOUT_ANIMATION_MS);
			const eased = easeOutCubic(t);
			for (const [nodeId, target] of after) {
				if (!graph.hasNode(nodeId)) continue;
				const from = before.get(nodeId) ?? target;
				graph.setNodeAttribute(
					nodeId,
					"x",
					from.x + (target.x - from.x) * eased,
				);
				graph.setNodeAttribute(
					nodeId,
					"y",
					from.y + (target.y - from.y) * eased,
				);
			}
			try {
				sigma.refresh({ skipIndexation: true });
			} catch {
				// WebGL context may be lost during the animation
			}
			if (t < 1) {
				animationRef.current = window.requestAnimationFrame(step);
			} else {
				try {
					sigma.refresh();
				} catch {
					// WebGL context may be lost during the final refresh
				}
				onApplied?.();
			}
		};
		animationRef.current = window.requestAnimationFrame(step);
	}, [command, sigma, onApplied]);

	useEffect(() => () => window.cancelAnimationFrame(animationRef.current), []);

	return null;
}

function SigmaControls({
	onResetLayout,
	onUnpinAll,
	pinnedCount,
	disabled,
}: {
	onResetLayout: () => void;
	onUnpinAll?: () => void;
	pinnedCount?: number;
	disabled?: boolean;
}) {
	const { t } = useTranslation("common");
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
				title={t("zoomIn", "Zoom in")}
				disabled={disabled}
			>
				<ZoomIn className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={handleZoomOut}
				title={t("zoomOut", "Zoom out")}
				disabled={disabled}
			>
				<ZoomOut className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={handleFitView}
				title={t("fitToView", "Fit to view")}
				disabled={disabled}
			>
				<Maximize className="h-4 w-4" />
			</button>
			<button
				type="button"
				className={buttonClassName}
				onClick={onResetLayout}
				title={t("rerunLayout", "Re-run layout")}
				disabled={disabled}
			>
				<RotateCcw className="h-4 w-4" />
			</button>
			{onUnpinAll && (pinnedCount ?? 0) > 0 && (
				<button
					type="button"
					className={buttonClassName}
					onClick={onUnpinAll}
					title={t("unpinAllNodes", "Unpin all nodes ({{count}})", {
						count: pinnedCount,
					})}
					disabled={disabled}
				>
					<PinOff className="h-4 w-4" />
				</button>
			)}
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
	visibleNodeIds,
	clusters,
	onNodeClick,
	onNodeShiftClick,
	onNodeDoubleClick,
	onNodeContextMenu,
	onEdgeClick,
	onStageClick,
	persistKey,
	layoutCommand,
	onCanvasApi,
	className,
}: GraphCanvasProps) {
	const { t } = useTranslation("common");
	const [themeTick, setThemeTick] = useState(0);
	const [layoutRunKey, setLayoutRunKey] = useState(0);
	const [graph, setGraph] = useState<Graph | null>(null);
	const [graphRevision, setGraphRevision] = useState(0);
	const [shouldRunWorkerLayout, setShouldRunWorkerLayout] = useState(false);
	const [preparationState, setPreparationState] =
		useState<GraphPreparationState>(IDLE_PREPARATION_STATE);
	const [showOverlay, setShowOverlay] = useState(false);
	const [isWorkerLayoutRunning, setIsWorkerLayoutRunning] = useState(false);
	const [workerStopToken, setWorkerStopToken] = useState(0);
	const [pinnedCount, setPinnedCount] = useState(0);
	const preparedDataRef = useRef<SubgraphResult | null>(null);
	const graphRef = useRef<Graph | null>(graph);
	const loadingRef = useRef(loading);
	const selectedNodeIdRef = useRef<string | null>(selectedNodeId ?? null);
	const forceLayoutRef = useRef(false);
	const lastClusterEpochRef = useRef<string | null>(null);
	const lastPaletteKeyRef = useRef<string>("");
	const pinnedNodesRef = useRef<Set<string>>(new Set());
	const storedSceneAppliedRef = useRef(false);
	const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const lastAutoLayoutSeqRef = useRef(0);

	graphRef.current = graph;
	loadingRef.current = loading;
	selectedNodeIdRef.current = selectedNodeId ?? null;

	// Loaded once per storage key; hydrates both positions and pins so a scene a
	// reader arranged yesterday comes back arranged.
	const storedScene = useMemo(() => {
		storedSceneAppliedRef.current = false;
		return persistKey ? loadGraphScene(persistKey) : null;
	}, [persistKey]);

	useEffect(() => {
		if (!storedScene) return;
		pinnedNodesRef.current = new Set(storedScene.pinned);
		setPinnedCount(storedScene.pinned.size);
	}, [storedScene]);

	const scheduleSceneSave = useCallback(() => {
		if (!persistKey) return;
		if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
		saveTimerRef.current = setTimeout(() => {
			saveTimerRef.current = null;
			const currentGraph = graphRef.current;
			if (currentGraph) {
				saveGraphScene(persistKey, currentGraph, pinnedNodesRef.current);
			}
		}, 1000);
	}, [persistKey]);

	useEffect(
		() => () => {
			// Flush a pending save on unmount so the arrangement survives navigation.
			if (saveTimerRef.current) {
				clearTimeout(saveTimerRef.current);
				saveTimerRef.current = null;
				if (persistKey && graphRef.current) {
					saveGraphScene(persistKey, graphRef.current, pinnedNodesRef.current);
				}
			}
		},
		[persistKey],
	);

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
		let previousPositions = snapshotGraphPositions(graphRef.current);
		// The first build of a persisted scene starts from the stored arrangement
		// instead of a fresh layout — the preserve path then keeps it.
		let restoredScene = false;
		if (
			previousPositions.size === 0 &&
			storedScene &&
			!storedSceneAppliedRef.current
		) {
			storedSceneAppliedRef.current = true;
			previousPositions = storedScene.positions;
			restoredScene = true;
		}
		// A regrouping has to relayout. Raising the node limit keeps enough old
		// positions to clear the preserve threshold, which would otherwise swallow
		// the new grouping and leave a stale-but-plausible arrangement on screen.
		// The one exception is the build that restored a stored scene: its epoch
		// change is bookkeeping, not a regroup, and forcing a layout there would
		// discard the arrangement the reader saved.
		const clusterEpoch = clusters?.epoch ?? null;
		const regrouped = clusterEpoch !== lastClusterEpochRef.current;
		lastClusterEpochRef.current = clusterEpoch;
		const forceLayout = forceLayoutRef.current || (regrouped && !restoredScene);
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
			title: t("preparingGraphScene", "Preparing graph scene"),
			detail: t(
				"schedulingGraphWorkSoThePageStaysResponsive",
				"Scheduling graph work so the page stays responsive.",
			),
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
					clusters,
					pinnedIds: pinnedNodesRef.current,
				},
			);

			if (!buildResult || cancelled) return;

			// A pin on a node the new sample no longer contains is dead weight.
			if (pinnedNodesRef.current.size > 0) {
				const survivors = new Set<string>();
				for (const nodeId of pinnedNodesRef.current) {
					if (buildResult.graph.hasNode(nodeId)) survivors.add(nodeId);
				}
				if (survivors.size !== pinnedNodesRef.current.size) {
					pinnedNodesRef.current = survivors;
					setPinnedCount(survivors.size);
				}
			}

			preparedDataRef.current = nextData;
			startTransition(() => {
				setGraph(buildResult.graph);
				setGraphRevision((current) => current + 1);
				setShouldRunWorkerLayout(buildResult.shouldRunWorkerLayout);
				setIsWorkerLayoutRunning(false);
				setPreparationState({
					phase: "ready",
					title: t("graphReady", "Graph ready"),
					detail: t("interactiveViewUpdated", "Interactive view updated."),
					progress: 1,
					nodeCount: nextData.nodes.length,
					edgeCount: nextData.edges.length,
				});
			});
		})();

		return () => {
			cancelled = true;
		};
	}, [data, layoutRunKey, clusters, storedScene]);

	// Every settled build is a scene worth remembering.
	useEffect(() => {
		if (graphRevision > 0) scheduleSceneSave();
	}, [graphRevision, scheduleSceneSave]);

	// The background layout keeps refining after the build's save; capture where
	// it actually settled.
	useEffect(() => {
		if (!isWorkerLayoutRunning) scheduleSceneSave();
	}, [isWorkerLayoutRunning, scheduleSceneSave]);

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
				title: graph
					? t("refreshingGraphSnapshot", "Refreshing graph snapshot")
					: t("loadingGraphSnapshot", "Loading graph snapshot"),
				detail: graph
					? `Keeping the current view visible while new graph data arrives.`
					: `Fetching nodes and connections from the database.`,
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
				title: t("preparingGraphScene", "Preparing graph scene"),
				detail: t(
					"updatingTheCanvasWithoutBlockingThePage",
					"Updating the canvas without blocking the page.",
				),
				progress: 0.01,
				nodeCount,
				edgeCount,
			};
		}

		if (preparationState.phase === "idle" && hasRenderableData) {
			return {
				title: t("preparingGraphScene", "Preparing graph scene"),
				detail: t(
					"schedulingGraphWorkSoThePageStaysResponsive",
					"Scheduling graph work so the page stays responsive.",
				),
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
			title: t("restartingLayout", "Restarting layout"),
			detail: t(
				"generatingAFreshLayoutWithoutBlockingThePage",
				"Generating a fresh layout without blocking the page.",
			),
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
		visibleNodeIds,
		neighborSet: null,
		connectedEdgeSet: null,
		labeledNeighborSet: null,
		visibleBoost: 1,
		visibleSizeCap: Number.POSITIVE_INFINITY,
	});

	// Keep ref in sync with props (no re-render)
	highlightRef.current.selectedNodeId = selectedNodeId ?? null;
	highlightRef.current.selectedEdgeKey = selectedEdgeKey ?? null;
	highlightRef.current.highlightedNodeIds = highlightedNodeIds;
	highlightRef.current.highlightedEdgeIds = highlightedEdgeIds;
	highlightRef.current.hiddenLabels = hiddenLabels;
	highlightRef.current.visibleNodeIds = visibleNodeIds;

	// Synced during render, not in an effect: the refresh that reads these runs
	// from a child effect, which fires before this component's own effects would.
	// When a filter leaves a fraction of the sample visible, the survivors get
	// back the pixel size the full sample's fit had taken from them.
	if (
		graph &&
		visibleNodeIds &&
		visibleNodeIds.size > 0 &&
		visibleNodeIds.size < graph.order
	) {
		highlightRef.current.visibleBoost = Math.max(
			1,
			Math.min(
				3,
				fitSizeScale(visibleNodeIds.size) / fitSizeScale(graph.order),
			),
		);
		highlightRef.current.visibleSizeCap = maxNodeSize(visibleNodeIds.size);
	} else {
		highlightRef.current.visibleBoost = 1;
		highlightRef.current.visibleSizeCap = Number.POSITIVE_INFINITY;
	}

	// Recompute neighbor sets when selectedNodeId changes
	useEffect(() => {
		if (!graph) {
			highlightRef.current.neighborSet = null;
			highlightRef.current.connectedEdgeSet = null;
			highlightRef.current.labeledNeighborSet = null;
			return;
		}

		const activeNode = selectedNodeId ?? highlightRef.current.hoveredNode;
		const sets = computeNeighborSets(graph, activeNode);
		highlightRef.current.neighborSet = sets.neighborSet;
		highlightRef.current.connectedEdgeSet = sets.connectedEdgeSet;
		highlightRef.current.labeledNeighborSet = sets.labeledNeighborSet;
	}, [selectedNodeId, graph]);

	// Pin management, exposed to the host for context-menu actions.
	const handleNodeDragged = useCallback(
		(nodeId: string) => {
			pinnedNodesRef.current.add(nodeId);
			setPinnedCount(pinnedNodesRef.current.size);
			scheduleSceneSave();
		},
		[scheduleSceneSave],
	);

	const setNodePinned = useCallback(
		(nodeId: string, pinned: boolean) => {
			const currentGraph = graphRef.current;
			if (!currentGraph || !currentGraph.hasNode(nodeId)) return;
			currentGraph.setNodeAttribute(nodeId, "pinned", pinned);
			currentGraph.setNodeAttribute(nodeId, "fixed", pinned);
			if (pinned) pinnedNodesRef.current.add(nodeId);
			else pinnedNodesRef.current.delete(nodeId);
			setPinnedCount(pinnedNodesRef.current.size);
			scheduleSceneSave();
		},
		[scheduleSceneSave],
	);

	const handleUnpinAll = useCallback(() => {
		const currentGraph = graphRef.current;
		if (currentGraph) {
			for (const nodeId of pinnedNodesRef.current) {
				if (!currentGraph.hasNode(nodeId)) continue;
				currentGraph.setNodeAttribute(nodeId, "pinned", false);
				currentGraph.setNodeAttribute(nodeId, "fixed", false);
			}
		}
		pinnedNodesRef.current.clear();
		setPinnedCount(0);
		scheduleSceneSave();
	}, [scheduleSceneSave]);

	useEffect(() => {
		if (!onCanvasApi) return;
		onCanvasApi({
			pinNode: setNodePinned,
			unpinAll: handleUnpinAll,
			isPinned: (nodeId: string) => pinnedNodesRef.current.has(nodeId),
		});
		return () => onCanvasApi(null);
	}, [onCanvasApi, setNodePinned, handleUnpinAll]);

	// An explicit "auto" layout request is the reset gesture from the host.
	useEffect(() => {
		if (
			!layoutCommand ||
			layoutCommand.mode !== "auto" ||
			layoutCommand.seq === lastAutoLayoutSeqRef.current
		) {
			return;
		}
		lastAutoLayoutSeqRef.current = layoutCommand.seq;
		handleResetLayout();
	}, [layoutCommand, handleResetLayout]);

	// Sigma renders from mutable graph attributes, so a prop that only feeds the
	// reducers needs an explicit refresh. This tuple's IDENTITY is the trigger —
	// serialising the sets rebuilt a ~300KB key per render on a 10k-node stage, and
	// every distinct key ran a full re-indexation. Safe because every set arrives
	// from a useMemo or useState in GraphViewer, the only consumer; a set allocated
	// fresh each render would refresh each render.
	const sigmaRefreshTrigger = useMemo(
		() => [
			hiddenLabels,
			highlightedNodeIds,
			highlightedEdgeIds,
			visibleNodeIds,
			selectedEdgeKey,
			themeTick,
		],
		[
			hiddenLabels,
			highlightedNodeIds,
			highlightedEdgeIds,
			visibleNodeIds,
			selectedEdgeKey,
			themeTick,
		],
	);

	// Stable reducers — read all dynamic state from the ref, and everything about
	// the element from its own attributes: attribute-only reducers are what makes
	// keeping them enabled affordable at the huge tier, where they used to be
	// switched off (silently killing legend toggles, focus and search highlight).
	const nodeReducer = useCallback(
		(node: string, attrs: Record<string, unknown>) => {
			const res = { ...attrs };
			const hl = highlightRef.current;

			const nodeLabel = attrs.nodeLabel as string;
			const origColor = attrs.originalColor as string;

			if (hl.hiddenLabels?.has(nodeLabel)) {
				res.hidden = true;
				return res;
			}

			if (hl.visibleNodeIds && !hl.visibleNodeIds.has(node)) {
				res.hidden = true;
				return res;
			}

			if (hl.visibleBoost > 1) {
				res.size = Math.min(
					((res.size as number) ?? DEFAULT_NODE_SIZE) * hl.visibleBoost,
					hl.visibleSizeCap,
				);
			}

			// A hand-pinned node wears a contrasting ring, so "why is this one not
			// moving" always has a visible answer.
			if (attrs.pinned === true) {
				const [fgR, fgG, fgB] = getGraphTheme().fgRgb;
				res.borderColor = `rgb(${fgR},${fgG},${fgB})`;
			}

			// Dropping back to a plain circle is what makes dimming visible at all:
			// the icon program draws the white glyph over the disc at full opacity,
			// so a node whose colour was faded still reads as bright and in focus.
			const pushToBackground = () => {
				const dim = dimTowardBackground(origColor);
				res.color = dim;
				res.borderColor = dim;
				res.type = "circle";
				res.image = undefined;
				res.label = "";
				res.zIndex = 0;
				res.size =
					((res.size as number) ?? DEFAULT_NODE_SIZE) * CONTEXT_DIM_NODE_SCALE;
			};

			const pullToForeground = (zIndex: number) => {
				res.color = origColor;
				res.borderColor = origColor;
				res.zIndex = zIndex;
			};

			if (hl.highlightedNodeIds && hl.highlightedNodeIds.size > 0) {
				if (!hl.highlightedNodeIds.has(node)) pushToBackground();
				else pullToForeground(2);
				return res;
			}

			const activeNode = hl.selectedNodeId ?? hl.hoveredNode;
			if (hl.neighborSet && activeNode) {
				if (node === activeNode) {
					pullToForeground(3);
					res.highlighted = true;
					res.forceLabel = true;
					res.size = ((res.size as number) ?? DEFAULT_NODE_SIZE) * 1.3;
				} else if (hl.neighborSet.has(node)) {
					pullToForeground(2);
					// Only the biggest few neighbours get a forced caption; forcing all
					// of them around a hub was a wall of text the grid exists to prevent.
					if (hl.labeledNeighborSet?.has(node)) res.forceLabel = true;
				} else {
					pushToBackground();
				}
			}
			return res;
		},
		[],
	);

	const edgeReducer = useCallback(
		(edge: string, attrs: Record<string, unknown>) => {
			const res = { ...attrs };
			const hl = highlightRef.current;

			if (!graph || !graph.hasEdge(edge)) return res;

			const [src, tgt] = graph.extremities(edge);
			const srcLabel = attrs.srcLabel as string;
			const tgtLabel = attrs.tgtLabel as string;
			const storedLabel = res.label as string | undefined;
			if (
				hl.hiddenLabels?.has(srcLabel) ||
				hl.hiddenLabels?.has(tgtLabel) ||
				(storedLabel && hl.hiddenLabels?.has(storedLabel)) ||
				(hl.visibleNodeIds &&
					(!hl.visibleNodeIds.has(src) || !hl.visibleNodeIds.has(tgt)))
			) {
				res.hidden = true;
				return res;
			}

			// Relationship names appear on hover, selection and highlight only. The
			// legend already names the types; drawn on every edge they were most of
			// the unreadable text in a dense view.
			res.label = undefined;

			const origColor = attrs.originalColor as string;
			const edgeId = attrs.edgeId as string | undefined;
			const isHoveredEdge = hl.hoveredEdge === edge;

			const hasNodeHighlight = Boolean(
				hl.highlightedNodeIds && hl.highlightedNodeIds.size > 0,
			);
			const hasEdgeHighlight = Boolean(
				hl.highlightedEdgeIds && hl.highlightedEdgeIds.size > 0,
			);

			// Either channel alone drives the dim treatment. Gating the edge set behind a
			// non-empty node set made "highlight these relationships" unexpressible.
			if (hasNodeHighlight || hasEdgeHighlight) {
				const isHighlighted = hasEdgeHighlight
					? Boolean(edgeId && hl.highlightedEdgeIds?.has(edgeId))
					: Boolean(
							hl.highlightedNodeIds?.has(src) ||
								hl.highlightedNodeIds?.has(tgt),
						);
				if (!isHighlighted) {
					res.color = hexToRgba(origColor, CONTEXT_DIM_EDGE_ALPHA);
					res.size = CONTEXT_DIM_EDGE_SIZE;
					res.zIndex = 0;
					res.forceLabel = false;
					return res;
				}
				res.color = hexToRgba(origColor, 0.5);
				res.size = 1;
				res.label = storedLabel;
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
					res.forceLabel = false;
				} else {
					res.color = hexToRgba(origColor, CONTEXT_DIM_EDGE_ALPHA);
					res.size = CONTEXT_DIM_EDGE_SIZE;
					res.zIndex = 0;
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
				res.label = storedLabel;
				res.forceLabel = true;
				res.size = 1.5;
				res.color = hexToRgba(origColor, 0.7);
				res.zIndex = 1;
			}

			if (hl.selectedEdgeKey && edgeId === hl.selectedEdgeKey) {
				res.size = 2.5;
				res.zIndex = 2;
				res.color = hexToRgba(origColor, 0.9);
				res.label = storedLabel;
				res.forceLabel = true;
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
			// Text and faint edges are what the eye cannot track mid-pan anyway;
			// dropping them while the camera moves keeps the motion fluid.
			hideLabelsOnMove: isLarge,
			hideEdgesOnMove: isHuge,
			edgeLabelSize: 10,
			labelSize: isHuge ? 10 : isLarge ? 11 : 12,
			// A rendered-pixel cutoff, so it tracks the same fit scale the nodes get:
			// left fixed while every node shrinks, it would cull the captions that
			// used to clear it and read as labels going missing.
			labelRenderedSizeThreshold: isHuge
				? 18
				: (isLarge ? 12 : 6) *
					LABEL_THRESHOLD_HEADROOM *
					fitSizeScale(nodeCount),
			labelDensity: isHuge ? 0.07 : isDense ? 0.25 : isLarge ? 0.35 : 0.5,
			labelGridCellSize: isHuge ? 300 : isDense ? 180 : isLarge ? 140 : 100,
			labelFont: "Inter, system-ui, sans-serif",
			labelWeight: "500",
			defaultDrawNodeLabel: drawNodeLabel,
			defaultDrawNodeHover: drawNodeHover,
			zIndex: !isHuge,
			// Attribute-only reducers stay on at every size: turning them off above
			// the huge threshold silently killed legend toggles, focus, search
			// highlight and selection dimming exactly where orientation matters most.
			nodeReducer,
			edgeReducer,
			// Sigma's default is `sqrt(ratio)`, which shrinks a node far slower than
			// it shrinks the distance to its neighbours: zooming out to take in the
			// whole graph is exactly when circles swallow the edges between them.
			zoomToSizeRatioFunction: (ratio: number) => ratio ** 0.8,
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
				{t("noGraphDataToDisplay", "No graph data to display")}
			</div>
		);
	}

	const [bgR, bgG, bgB] = theme.bgRgb;

	return (
		<div
			className={`relative h-full w-full ${className ?? ""}`}
			onContextMenu={
				onNodeContextMenu ? (event) => event.preventDefault() : undefined
			}
		>
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
						onNodeDoubleClick={onNodeDoubleClick}
						onNodeContextMenu={onNodeContextMenu}
						onEdgeClick={onEdgeClick}
						onStageClick={onStageClick}
						onNodeDragged={handleNodeDragged}
						highlightRef={highlightRef}
						graph={graph}
					/>
					{useWorkerLayout ? (
						<SigmaWorkerLayout
							enabled={useWorkerLayout}
							graphRevision={graphRevision}
							nodeCount={graph.order}
							edgeCount={graph.size}
							stopToken={workerStopToken}
							onRunningChange={setIsWorkerLayoutRunning}
						/>
					) : null}
					<SigmaLayoutApplier
						command={layoutCommand ?? null}
						onApplied={scheduleSceneSave}
					/>
					<SigmaRefresher refreshKey={sigmaRefreshTrigger} />
					<SigmaControls
						onResetLayout={handleResetLayout}
						onUnpinAll={handleUnpinAll}
						pinnedCount={pinnedCount}
						disabled={isBusy}
					/>
				</SigmaContainer>
			) : null}

			{isWorkerLayoutRunning ? (
				<div className="absolute left-3 top-3 z-10 flex items-center gap-2 rounded-full border bg-background/80 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur-sm">
					<LoaderCircle className="h-3.5 w-3.5 animate-spin text-primary" />
					<span>{t("refiningLayout", "Refining layout")}</span>
					<button
						type="button"
						className="flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] hover:bg-accent"
						onClick={() => setWorkerStopToken((token) => token + 1)}
						title={t("stopArranging", "Stop arranging and settle the layout")}
					>
						<Square className="h-2.5 w-2.5" />
						{t("stop", "Stop")}
					</button>
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
