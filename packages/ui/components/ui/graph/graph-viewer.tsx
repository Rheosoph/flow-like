"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	BarChart3,
	Crosshair,
	Filter,
	FilterX,
	LayoutGrid,
	Route,
	Search,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	GraphAnalyticsResult,
	GraphOverlay,
	GraphPathsResult,
	LabelStyle,
	OntologyActionDefinition,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { subgraphFromCypherRows } from "./cypher-subgraph";
import {
	GraphCanvas,
	type GraphCanvasApi,
	type GraphLayoutCommand,
} from "./graph-canvas";
import { buildClusterModel } from "./graph-clusters";
import {
	collapseClusters,
	collapsedGroupClusterId,
	isCollapsedGroupId,
} from "./graph-collapse";
import {
	GraphContextMenu,
	type GraphContextMenuState,
} from "./graph-context-menu";
import { GraphDensityControl } from "./graph-density-control";
import { GraphEdgeInspector } from "./graph-edge-inspector";
import {
	type ExpansionOptions,
	GraphExpansionDialog,
	buildExpansionChoices,
} from "./graph-expansion-dialog";
import { GraphHistogramPanel } from "./graph-histogram-panel";
import type { GraphLayoutMode } from "./graph-layout";
import { GraphLegend, type LegendEntry } from "./graph-legend";
import {
	type ConnectionInfo,
	GraphNodeInspector,
} from "./graph-node-inspector";
import { GraphQueryPanel } from "./graph-query-panel";

const GRAPH_VIEW_LIMIT_OPTIONS = [
	50, 100, 200, 500, 1000, 2500, 5000, 10000,
] as const;

/** Members past which a group lands pre-collapsed instead of as loose nodes. */
const AUTO_COLLAPSE_MIN_MEMBERS = 75;
/** Budget for one-gesture expansions (double-click, context-menu rows). */
const QUICK_EXPANSION_LIMIT = 100;

const LAYOUT_MODE_OPTIONS: { mode: GraphLayoutMode; label: string }[] = [
	{ mode: "auto", label: "Auto" },
	{ mode: "force", label: "Force" },
	{ mode: "hierarchy", label: "Hierarchy" },
	{ mode: "radial", label: "Radial" },
	{ mode: "circular", label: "Circular" },
	{ mode: "grid", label: "Grid" },
];

function formatGraphLimitOption(limit: number): string {
	if (limit >= 1000000)
		return i18next.t("valmNodes", "{{val}}m nodes", { val: limit / 1000000 });
	if (limit >= 1000)
		return i18next.t("valkNodes", "{{val}}k nodes", { val: limit / 1000 });
	return i18next.t("limitNodes", "{{limit}} nodes", { limit });
}

function getEffectiveNodeIdColumn(
	overlay: GraphOverlay,
	label: string,
): string | undefined {
	for (const edge of overlay.edges) {
		if (edge.src_label === label && edge.src_node_column) {
			return edge.src_node_column;
		}
		if (edge.dst_label === label && edge.dst_node_column) {
			return edge.dst_node_column;
		}
	}

	return overlay.nodes.find((node) => node.label === label)?.id_column;
}

export function getNodeRawId(
	node: SubgraphNode,
	overlay: GraphOverlay,
): unknown {
	const idColumn = getEffectiveNodeIdColumn(overlay, node.label);
	if (idColumn) {
		const rawId = node.props[idColumn];
		if (rawId !== undefined && rawId !== null) {
			return rawId;
		}
	}

	const prefix = `${node.label}:`;
	return node.id.startsWith(prefix) ? node.id.slice(prefix.length) : node.id;
}

function areNodeSetsEqual(left: Set<string>, right: Set<string>): boolean {
	if (left.size !== right.size) return false;
	for (const value of left) {
		if (!right.has(value)) return false;
	}
	return true;
}

function getSearchErrorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	return String(error);
}

export interface GraphViewerProps {
	overlay: GraphOverlay;
	data: SubgraphResult | null;
	loading?: boolean;
	truncated?: boolean;
	onRunCypher?: (query: string) => void;
	cypherResults?: unknown[] | null;
	cypherLoading?: boolean;
	cypherError?: string | null;
	/**
	 * May resolve to the node ids the expansion actually added — that list is
	 * what makes a double-click expansion reversible.
	 */
	onExpandNode?: (
		nodeId: string,
		label: string,
		rawId?: unknown,
		seedNode?: SubgraphNode,
		depth?: number,
		options?: ExpansionOptions,
		// biome-ignore lint/suspicious/noConfusingVoidType: fire-and-forget handlers stay assignable; only the promise form reports added ids
	) => void | Promise<string[] | undefined>;
	onExpandChildren?: (nodeId: string, label: string, rawId?: unknown) => void;
	onCollapseChildren?: (nodeId: string) => void;
	/** Removes previously-loaded nodes again (undo for reversible expansion). */
	onRemoveNodes?: (nodeIds: string[]) => void;
	/** Merges an externally built result (Cypher, say) into the loaded scene. */
	onMergeSubgraph?: (result: SubgraphResult) => void;
	/** Storage key under which node positions and pins survive a reload. */
	persistKey?: string;
	expandedChildParents?: Set<string>;
	onSearchNodes?: (query: string) => Promise<SubgraphNode[]>;
	onStyleChange?: (
		label: string,
		type: "node" | "edge",
		style: LabelStyle,
	) => void;
	onLimitChange?: (limit: number) => void;
	limit?: number;
	onFindPaths?: (
		from: SubgraphNode,
		to: SubgraphNode,
	) => Promise<GraphPathsResult>;
	onRunAction?: (action: OntologyActionDefinition, node: SubgraphNode) => void;
	/** Fires whenever the selected node changes, including deselection. */
	onNodeSelect?: (node: SubgraphNode | null) => void;
	/** Fires whenever the selected edge changes, including deselection. */
	onEdgeSelect?: (edge: SubgraphEdge | null) => void;
	showToolbar?: boolean;
	showSearch?: boolean;
	showLegend?: boolean;
	/** Node/edge detail drawer opened by selection. */
	showInspector?: boolean;
	/** Whole-population counts, used to say what the loaded sample is a sample of. */
	analytics?: GraphAnalyticsResult | null;
	/**
	 * Groups the canvas into constellations around their hubs. Off by default:
	 * the inline a2ui graph element builds a synthetic overlay with no
	 * containment mappings, and must keep rendering exactly as it does today.
	 */
	enableClusterLayout?: boolean;
}

interface PathOutcome {
	found: boolean;
	hops: number;
	alternatives: number;
	error?: string;
}

export function GraphViewer({
	overlay,
	data,
	loading,
	truncated,
	onRunCypher,
	cypherResults,
	cypherLoading,
	cypherError,
	onExpandNode,
	onExpandChildren,
	onCollapseChildren,
	onRemoveNodes,
	onMergeSubgraph,
	persistKey,
	expandedChildParents,
	onSearchNodes,
	onStyleChange,
	onLimitChange,
	limit,
	onFindPaths,
	onRunAction,
	onNodeSelect,
	onEdgeSelect,
	showToolbar = true,
	showSearch = true,
	showLegend = true,
	showInspector = true,
	analytics,
	enableClusterLayout = false,
}: GraphViewerProps) {
	const { t } = useTranslation("common");
	const [selectedNode, setSelectedNode] = useState<SubgraphNode | null>(null);
	const [selectedEdge, setSelectedEdge] = useState<SubgraphEdge | null>(null);
	const [selectedEdgeKey, setSelectedEdgeKey] = useState<string | null>(null);
	const [showQuery, setShowQuery] = useState(false);
	const [hiddenLabels, setHiddenLabels] = useState<Set<string>>(new Set());
	const [searchHighlight, setSearchHighlight] = useState<Set<string>>(
		new Set(),
	);
	const [searchQuery, setSearchQuery] = useState("");
	const [remoteSearchMatches, setRemoteSearchMatches] = useState<
		SubgraphNode[]
	>([]);
	const [remoteSearchLoading, setRemoteSearchLoading] = useState(false);
	const [remoteSearchError, setRemoteSearchError] = useState<string | null>(
		null,
	);
	const [pendingSearchNodeId, setPendingSearchNodeId] = useState<string | null>(
		null,
	);
	const latestRemoteSearchRequestRef = useRef(0);
	const autoExpandedSearchQueryRef = useRef<string | null>(null);

	/**
	 * Focus is two-stage, Bloom-style: `dim` keeps the rest of the graph as
	 * grayed-out context, `hide` removes it and gives the survivors the stage.
	 */
	const [focus, setFocus] = useState<{
		nodeId: string;
		depth: number;
		mode: "dim" | "hide";
	} | null>(null);
	const [expansionTarget, setExpansionTarget] = useState<SubgraphNode | null>(
		null,
	);
	/** Degree at or below which a node is dropped as a leaf; 0 keeps everything. */
	const [leafCutoff, setLeafCutoff] = useState(0);
	const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(
		new Set(),
	);
	const [contextMenu, setContextMenu] = useState<GraphContextMenuState | null>(
		null,
	);
	/** Objects hidden one at a time from the context menu. */
	const [manualHidden, setManualHidden] = useState<Set<string>>(new Set());
	/** Facet-panel filters; `to` restricts, `out` subtracts. */
	const [facetFilters, setFacetFilters] = useState<
		{ id: string; title: string; mode: "to" | "out"; ids: Set<string> }[]
	>([]);
	const [facetsOpen, setFacetsOpen] = useState(false);
	const [facetHover, setFacetHover] = useState<Set<string> | null>(null);
	const [layoutCommand, setLayoutCommand] = useState<GraphLayoutCommand | null>(
		null,
	);
	const [layoutMode, setLayoutMode] = useState<GraphLayoutMode>("auto");
	const [canvasApi, setCanvasApi] = useState<GraphCanvasApi | null>(null);
	/** nodeId → the nodes its double-click expansion added, for the undo click. */
	const doubleExpandedRef = useRef<Map<string, string[]>>(new Map());
	/** Groups the reader opened by hand; auto-collapse never re-folds these. */
	const userOpenedGroupsRef = useRef<Set<string>>(new Set());
	const autoCollapsedRef = useRef<Set<string>>(new Set());

	const [pathSource, setPathSource] = useState<SubgraphNode | null>(null);
	const [pathHighlight, setPathHighlight] = useState<Set<string> | null>(null);
	const [pathEdgeHighlight, setPathEdgeHighlight] =
		useState<Set<string> | null>(null);
	const [pathOutcome, setPathOutcome] = useState<PathOutcome | null>(null);
	const [pathFinding, setPathFinding] = useState(false);
	const latestPathRequestRef = useRef(0);
	const [dismissedWarningsKey, setDismissedWarningsKey] = useState<
		string | null
	>(null);

	const warnings = data?.warnings ?? [];
	const warningsKey = warnings.join(" ");
	const warningsDismissed = dismissedWarningsKey === warningsKey;

	const nodeCount = data?.nodes.length ?? 0;
	const edgeCount = data?.edges.length ?? 0;

	// Grouping is computed over the loaded sample, never over the collapsed view:
	// collapsing must not change which groups exist, or opening one would land the
	// reader in a differently-grouped graph than the one they collapsed.
	const clusterModel = useMemo(
		() =>
			enableClusterLayout && data ? buildClusterModel(data, overlay) : null,
		[enableClusterLayout, data, overlay],
	);

	const collapsed = useMemo(() => {
		if (!data || !clusterModel || collapsedGroups.size === 0) {
			return { data, hiddenNodeCount: 0 };
		}
		return collapseClusters(data, clusterModel, collapsedGroups);
	}, [data, clusterModel, collapsedGroups]);

	const viewData = collapsed.data;

	// The layout needs a grouping over what is actually drawn. Reusing the model
	// built from the full sample would leave every group node unplaced: a synthetic
	// node belongs to no cluster, so the cluster layout would never move it and it
	// would keep whatever seed position it was born with.
	const layoutClusterModel = useMemo(() => {
		if (collapsedGroups.size === 0) return clusterModel;
		if (!enableClusterLayout || !viewData) return null;
		return buildClusterModel(viewData, overlay);
	}, [collapsedGroups, clusterModel, enableClusterLayout, viewData, overlay]);

	const collapseAllGroups = useCallback(() => {
		setCollapsedGroups(
			new Set((clusterModel?.clusters ?? []).map((cluster) => cluster.id)),
		);
	}, [clusterModel]);

	const expandAllGroups = useCallback(() => {
		setCollapsedGroups((prev) => {
			for (const id of prev) userOpenedGroupsRef.current.add(id);
			return new Set();
		});
	}, []);

	// A group id that no longer exists would keep a phantom collapsed forever.
	useEffect(() => {
		if (collapsedGroups.size === 0) return;
		const live = new Set((clusterModel?.clusters ?? []).map((c) => c.id));
		const next = new Set([...collapsedGroups].filter((id) => live.has(id)));
		if (next.size !== collapsedGroups.size) setCollapsedGroups(next);
	}, [clusterModel, collapsedGroups]);

	// Overflow degrades to aggregation, never to a hairball: a group past this
	// size arrives folded into one badge-carrying node, and opening it is one
	// click. Groups the reader opened by hand are never re-folded.
	useEffect(() => {
		if (!clusterModel) return;
		const toCollapse: string[] = [];
		for (const cluster of clusterModel.clusters) {
			if (cluster.memberIds.length < AUTO_COLLAPSE_MIN_MEMBERS) continue;
			if (autoCollapsedRef.current.has(cluster.id)) continue;
			if (userOpenedGroupsRef.current.has(cluster.id)) continue;
			autoCollapsedRef.current.add(cluster.id);
			if (!collapsedGroups.has(cluster.id)) toCollapse.push(cluster.id);
		}
		if (toCollapse.length > 0) {
			setCollapsedGroups((prev) => new Set([...prev, ...toCollapse]));
		}
	}, [clusterModel, collapsedGroups]);

	// Only node_count and label_counts describe the whole population; every other
	// analytics field is measured over a bounded edge snapshot and would read as a
	// finding about the ontology when it is really a fact about the sample.
	//
	// label_counts is count_rows() on the mapped table, so a label that shares its
	// table with another label (an object collapsed by a coarser key, say) gets the
	// row count rather than its own population. Those are dropped instead of shown
	// as exact.
	const populationByLabel = useMemo(() => {
		const labelsPerTable = new Map<string, number>();
		for (const node of overlay.nodes) {
			labelsPerTable.set(node.table, (labelsPerTable.get(node.table) ?? 0) + 1);
		}
		const soleMappingLabels = new Set(
			overlay.nodes
				.filter((node) => (labelsPerTable.get(node.table) ?? 0) === 1)
				.map((node) => node.label),
		);

		const totals = new Map<string, number>();
		for (const entry of analytics?.label_counts ?? []) {
			if (soleMappingLabels.has(entry.label))
				totals.set(entry.label, entry.nodes);
		}
		return totals;
	}, [analytics, overlay]);

	const legendEntries = useMemo<LegendEntry[]>(() => {
		const entries: LegendEntry[] = [];
		const nodeCounts = new Map<string, number>();
		const edgeCounts = new Map<string, number>();

		if (data) {
			for (const n of data.nodes) {
				nodeCounts.set(n.label, (nodeCounts.get(n.label) ?? 0) + 1);
			}
			for (const e of data.edges) {
				edgeCounts.set(e.label, (edgeCounts.get(e.label) ?? 0) + 1);
			}
		}

		for (const nm of overlay.nodes) {
			entries.push({
				label: nm.label,
				style: nm.style,
				count: nodeCounts.get(nm.label),
				total: populationByLabel.get(nm.label),
				type: "node",
			});
		}
		for (const em of overlay.edges) {
			entries.push({
				label: em.label,
				style: em.style,
				count: edgeCounts.get(em.label),
				type: "edge",
			});
		}
		return entries;
	}, [overlay, data, populationByLabel]);

	/** Says what the loaded nodes are a sample of, so the view never implies it is everything. */
	const censusSummary = useMemo(() => {
		if (populationByLabel.size === 0) return null;

		const loadedByLabel = new Map<string, number>();
		for (const node of data?.nodes ?? []) {
			loadedByLabel.set(node.label, (loadedByLabel.get(node.label) ?? 0) + 1);
		}

		const parts = Array.from(populationByLabel.entries())
			.filter(([label]) => (loadedByLabel.get(label) ?? 0) > 0)
			.sort((a, b) => b[1] - a[1])
			.map(([label, total]) =>
				t("valOfVal2Label", "{{val}} of {{val2}} {{label}}", {
					val: (loadedByLabel.get(label) ?? 0).toLocaleString(),
					val2: total.toLocaleString(),
					label,
				}),
			);

		return parts.length > 0
			? t("showingVal", "Showing {{val}}", { val: parts.join(" · ") })
			: null;
	}, [populationByLabel, data, t]);

	const nodeMap = useMemo(() => {
		if (!data) return new Map<string, SubgraphNode>();
		const map = new Map<string, SubgraphNode>();
		for (const n of data.nodes) map.set(n.id, n);
		return map;
	}, [data]);

	const unloadedRemoteSearchMatches = useMemo(
		() => remoteSearchMatches.filter((node) => !nodeMap.has(node.id)),
		[nodeMap, remoteSearchMatches],
	);

	const nodeConnections = useMemo<ConnectionInfo[]>(() => {
		if (!selectedNode || !data) return [];
		const conns: ConnectionInfo[] = [];
		for (const edge of data.edges) {
			if (edge.source === selectedNode.id) {
				const target = nodeMap.get(edge.target);
				conns.push({
					label: edge.label,
					direction: "outgoing",
					targetCaption: target?.caption ?? target?.id ?? edge.target,
					targetId: edge.target,
				});
			} else if (edge.target === selectedNode.id) {
				const source = nodeMap.get(edge.source);
				conns.push({
					label: edge.label,
					direction: "incoming",
					targetCaption: source?.caption ?? source?.id ?? edge.source,
					targetId: edge.source,
				});
			}
		}
		return conns;
	}, [selectedNode, data, nodeMap]);

	const edgeSourceCaption = useMemo(() => {
		if (!selectedEdge) return undefined;
		const src = nodeMap.get(selectedEdge.source);
		return src?.caption ?? src?.id;
	}, [selectedEdge, nodeMap]);

	const edgeTargetCaption = useMemo(() => {
		if (!selectedEdge) return undefined;
		const tgt = nodeMap.get(selectedEdge.target);
		return tgt?.caption ?? tgt?.id;
	}, [selectedEdge, nodeMap]);

	useEffect(() => {
		if (!selectedNode || !data) return;
		const updated = data.nodes.find((node) => node.id === selectedNode.id);
		if (updated && updated !== selectedNode) {
			setSelectedNode(updated);
		}
	}, [data, selectedNode]);

	// Selection is reached from the canvas, the search panel and the inspector.
	// Emitting from the resolved state keeps one notification per actual change;
	// the id guard swallows the re-selection that data refreshes trigger.
	const emittedNodeIdRef = useRef<string | null>(null);
	useEffect(() => {
		const id = selectedNode?.id ?? null;
		if (emittedNodeIdRef.current === id) return;
		emittedNodeIdRef.current = id;
		onNodeSelect?.(selectedNode);
	}, [selectedNode, onNodeSelect]);

	const emittedEdgeIdRef = useRef<string | null>(null);
	useEffect(() => {
		const id = selectedEdge?.id ?? null;
		if (emittedEdgeIdRef.current === id) return;
		emittedEdgeIdRef.current = id;
		onEdgeSelect?.(selectedEdge);
	}, [selectedEdge, onEdgeSelect]);

	useEffect(() => {
		if (!pendingSearchNodeId || !data) return;
		const node = data.nodes.find(
			(candidate) => candidate.id === pendingSearchNodeId,
		);
		if (!node) return;
		setSelectedNode(node);
		setSelectedEdge(null);
		setSelectedEdgeKey(null);
		setPendingSearchNodeId(null);
	}, [data, pendingSearchNodeId]);

	useEffect(() => {
		const trimmedQuery = searchQuery.trim().toLowerCase();
		if (!data || !trimmedQuery) {
			setSearchHighlight((current) =>
				current.size === 0 ? current : new Set<string>(),
			);
			return;
		}

		const matches = new Set<string>();
		for (const node of data.nodes) {
			const caption = (node.caption ?? "").toLowerCase();
			const fullId = node.id.toLowerCase();
			const label = node.label.toLowerCase();
			if (
				caption.includes(trimmedQuery) ||
				fullId.includes(trimmedQuery) ||
				label.includes(trimmedQuery)
			) {
				matches.add(node.id);
			}
		}

		setSearchHighlight((current) =>
			areNodeSetsEqual(current, matches) ? current : matches,
		);

		if (matches.size === 1) {
			const [matchId] = matches;
			const node = data.nodes.find((candidate) => candidate.id === matchId);
			if (node) {
				setSelectedNode(node);
				setSelectedEdge(null);
				setSelectedEdgeKey(null);
			}
		}
	}, [data, searchQuery]);

	useEffect(() => {
		const trimmedQuery = searchQuery.trim();
		if (!trimmedQuery || trimmedQuery.length < 2 || !onSearchNodes) {
			latestRemoteSearchRequestRef.current += 1;
			setRemoteSearchMatches([]);
			setRemoteSearchError(null);
			setRemoteSearchLoading(false);
			if (!trimmedQuery) {
				autoExpandedSearchQueryRef.current = null;
			}
			return;
		}

		const requestId = latestRemoteSearchRequestRef.current + 1;
		latestRemoteSearchRequestRef.current = requestId;
		setRemoteSearchLoading(true);
		setRemoteSearchError(null);

		const timeoutId = window.setTimeout(() => {
			void (async () => {
				try {
					const matches = await onSearchNodes(trimmedQuery);
					if (latestRemoteSearchRequestRef.current !== requestId) return;
					setRemoteSearchMatches(matches);
				} catch (error) {
					if (latestRemoteSearchRequestRef.current !== requestId) return;
					setRemoteSearchMatches([]);
					setRemoteSearchError(getSearchErrorMessage(error));
				} finally {
					if (latestRemoteSearchRequestRef.current === requestId) {
						setRemoteSearchLoading(false);
					}
				}
			})();
		}, 250);

		return () => window.clearTimeout(timeoutId);
	}, [onSearchNodes, searchQuery]);

	useEffect(() => {
		const trimmedQuery = searchQuery.trim();
		if (!trimmedQuery || !onExpandNode) return;
		if (searchHighlight.size > 0 || unloadedRemoteSearchMatches.length !== 1)
			return;
		if (autoExpandedSearchQueryRef.current === trimmedQuery) return;

		const [match] = unloadedRemoteSearchMatches;
		autoExpandedSearchQueryRef.current = trimmedQuery;
		setPendingSearchNodeId(match.id);
		void onExpandNode(
			match.id,
			match.label,
			getNodeRawId(match, overlay),
			match,
		);
	}, [
		onExpandNode,
		overlay,
		searchHighlight.size,
		searchQuery,
		unloadedRemoteSearchMatches,
	]);

	const runPath = useCallback(
		async (target: SubgraphNode) => {
			if (!pathSource || !onFindPaths || target.id === pathSource.id) return;
			const source = pathSource;
			const requestId = latestPathRequestRef.current + 1;
			latestPathRequestRef.current = requestId;
			setPathFinding(true);
			setPathOutcome(null);
			try {
				const result = await onFindPaths(source, target);
				if (latestPathRequestRef.current !== requestId) return;
				const ids = new Set<string>();
				const edgeIds = new Set<string>();
				for (const path of result.paths) {
					for (const id of path.node_ids) ids.add(id);
					for (const id of path.edge_ids) edgeIds.add(id);
				}
				ids.add(source.id);
				ids.add(target.id);
				const shortest = result.paths.reduce(
					(min, path) => Math.min(min, path.length),
					Number.POSITIVE_INFINITY,
				);
				setPathHighlight(result.found && ids.size > 0 ? ids : null);
				setPathEdgeHighlight(result.found ? edgeIds : null);
				setPathOutcome({
					found: result.found,
					hops: Number.isFinite(shortest) ? shortest : 0,
					alternatives: Math.max(0, result.paths.length - 1),
				});
			} catch (error) {
				if (latestPathRequestRef.current !== requestId) return;
				setPathHighlight(null);
				setPathEdgeHighlight(null);
				setPathOutcome({
					found: false,
					hops: 0,
					alternatives: 0,
					error: getSearchErrorMessage(error),
				});
			} finally {
				if (latestPathRequestRef.current === requestId) {
					setPathFinding(false);
					setPathSource(null);
				}
			}
		},
		[onFindPaths, pathSource],
	);

	const handleArmPath = useCallback((node: SubgraphNode) => {
		latestPathRequestRef.current += 1;
		setPathFinding(false);
		setPathSource(node);
		setPathHighlight(null);
		setPathEdgeHighlight(null);
		setPathOutcome(null);
	}, []);

	const exitPathMode = useCallback(() => {
		latestPathRequestRef.current += 1;
		setPathSource(null);
		setPathHighlight(null);
		setPathEdgeHighlight(null);
		setPathOutcome(null);
		setPathFinding(false);
	}, []);

	const hasContainmentChildren = useCallback(
		(node: SubgraphNode) =>
			overlay.edges.some(
				(edge) => edge.containment && edge.src_label === node.label,
			),
		[overlay],
	);

	const adjacency = useMemo(() => {
		const map = new Map<string, string[]>();
		for (const edge of viewData?.edges ?? []) {
			const forward = map.get(edge.source);
			if (forward) forward.push(edge.target);
			else map.set(edge.source, [edge.target]);
			const backward = map.get(edge.target);
			if (backward) backward.push(edge.source);
			else map.set(edge.target, [edge.source]);
		}
		return map;
	}, [viewData]);

	/**
	 * Nodes within `focus.depth` hops of the focused one.
	 *
	 * Dimming answers "which of these is related"; on a sample this size the
	 * question is "can I see anything at all", and only removing the rest of the
	 * graph answers that.
	 */
	const focusedNodeIds = useMemo(() => {
		if (!focus || !viewData) return undefined;

		const visible = new Set<string>([focus.nodeId]);
		let frontier = [focus.nodeId];
		for (let hop = 0; hop < focus.depth; hop += 1) {
			const next: string[] = [];
			for (const nodeId of frontier) {
				for (const neighbor of adjacency.get(nodeId) ?? []) {
					if (visible.has(neighbor)) continue;
					visible.add(neighbor);
					next.push(neighbor);
				}
			}
			if (next.length === 0) break;
			frontier = next;
		}
		return visible;
	}, [focus, viewData, adjacency]);

	/**
	 * Nodes surviving the leaf cutoff — degree is counted over the loaded sample,
	 * which is the only degree the reader can see and therefore the only one the
	 * control can honestly claim to filter on.
	 */
	const nonLeafNodeIds = useMemo(() => {
		if (leafCutoff <= 0 || !viewData) return undefined;
		const kept = new Set<string>();
		for (const node of viewData.nodes) {
			// A collapsed group is never a leaf: it stands for members whose degree
			// the reader cannot see, so hiding it would hide what it represents.
			if (
				isCollapsedGroupId(node.id) ||
				(adjacency.get(node.id)?.length ?? 0) > leafCutoff
			) {
				kept.add(node.id);
			}
		}
		return kept;
	}, [leafCutoff, viewData, adjacency]);

	const hiddenLeafCount = useMemo(
		() =>
			nonLeafNodeIds
				? Math.max(0, (viewData?.nodes.length ?? 0) - nonLeafNodeIds.size)
				: 0,
		[nonLeafNodeIds, viewData],
	);

	/**
	 * Everything that restricts the stage, composed: hide-mode focus, the leaf
	 * cutoff, facet filter-to sets, then the subtractive channels (facet
	 * excludes and per-object hides). `undefined` means "no restriction".
	 */
	const visibleNodeIds = useMemo(() => {
		const intersect = (
			current: Set<string> | undefined,
			next: ReadonlySet<string>,
		): Set<string> => {
			if (!current) return new Set(next);
			const out = new Set<string>();
			for (const id of current) {
				if (next.has(id)) out.add(id);
			}
			return out;
		};

		let base: Set<string> | undefined;
		if (focus?.mode === "hide" && focusedNodeIds) {
			base = intersect(base, focusedNodeIds);
		}
		if (nonLeafNodeIds) base = intersect(base, nonLeafNodeIds);
		for (const filter of facetFilters) {
			if (filter.mode === "to") base = intersect(base, filter.ids);
		}
		// The focused node itself always survives, or the view it anchors would
		// vanish the moment a cutoff passed its own degree.
		if (base && focus) base.add(focus.nodeId);

		const hasSubtractive =
			manualHidden.size > 0 ||
			facetFilters.some((filter) => filter.mode === "out");
		if (hasSubtractive) {
			if (!base) {
				base = new Set((viewData?.nodes ?? []).map((node) => node.id));
			}
			for (const filter of facetFilters) {
				if (filter.mode !== "out") continue;
				for (const id of filter.ids) base.delete(id);
			}
			for (const id of manualHidden) base.delete(id);
		}
		return base;
	}, [
		focus,
		focusedNodeIds,
		nonLeafNodeIds,
		facetFilters,
		manualHidden,
		viewData,
	]);

	const expansionChoices = useMemo(() => {
		if (!expansionTarget) return [];
		const loadedByLabel = new Map<string, number>();
		for (const edge of data?.edges ?? []) {
			if (
				edge.source !== expansionTarget.id &&
				edge.target !== expansionTarget.id
			)
				continue;
			loadedByLabel.set(edge.label, (loadedByLabel.get(edge.label) ?? 0) + 1);
		}
		return buildExpansionChoices(expansionTarget, overlay, loadedByLabel);
	}, [expansionTarget, overlay, data]);

	const handleFocus = useCallback(
		(depth: number | null) => {
			if (depth === null || !selectedNode) {
				setFocus(null);
				return;
			}
			// Dim first: the rest of the graph stays as grayed context, and hiding
			// it is a second, explicit step in the banner.
			setFocus((prev) => ({
				nodeId: selectedNode.id,
				depth,
				mode: prev?.nodeId === selectedNode.id ? prev.mode : "dim",
			}));
		},
		[selectedNode],
	);

	const focusNodeById = useCallback((nodeId: string) => {
		setFocus((prev) =>
			prev?.nodeId === nodeId ? null : { nodeId, depth: 1, mode: "dim" },
		);
	}, []);

	// A focus is anchored to one node, so it cannot outlive that node being drawn.
	// Checked against the collapsed view, not the sample: folding the focused node
	// into a group removes it from the stage just as surely as a new query does,
	// and a focus on an undrawn node leaves nothing visible at all.
	useEffect(() => {
		if (!focus || !viewData) return;
		if (!viewData.nodes.some((node) => node.id === focus.nodeId))
			setFocus(null);
	}, [focus, viewData]);

	const openCollapsedGroup = useCallback((nodeId: string) => {
		const clusterId = collapsedGroupClusterId(nodeId);
		userOpenedGroupsRef.current.add(clusterId);
		setCollapsedGroups((prev) => {
			const next = new Set(prev);
			next.delete(clusterId);
			return next;
		});
	}, []);

	const handleNodeClick = useCallback(
		(nodeId: string) => {
			// A group is an affordance, not an object: clicking it opens it rather
			// than offering properties that belong to nobody.
			if (isCollapsedGroupId(nodeId)) {
				openCollapsedGroup(nodeId);
				return;
			}

			const node = data?.nodes.find((n) => n.id === nodeId);
			if (!node) return;
			if (pathSource && node.id !== pathSource.id) {
				void runPath(node);
				return;
			}
			setSelectedNode((prev) => (prev?.id === node.id ? null : node));
			setSelectedEdge(null);
			setSelectedEdgeKey(null);
		},
		[data, pathSource, runPath, openCollapsedGroup],
	);

	const handleEdgeClick = useCallback(
		(edgeId: string) => {
			if (selectedEdge?.id === edgeId) {
				setSelectedEdge(null);
				setSelectedEdgeKey(null);
				return;
			}
			const found = data?.edges.find((e) => e.id === edgeId);
			if (found) {
				setSelectedEdge(found);
				setSelectedEdgeKey(edgeId);
				setSelectedNode(null);
			}
		},
		[data, selectedEdge],
	);

	const handleStageClick = useCallback(() => {
		setSelectedNode(null);
		setSelectedEdge(null);
		setSelectedEdgeKey(null);
	}, []);

	const handleNodeShiftClick = useCallback(
		(nodeId: string, label: string) => {
			// A group stands for members the backend has never heard of, so it can
			// only be opened locally — expanding it would send a synthetic id.
			if (isCollapsedGroupId(nodeId)) {
				openCollapsedGroup(nodeId);
				return;
			}

			const node = data?.nodes.find((candidate) => candidate.id === nodeId);
			// Shift+Click opens the guard rather than expanding: this is the gesture
			// most likely to be aimed at a node with a four-figure fan-out.
			if (node) setExpansionTarget(node);
			else void onExpandNode?.(nodeId, label, undefined);
		},
		[data, onExpandNode, openCollapsedGroup],
	);

	/**
	 * Double-click grows the graph one budgeted hop, and a second double-click
	 * takes exactly that expansion back — the Browser-style reversible gesture.
	 */
	const handleNodeDoubleClick = useCallback(
		(nodeId: string) => {
			if (isCollapsedGroupId(nodeId)) {
				openCollapsedGroup(nodeId);
				return;
			}
			const node = data?.nodes.find((candidate) => candidate.id === nodeId);
			if (!node) return;

			const undo = doubleExpandedRef.current.get(nodeId);
			if (undo && undo.length > 0 && onRemoveNodes) {
				doubleExpandedRef.current.delete(nodeId);
				onRemoveNodes(undo);
				return;
			}
			if (!onExpandNode) return;
			void Promise.resolve(
				onExpandNode(
					node.id,
					node.label,
					getNodeRawId(node, overlay),
					undefined,
					1,
					{
						limit: QUICK_EXPANSION_LIMIT,
					},
				),
			).then((added) => {
				if (Array.isArray(added) && added.length > 0) {
					doubleExpandedRef.current.set(nodeId, added);
				}
			});
		},
		[data, onExpandNode, onRemoveNodes, overlay, openCollapsedGroup],
	);

	const handleNodeContextMenu = useCallback(
		(nodeId: string, position: { x: number; y: number }) => {
			setContextMenu({ nodeId, x: position.x, y: position.y });
		},
		[],
	);

	const closeContextMenu = useCallback(() => setContextMenu(null), []);

	const contextNode = useMemo(() => {
		if (!contextMenu) return null;
		return (
			viewData?.nodes.find((node) => node.id === contextMenu.nodeId) ??
			data?.nodes.find((node) => node.id === contextMenu.nodeId) ??
			null
		);
	}, [contextMenu, viewData, data]);

	const contextChoices = useMemo(() => {
		if (!contextNode || isCollapsedGroupId(contextNode.id)) return [];
		const loadedByLabel = new Map<string, number>();
		for (const edge of data?.edges ?? []) {
			if (edge.source !== contextNode.id && edge.target !== contextNode.id)
				continue;
			loadedByLabel.set(edge.label, (loadedByLabel.get(edge.label) ?? 0) + 1);
		}
		return buildExpansionChoices(contextNode, overlay, loadedByLabel);
	}, [contextNode, overlay, data]);

	const contextClusterId = useMemo(() => {
		if (!contextMenu || !clusterModel) return null;
		const assignment = clusterModel.byNode.get(contextMenu.nodeId);
		if (!assignment) return null;
		const cluster = clusterModel.clusters.find(
			(candidate) => candidate.id === assignment.clusterId,
		);
		return cluster && cluster.memberIds.length > 1 ? cluster.id : null;
	}, [contextMenu, clusterModel]);

	const hideNode = useCallback((nodeId: string) => {
		setManualHidden((prev) => new Set(prev).add(nodeId));
		setSelectedNode((prev) => (prev?.id === nodeId ? null : prev));
	}, []);

	const showHiddenNodes = useCallback(() => setManualHidden(new Set()), []);

	const removeFacetFilter = useCallback((filterId: string) => {
		setFacetFilters((prev) => prev.filter((filter) => filter.id !== filterId));
	}, []);

	const addFacetFilter = useCallback(
		(mode: "to" | "out", title: string, ids: Set<string>) => {
			setFacetFilters((prev) => [
				...prev,
				{ id: `${mode}-${Date.now()}-${prev.length}`, title, mode, ids },
			]);
		},
		[],
	);

	const applyLayoutMode = useCallback(
		(mode: GraphLayoutMode) => {
			setLayoutMode(mode);
			setLayoutCommand((prev) => ({
				mode,
				seq: (prev?.seq ?? 0) + 1,
				centerNodeId: selectedNode?.id ?? null,
			}));
		},
		[selectedNode],
	);

	// Cypher rows resolved back into drawable structure, when they can be.
	const cypherSubgraph = useMemo(
		() =>
			cypherResults && onMergeSubgraph
				? subgraphFromCypherRows(cypherResults, overlay)
				: null,
		[cypherResults, onMergeSubgraph, overlay],
	);

	const addCypherToCanvas = useCallback(() => {
		if (cypherSubgraph && onMergeSubgraph) onMergeSubgraph(cypherSubgraph);
	}, [cypherSubgraph, onMergeSubgraph]);

	/**
	 * One highlight channel reaches the canvas; priority order: an explicit path
	 * beats a search, a search beats a hovered facet bar, and dim-mode focus is
	 * the ambient baseline under all of them.
	 */
	const effectiveHighlight = useMemo(() => {
		if (pathHighlight) return pathHighlight;
		if (searchHighlight.size > 0) return searchHighlight;
		if (facetHover && facetHover.size > 0) return facetHover;
		if (focus?.mode === "dim" && focusedNodeIds) return focusedNodeIds;
		return undefined;
	}, [pathHighlight, searchHighlight, facetHover, focus, focusedNodeIds]);

	const handleConnectionClick = useCallback(
		(targetNodeId: string) => {
			const node = data?.nodes.find((n) => n.id === targetNodeId);
			if (node) {
				setSelectedNode(node);
				setSelectedEdge(null);
				setSelectedEdgeKey(null);
			}
		},
		[data],
	);

	const handleToggleVisibility = useCallback(
		(label: string, visible: boolean) => {
			setHiddenLabels((prev) => {
				const next = new Set(prev);
				if (visible) {
					next.delete(label);
				} else {
					next.add(label);
				}
				return next;
			});
		},
		[],
	);

	const handleSearch = useCallback((query: string) => {
		setSearchQuery(query);
	}, []);

	const handleRemoteSearchMatchClick = useCallback(
		async (node: SubgraphNode) => {
			setPendingSearchNodeId(node.id);
			setSelectedEdge(null);
			setSelectedEdgeKey(null);
			if (onExpandNode) {
				await onExpandNode(
					node.id,
					node.label,
					getNodeRawId(node, overlay),
					node,
				);
				return;
			}
			setSelectedNode(node);
		},
		[onExpandNode, overlay],
	);

	const clearSearch = useCallback(() => {
		latestRemoteSearchRequestRef.current += 1;
		autoExpandedSearchQueryRef.current = null;
		setPendingSearchNodeId(null);
		setSearchQuery("");
		setSearchHighlight(new Set());
		setRemoteSearchMatches([]);
		setRemoteSearchError(null);
		setRemoteSearchLoading(false);
	}, []);

	const hasRemoteSearchQuery = searchQuery.trim().length >= 2;
	const showRemoteSearchPanel =
		hasRemoteSearchQuery &&
		!!onSearchNodes &&
		(remoteSearchLoading ||
			remoteSearchError !== null ||
			unloadedRemoteSearchMatches.length > 0 ||
			searchHighlight.size === 0);

	return (
		<div className="flex h-full min-h-0 w-full relative">
			{/* Main graph area */}
			<div className="flex-1 flex flex-col min-w-0 min-h-0">
				{/* Toolbar */}
				{showToolbar && (
					<div className="flex min-w-0 items-center gap-2 overflow-hidden border-b bg-background p-2">
						{showSearch && (
							<>
								{/* Live search */}
								<div className="relative flex-1 max-w-sm">
									<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
									<input
										type="text"
										value={searchQuery}
										onChange={(e) => handleSearch(e.target.value)}
										placeholder={t(
											"searchLoadedNodesThenFallbackToFullGraph",
											"Search loaded nodes, then fallback to full graph...",
										)}
										className="w-full h-9 pl-8 pr-8 text-sm rounded-md border bg-transparent focus:outline-none focus:ring-1 focus:ring-ring"
									/>
									{searchQuery && (
										<button
											type="button"
											className="absolute right-2 top-2.5 h-4 w-4 text-muted-foreground hover:text-foreground"
											onClick={clearSearch}
										>
											<X className="h-4 w-4" />
										</button>
									)}
									{showRemoteSearchPanel && (
										<div className="absolute left-0 right-0 top-full z-20 mt-1 rounded-md border bg-popover shadow-lg overflow-hidden">
											{remoteSearchLoading ? (
												<div className="px-3 py-2 text-xs text-muted-foreground">
													{t("searchingFullGraph", "Searching full graph...")}
												</div>
											) : remoteSearchError ? (
												<div className="px-3 py-2 text-xs text-destructive">
													{remoteSearchError}
												</div>
											) : unloadedRemoteSearchMatches.length > 0 ? (
												<div className="max-h-64 overflow-y-auto py-1">
													{unloadedRemoteSearchMatches.map((node) => (
														<button
															key={node.id}
															type="button"
															className="flex w-full flex-col items-start gap-0.5 px-3 py-2 text-left hover:bg-accent"
															onClick={() =>
																void handleRemoteSearchMatchClick(node)
															}
														>
															<span className="text-sm font-medium truncate max-w-full">
																{node.caption ?? node.id}
															</span>
															<span className="text-[11px] text-muted-foreground truncate max-w-full">{`${node.label} · ${node.id}`}</span>
														</button>
													))}
												</div>
											) : searchHighlight.size === 0 ? (
												<div className="px-3 py-2 text-xs text-muted-foreground">
													{t(
														"noNodesFoundInTheFullGraph",
														"No nodes found in the full graph.",
													)}
												</div>
											) : null}
										</div>
									)}
								</div>

								{searchHighlight.size > 0 && (
									<span className="text-xs text-muted-foreground whitespace-nowrap">
										{searchHighlight.size} loaded match
										{searchHighlight.size !== 1 ? "es" : ""}
									</span>
								)}

								{unloadedRemoteSearchMatches.length > 0 && (
									<span className="text-xs text-muted-foreground whitespace-nowrap">
										{t("lengthMoreInGraph", "{{length}} more in graph", {
											length: unloadedRemoteSearchMatches.length,
										})}
									</span>
								)}

								{hasRemoteSearchQuery && remoteSearchLoading && (
									<span className="text-xs text-muted-foreground whitespace-nowrap">
										{t("searchingFullGraph", "Searching full graph...")}
									</span>
								)}

								<div className="h-5 w-px bg-border" />
							</>
						)}

						{/* Node / edge count */}
						<span className="shrink-0 text-xs text-muted-foreground whitespace-nowrap">
							{nodeCount.toLocaleString()} nodes · {edgeCount.toLocaleString()}{" "}
							edges
						</span>

						{/* Limit selector */}
						{onLimitChange && (
							<>
								<div className="h-5 w-px shrink-0 bg-border" />
								<select
									value={limit ?? 200}
									onChange={(e) => onLimitChange(Number(e.target.value))}
									className="h-8 shrink-0 text-xs rounded-md border bg-transparent px-2 focus:outline-none focus:ring-1 focus:ring-ring"
								>
									{GRAPH_VIEW_LIMIT_OPTIONS.map((option) => (
										<option key={option} value={option}>
											{formatGraphLimitOption(option)}
										</option>
									))}
								</select>
							</>
						)}

						<div className="h-5 w-px shrink-0 bg-border" />

						{onRunCypher && (
							<button
								type="button"
								className="shrink-0 text-xs text-muted-foreground hover:text-foreground px-2 py-1 rounded border whitespace-nowrap"
								onClick={() => setShowQuery(!showQuery)}
							>
								{showQuery ? "Hide Query" : "Query"}
							</button>
						)}

						<GraphDensityControl
							collapsedGroups={collapsedGroups.size}
							groupCount={clusterModel?.clusters.length ?? 0}
							onCollapseAll={collapseAllGroups}
							onExpandAll={expandAllGroups}
							leafCutoff={leafCutoff}
							onLeafCutoffChange={setLeafCutoff}
							hiddenLeaves={hiddenLeafCount}
						/>

						<DropdownMenu>
							<DropdownMenuTrigger asChild>
								<button
									type="button"
									className={`flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded border px-2 py-1 text-xs transition-colors ${
										layoutMode !== "auto"
											? "border-primary/40 bg-primary/10 text-foreground"
											: "text-muted-foreground hover:text-foreground"
									}`}
									title={t(
										"chooseHowTheGraphIsArranged",
										"Choose how the graph is arranged",
									)}
								>
									<LayoutGrid className="h-3.5 w-3.5" />
									{LAYOUT_MODE_OPTIONS.find(
										(option) => option.mode === layoutMode,
									)?.label ?? "Auto"}
								</button>
							</DropdownMenuTrigger>
							<DropdownMenuContent align="start">
								{LAYOUT_MODE_OPTIONS.map((option) => (
									<DropdownMenuItem
										key={option.mode}
										className="text-xs"
										onSelect={() => applyLayoutMode(option.mode)}
									>
										<span className="min-w-0 flex-1">{option.label}</span>
										{option.mode === layoutMode && (
											<span className="text-primary">•</span>
										)}
									</DropdownMenuItem>
								))}
							</DropdownMenuContent>
						</DropdownMenu>

						<button
							type="button"
							className={`flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded border px-2 py-1 text-xs transition-colors ${
								facetsOpen || facetFilters.length > 0
									? "border-primary/40 bg-primary/10 text-foreground"
									: "text-muted-foreground hover:text-foreground"
							}`}
							onClick={() => {
								setFacetsOpen((open) => {
									if (!open) {
										setSelectedNode(null);
										setSelectedEdge(null);
										setSelectedEdgeKey(null);
									}
									return !open;
								});
							}}
							title={t(
								"summarizeAndFilterTheLoadedObjects",
								"Summarize and filter the loaded objects",
							)}
						>
							<BarChart3 className="h-3.5 w-3.5" />
							{t("facets", "Facets")}
						</button>

						<div className="ml-auto flex min-w-0 items-center gap-2">
							{warnings.length > 0 && !warningsDismissed && (
								<Popover>
									<PopoverTrigger asChild>
										<button
											type="button"
											className="flex items-center gap-1.5 rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-xs font-medium text-amber-600 dark:text-amber-400 whitespace-nowrap hover:bg-amber-500/20 transition-colors"
										>
											<AlertTriangle className="h-3.5 w-3.5" />
											{warnings.length} data warning
											{warnings.length !== 1 ? "s" : ""}
										</button>
									</PopoverTrigger>
									<PopoverContent align="end" className="w-80 p-0">
										<div className="flex items-center justify-between border-b px-3 py-2">
											<span className="text-xs font-semibold text-amber-600 dark:text-amber-400">
												{t("dataWarnings", "Data warnings")}
											</span>
											<button
												type="button"
												className="text-muted-foreground hover:text-foreground"
												onClick={() => setDismissedWarningsKey(warningsKey)}
												title={t("dismissWarnings", "Dismiss warnings")}
											>
												<X className="h-3.5 w-3.5" />
											</button>
										</div>
										<ul className="max-h-64 overflow-y-auto p-2 space-y-1">
											{warnings.map((warning) => (
												<li
													key={warning}
													className="rounded bg-amber-500/5 px-2 py-1.5 text-xs text-muted-foreground"
												>
													{warning}
												</li>
											))}
										</ul>
									</PopoverContent>
								</Popover>
							)}
							{censusSummary && (
								<span
									className="min-w-0 flex-1 truncate text-xs text-muted-foreground"
									title={censusSummary}
								>
									{censusSummary}
								</span>
							)}
							{/* Never suppressed by the census line: that line only names
							    labels whose population is known exactly, so it can read as a
							    complete description of a view that is nothing of the kind. */}
							{truncated && (
								<span className="text-xs text-amber-500 whitespace-nowrap">
									{t("resultTruncated", "Result truncated")}
								</span>
							)}
							{loading && (
								<span className="text-xs text-muted-foreground animate-pulse whitespace-nowrap">
									Loading...
								</span>
							)}
						</div>
					</div>
				)}

				{/* Active filter chips — the removable record of what the view omits */}
				{(facetFilters.length > 0 || manualHidden.size > 0) && (
					<div className="flex flex-wrap items-center gap-1.5 border-b bg-background px-2 py-1.5">
						{facetFilters.map((filter) => (
							<span
								key={filter.id}
								className="flex items-center gap-1 rounded-full border bg-muted/50 px-2 py-0.5 text-[11px]"
							>
								{filter.mode === "to" ? (
									<Filter className="h-3 w-3 text-primary" />
								) : (
									<FilterX className="h-3 w-3 text-destructive" />
								)}
								<span className="max-w-48 truncate">{filter.title}</span>
								<span className="tabular-nums text-muted-foreground">
									{filter.ids.size.toLocaleString()}
								</span>
								<button
									type="button"
									className="text-muted-foreground hover:text-foreground"
									onClick={() => removeFacetFilter(filter.id)}
									title={t("removeFilter", "Remove filter")}
								>
									<X className="h-3 w-3" />
								</button>
							</span>
						))}
						{manualHidden.size > 0 && (
							<span className="flex items-center gap-1.5 rounded-full border bg-muted/50 px-2 py-0.5 text-[11px]">
								{t("countObjectsHidden", "{{count}} objects hidden", {
									count: manualHidden.size,
								})}
								<button
									type="button"
									className="font-medium text-primary hover:underline"
									onClick={showHiddenNodes}
								>
									{t("show", "Show")}
								</button>
							</span>
						)}
					</div>
				)}

				{/* Query panel (collapsible) */}
				{showQuery && onRunCypher && (
					<div className="border-b p-2">
						<GraphQueryPanel
							onRunCypher={onRunCypher}
							results={cypherResults ?? null}
							loading={cypherLoading}
							error={cypherError}
							onAddToCanvas={onMergeSubgraph ? addCypherToCanvas : undefined}
							addToCanvasCount={cypherSubgraph?.nodes.length ?? 0}
						/>
					</div>
				)}

				{/* Canvas — zoom/fit/reset controls are rendered inside SigmaContainer */}
				<div className="flex-1 relative min-h-0">
					<GraphCanvas
						data={viewData}
						loading={loading}
						selectedNodeId={selectedNode?.id}
						selectedEdgeKey={selectedEdgeKey}
						highlightedNodeIds={effectiveHighlight}
						highlightedEdgeIds={
							pathHighlight ? (pathEdgeHighlight ?? undefined) : undefined
						}
						hiddenLabels={hiddenLabels.size > 0 ? hiddenLabels : undefined}
						visibleNodeIds={visibleNodeIds}
						clusters={layoutClusterModel}
						onNodeClick={handleNodeClick}
						onNodeShiftClick={handleNodeShiftClick}
						onNodeDoubleClick={handleNodeDoubleClick}
						onNodeContextMenu={handleNodeContextMenu}
						onEdgeClick={handleEdgeClick}
						onStageClick={handleStageClick}
						persistKey={persistKey}
						layoutCommand={layoutCommand}
						onCanvasApi={setCanvasApi}
						className="absolute inset-0"
					/>

					<GraphContextMenu
						state={contextMenu}
						node={contextNode}
						isGroup={
							contextMenu ? isCollapsedGroupId(contextMenu.nodeId) : false
						}
						collapsibleClusterId={contextClusterId}
						pinned={
							contextMenu
								? (canvasApi?.isPinned(contextMenu.nodeId) ?? false)
								: false
						}
						focused={
							contextMenu !== null && focus?.nodeId === contextMenu.nodeId
						}
						choices={contextChoices}
						onClose={closeContextMenu}
						onExpandChoice={
							onExpandNode && contextNode
								? (choice) => {
										void onExpandNode(
											contextNode.id,
											contextNode.label,
											getNodeRawId(contextNode, overlay),
											undefined,
											1,
											{
												edgeLabels: [choice.label],
												direction: choice.direction,
												limit: QUICK_EXPANSION_LIMIT,
											},
										);
									}
								: undefined
						}
						onExpandAll={
							onExpandNode && contextNode
								? () => {
										void onExpandNode(
											contextNode.id,
											contextNode.label,
											getNodeRawId(contextNode, overlay),
											undefined,
											1,
											{ limit: QUICK_EXPANSION_LIMIT },
										);
									}
								: undefined
						}
						onGuidedExpand={
							onExpandNode && contextNode
								? () => setExpansionTarget(contextNode)
								: undefined
						}
						onOpenGroup={
							contextMenu && isCollapsedGroupId(contextMenu.nodeId)
								? () => openCollapsedGroup(contextMenu.nodeId)
								: undefined
						}
						onCollapseGroup={
							contextClusterId
								? () =>
										setCollapsedGroups((prev) =>
											new Set(prev).add(contextClusterId),
										)
								: undefined
						}
						onToggleFocus={
							contextMenu ? () => focusNodeById(contextMenu.nodeId) : undefined
						}
						onHide={
							contextMenu ? () => hideNode(contextMenu.nodeId) : undefined
						}
						onTogglePin={
							canvasApi && contextMenu
								? () =>
										canvasApi.pinNode(
											contextMenu.nodeId,
											!canvasApi.isPinned(contextMenu.nodeId),
										)
								: undefined
						}
						onFindPath={
							onFindPaths && contextNode
								? () => handleArmPath(contextNode)
								: undefined
						}
					/>

					{/* Focus banner — the only way back out, so it is always on top */}
					{focus && focusedNodeIds && (
						<div className="absolute left-1/2 top-3 z-30 -translate-x-1/2">
							<div className="flex items-center gap-2 rounded-full border border-primary/40 bg-primary/10 px-3 py-1.5 text-xs shadow-sm backdrop-blur-sm">
								<Crosshair className="h-3.5 w-3.5 text-primary" />
								<span className="whitespace-nowrap">
									{focus.mode === "dim"
										? t("highlightingCountOfTotalObjectsAroundName", {
												defaultValue_one:
													"Highlighting {{count}} of {{total}} objects around {{name}}",
												defaultValue_other:
													"Highlighting {{count}} of {{total}} objects around {{name}}",
												count: focusedNodeIds.size,
												total: nodeCount,
												name:
													nodeMap.get(focus.nodeId)?.caption ?? focus.nodeId,
											})
										: t("showingCountOfTotalObjectsAroundName", {
												defaultValue_one:
													"Showing {{count}} of {{total}} objects around {{name}}",
												defaultValue_other:
													"Showing {{count}} of {{total}} objects around {{name}}",
												count: focusedNodeIds.size,
												total: nodeCount,
												name:
													nodeMap.get(focus.nodeId)?.caption ?? focus.nodeId,
											})}
								</span>
								<button
									type="button"
									className="whitespace-nowrap rounded-full border px-1.5 py-0.5 text-[10px] font-medium hover:bg-accent"
									onClick={() =>
										setFocus((prev) =>
											prev
												? { ...prev, depth: prev.depth === 1 ? 2 : 1 }
												: prev,
										)
									}
									title={t(
										"toggleBetweenOneAndTwoHops",
										"Toggle between one and two hops",
									)}
								>
									{focus.depth === 1
										? t("2Hops", "2 hops")
										: t("1Hop", "1 hop")}
								</button>
								<button
									type="button"
									className="whitespace-nowrap rounded-full border px-1.5 py-0.5 text-[10px] font-medium hover:bg-accent"
									onClick={() =>
										setFocus((prev) =>
											prev
												? {
														...prev,
														mode: prev.mode === "dim" ? "hide" : "dim",
													}
												: prev,
										)
									}
									title={t(
										"dimKeepsContextHideRemovesIt",
										"Dim keeps the rest as context; hide removes it",
									)}
								>
									{focus.mode === "dim"
										? t("hideOthers", "Hide others")
										: t("dimOthers", "Dim others")}
								</button>
								<button
									type="button"
									className="text-muted-foreground hover:text-foreground"
									onClick={() => setFocus(null)}
									title={t("exitFocus", "Exit focus")}
								>
									<X className="h-3.5 w-3.5" />
								</button>
							</div>
						</div>
					)}

					{/* Path-finding banner + result chip */}
					{(pathSource || pathOutcome || pathFinding) && (
						<div className="absolute left-1/2 top-3 z-20 -translate-x-1/2">
							{pathSource ? (
								<div className="flex items-center gap-2 rounded-full border bg-background/90 px-3 py-1.5 text-xs shadow-sm backdrop-blur-sm">
									<Route className="h-3.5 w-3.5 text-primary" />
									<span className="whitespace-nowrap">
										{t("findingPathFrom", "Finding path from")}{" "}
										<span className="font-medium">
											{pathSource.caption ?? pathSource.id}
										</span>{" "}
										{t("selectATargetNode", "— select a target node")}
									</span>
									<button
										type="button"
										className="text-muted-foreground hover:text-foreground"
										onClick={exitPathMode}
										title={t("cancelPathFinding", "Cancel path finding")}
									>
										<X className="h-3.5 w-3.5" />
									</button>
								</div>
							) : pathFinding ? (
								<div className="flex items-center gap-2 rounded-full border bg-background/90 px-3 py-1.5 text-xs shadow-sm backdrop-blur-sm">
									<Route className="h-3.5 w-3.5 animate-pulse text-primary" />
									<span className="whitespace-nowrap">
										{t("findingPath", "Finding path…")}
									</span>
								</div>
							) : pathOutcome ? (
								<div
									className={`flex items-center gap-2 rounded-full border px-3 py-1.5 text-xs shadow-sm backdrop-blur-sm ${
										pathOutcome.error
											? "border-destructive/40 bg-destructive/10 text-destructive"
											: pathOutcome.found
												? "border-primary/40 bg-primary/10 text-foreground"
												: "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400"
									}`}
								>
									<Route className="h-3.5 w-3.5" />
									<span className="whitespace-nowrap">
										{pathOutcome.error
											? pathOutcome.error
											: pathOutcome.found
												? t(
														"connectedHopsHopvalval2",
														"Connected — {{hops}} hop{{val}}{{val2}}",
														{
															hops: pathOutcome.hops,
															val: pathOutcome.hops !== 1 ? "s" : "",
															val2:
																pathOutcome.alternatives > 0
																	? ` (${pathOutcome.alternatives} alternative route${
																			pathOutcome.alternatives !== 1 ? "s" : ""
																		})`
																	: "",
														},
													)
												: t(
														"noConnectionWithin4Hops",
														"No connection within 4 hops",
													)}
									</span>
									<button
										type="button"
										className="hover:opacity-70"
										onClick={exitPathMode}
										title={t("clearPath", "Clear path")}
									>
										<X className="h-3.5 w-3.5" />
									</button>
								</div>
							) : null}
						</div>
					)}

					{/* Floating legend */}
					{showLegend && (
						<div className="absolute bottom-3 left-3 z-10">
							<GraphLegend
								entries={legendEntries}
								hidden={hiddenLabels}
								onToggleVisibility={handleToggleVisibility}
								onStyleChange={onStyleChange}
							/>
						</div>
					)}
				</div>
			</div>

			{/* Facet histogram (right drawer; the inspectors take precedence) */}
			{facetsOpen && !selectedNode && !selectedEdge && (
				<GraphHistogramPanel
					nodes={viewData?.nodes ?? []}
					onClose={() => {
						setFacetsOpen(false);
						setFacetHover(null);
					}}
					onHoverValue={setFacetHover}
					onFilterTo={(title, ids) => addFacetFilter("to", title, ids)}
					onFilterOut={(title, ids) => addFacetFilter("out", title, ids)}
				/>
			)}

			{/* Node inspector (right drawer) */}
			{showInspector && selectedNode && (
				<GraphNodeInspector
					node={selectedNode}
					overlay={overlay}
					connections={nodeConnections}
					onClose={() => setSelectedNode(null)}
					onConnectionClick={handleConnectionClick}
					onExpand={
						onExpandNode
							? (depth: number) =>
									onExpandNode(
										selectedNode.id,
										selectedNode.label,
										getNodeRawId(selectedNode, overlay),
										undefined,
										depth,
									)
							: undefined
					}
					onGuidedExpand={
						onExpandNode ? () => setExpansionTarget(selectedNode) : undefined
					}
					onFocus={handleFocus}
					focused={focus?.nodeId === selectedNode.id}
					hasChildren={hasContainmentChildren(selectedNode)}
					childrenExpanded={expandedChildParents?.has(selectedNode.id) ?? false}
					onExpandChildren={
						onExpandChildren
							? () =>
									onExpandChildren(
										selectedNode.id,
										selectedNode.label,
										getNodeRawId(selectedNode, overlay),
									)
							: undefined
					}
					onCollapseChildren={
						onCollapseChildren
							? () => onCollapseChildren(selectedNode.id)
							: undefined
					}
					onFindPath={onFindPaths ? handleArmPath : undefined}
					onRunAction={onRunAction}
				/>
			)}

			{/* Edge inspector (right drawer) */}
			{showInspector && selectedEdge && (
				<GraphEdgeInspector
					edge={selectedEdge}
					sourceCaption={edgeSourceCaption}
					targetCaption={edgeTargetCaption}
					onClose={() => {
						setSelectedEdge(null);
						setSelectedEdgeKey(null);
					}}
				/>
			)}

			<GraphExpansionDialog
				node={expansionTarget}
				overlay={overlay}
				choices={expansionChoices}
				maxLimit={limit ?? GRAPH_VIEW_LIMIT_OPTIONS[3]}
				onClose={() => setExpansionTarget(null)}
				onExpand={(options) => {
					if (!expansionTarget) return;
					onExpandNode?.(
						expansionTarget.id,
						expansionTarget.label,
						getNodeRawId(expansionTarget, overlay),
						undefined,
						1,
						options,
					);
				}}
			/>
		</div>
	);
}
