"use client";

import { Search, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	GraphOverlay,
	LabelStyle,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import { GraphCanvas } from "./graph-canvas";
import { GraphEdgeInspector } from "./graph-edge-inspector";
import { GraphLegend, type LegendEntry } from "./graph-legend";
import {
	type ConnectionInfo,
	GraphNodeInspector,
} from "./graph-node-inspector";
import { GraphQueryPanel } from "./graph-query-panel";

const GRAPH_VIEW_LIMIT_OPTIONS = [
	50, 100, 200, 500, 1000, 2500, 5000, 10000, 25000, 50000, 100000, 1000000,
] as const;

function formatGraphLimitOption(limit: number): string {
	if (limit >= 1000000) return `${limit / 1000000}m nodes`;
	if (limit >= 1000) return `${limit / 1000}k nodes`;
	return `${limit} nodes`;
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

function getNodeRawId(node: SubgraphNode, overlay: GraphOverlay): unknown {
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
	onExpandNode?: (
		nodeId: string,
		label: string,
		rawId?: unknown,
		seedNode?: SubgraphNode,
	) => void;
	onSearchNodes?: (query: string) => Promise<SubgraphNode[]>;
	onStyleChange?: (
		label: string,
		type: "node" | "edge",
		style: LabelStyle,
	) => void;
	onLimitChange?: (limit: number) => void;
	limit?: number;
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
	onSearchNodes,
	onStyleChange,
	onLimitChange,
	limit,
}: GraphViewerProps) {
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

	const nodeCount = data?.nodes.length ?? 0;
	const edgeCount = data?.edges.length ?? 0;

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
	}, [overlay, data]);

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

	const handleNodeClick = useCallback(
		(nodeId: string) => {
			const node = data?.nodes.find((n) => n.id === nodeId);
			if (node) {
				setSelectedNode((prev) => (prev?.id === node.id ? null : node));
				if (onExpandNode) {
					onExpandNode(node.id, node.label, getNodeRawId(node, overlay));
				}
			}
			setSelectedEdge(null);
			setSelectedEdgeKey(null);
		},
		[data, onExpandNode, overlay],
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
			const node = data?.nodes.find((candidate) => candidate.id === nodeId);
			onExpandNode?.(
				nodeId,
				label,
				node ? getNodeRawId(node, overlay) : undefined,
			);
		},
		[data, onExpandNode, overlay],
	);

	const handleConnectionClick = useCallback(
		(targetNodeId: string) => {
			const node = data?.nodes.find((n) => n.id === targetNodeId);
			if (node) {
				setSelectedNode(node);
				setSelectedEdge(null);
				setSelectedEdgeKey(null);
				if (onExpandNode) {
					onExpandNode(node.id, node.label, getNodeRawId(node, overlay));
				}
			}
		},
		[data, onExpandNode, overlay],
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
				<div className="flex items-center gap-2 p-2 border-b bg-background">
					{/* Live search */}
					<div className="relative flex-1 max-w-sm">
						<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
						<input
							type="text"
							value={searchQuery}
							onChange={(e) => handleSearch(e.target.value)}
							placeholder="Search loaded nodes, then fallback to full graph..."
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
										Searching full graph...
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
												onClick={() => void handleRemoteSearchMatchClick(node)}
											>
												<span className="text-sm font-medium truncate max-w-full">
													{node.caption ?? node.id}
												</span>
												<span className="text-[11px] text-muted-foreground truncate max-w-full">
													{node.label} · {node.id}
												</span>
											</button>
										))}
									</div>
								) : searchHighlight.size === 0 ? (
									<div className="px-3 py-2 text-xs text-muted-foreground">
										No nodes found in the full graph.
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
							{unloadedRemoteSearchMatches.length} more in graph
						</span>
					)}

					{hasRemoteSearchQuery && remoteSearchLoading && (
						<span className="text-xs text-muted-foreground whitespace-nowrap">
							Searching full graph...
						</span>
					)}

					<div className="h-5 w-px bg-border" />

					{/* Node / edge count */}
					<span className="text-xs text-muted-foreground whitespace-nowrap">
						{nodeCount.toLocaleString()} nodes · {edgeCount.toLocaleString()}{" "}
						edges
					</span>

					{/* Limit selector */}
					{onLimitChange && (
						<>
							<div className="h-5 w-px bg-border" />
							<select
								value={limit ?? 200}
								onChange={(e) => onLimitChange(Number(e.target.value))}
								className="h-8 text-xs rounded-md border bg-transparent px-2 focus:outline-none focus:ring-1 focus:ring-ring"
							>
								{GRAPH_VIEW_LIMIT_OPTIONS.map((option) => (
									<option key={option} value={option}>
										{formatGraphLimitOption(option)}
									</option>
								))}
							</select>
						</>
					)}

					<div className="h-5 w-px bg-border" />

					{onRunCypher && (
						<button
							type="button"
							className="text-xs text-muted-foreground hover:text-foreground px-2 py-1 rounded border whitespace-nowrap"
							onClick={() => setShowQuery(!showQuery)}
						>
							{showQuery ? "Hide Query" : "Query"}
						</button>
					)}

					{truncated && (
						<span className="text-xs text-amber-500 ml-auto whitespace-nowrap">
							Result truncated
						</span>
					)}
					{loading && (
						<span className="text-xs text-muted-foreground ml-auto animate-pulse whitespace-nowrap">
							Loading...
						</span>
					)}
				</div>

				{/* Query panel (collapsible) */}
				{showQuery && onRunCypher && (
					<div className="border-b p-2">
						<GraphQueryPanel
							onRunCypher={onRunCypher}
							results={cypherResults ?? null}
							loading={cypherLoading}
							error={cypherError}
						/>
					</div>
				)}

				{/* Canvas — zoom/fit/reset controls are rendered inside SigmaContainer */}
				<div className="flex-1 relative min-h-0">
					<GraphCanvas
						data={data}
						loading={loading}
						selectedNodeId={selectedNode?.id}
						selectedEdgeKey={selectedEdgeKey}
						highlightedNodeIds={
							searchHighlight.size > 0 ? searchHighlight : undefined
						}
						hiddenLabels={hiddenLabels.size > 0 ? hiddenLabels : undefined}
						onNodeClick={handleNodeClick}
						onNodeShiftClick={handleNodeShiftClick}
						onEdgeClick={handleEdgeClick}
						onStageClick={handleStageClick}
						className="absolute inset-0"
					/>

					{/* Floating legend */}
					<div className="absolute bottom-3 left-3 z-10">
						<GraphLegend
							entries={legendEntries}
							onToggleVisibility={handleToggleVisibility}
							onStyleChange={onStyleChange}
						/>
					</div>
				</div>
			</div>

			{/* Node inspector (right drawer) */}
			{selectedNode && (
				<GraphNodeInspector
					node={selectedNode}
					connections={nodeConnections}
					onClose={() => setSelectedNode(null)}
					onConnectionClick={handleConnectionClick}
					onExpand={
						onExpandNode
							? () =>
									onExpandNode(
										selectedNode.id,
										selectedNode.label,
										getNodeRawId(selectedNode, overlay),
									)
							: undefined
					}
				/>
			)}

			{/* Edge inspector (right drawer) */}
			{selectedEdge && (
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
		</div>
	);
}
