"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useBackend } from "../../../state/backend-state";
import type {
	GraphAnalyticsResult,
	GraphOverlay,
	GraphPathsResult,
	InvokeOntologyActionPayload,
	LabelStyle,
	OntologyActionDefinition,
	OntologyActionRun,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import type { ExpansionOptions } from "./graph-expansion-dialog";
import { GraphViewer, getNodeRawId } from "./graph-viewer";
import {
	OntologyActionDialog,
	type OntologyActionTarget,
	extractGraphErrorMessage,
} from "./ontology-action-dialog";
import {
	applyStyleToOverlay,
	collectSubtree,
	enrichSubgraphWithStyles,
	mergeSubgraphData,
	removeSubtree,
} from "./subgraph-utils";

export const GRAPH_MAX_NODE_LIMIT = 10_000;
export const GRAPH_NODE_EXPANSION_LIMIT = 500;
export const GRAPH_SEARCH_MATCH_LIMIT = 12;
export const GRAPH_VIEW_LIMIT_MAX = GRAPH_MAX_NODE_LIMIT;
export const GRAPH_MAX_EXPANSION_DEPTH = 2;
const GRAPH_DEFAULT_LIMIT = 200;
/** Only the exact per-label totals are used, and those ignore this bound entirely. */
const GRAPH_ANALYTICS_EDGE_LIMIT = 2_000;
const STYLE_PERSIST_DEBOUNCE_MS = 500;

function isConflictError(err: unknown): boolean {
	const message = extractGraphErrorMessage(err).toLowerCase();
	return (
		message.includes("409") ||
		message.includes("conflict") ||
		message.includes("updated_at") ||
		message.includes("stale")
	);
}

export interface OntologyExplorerProps {
	appId: string;
	overlayId: string;
	/** Overrides the overlay's stored default node limit for the first load. */
	limit?: number;
	className?: string;
	/** Neighbour and containment expansion from the canvas and the inspector. */
	allowExpand?: boolean;
	/** Live search across the loaded graph and the full ontology. */
	allowSearch?: boolean;
	/** Shortest-path finding between two nodes. */
	allowPaths?: boolean;
	/** Invoking governed ontology actions on a node. */
	allowActions?: boolean;
	/** The Cypher query panel. */
	allowCypher?: boolean;
	/** Legend style edits, which are persisted back onto the shared overlay. */
	allowStyleEdit?: boolean;
	/** The node-limit selector in the toolbar. */
	allowLimitChange?: boolean;
	showToolbar?: boolean;
	showLegend?: boolean;
	onOverlayLoaded?: (overlay: GraphOverlay) => void;
	onNodeSelect?: (node: SubgraphNode | null) => void;
	onEdgeSelect?: (edge: SubgraphEdge | null) => void;
	onError?: (message: string) => void;
	/** Rendered instead of the built-in error card when the overlay cannot load. */
	renderError?: (message: string, retry: () => void) => React.ReactNode;
}

/**
 * The full ontology graph experience: loads an overlay, streams its subgraph and
 * wires every remote operation the viewer offers. Backs both the Data Studio
 * overlay view and the a2ui `ontologyGraph` element, so the two stay identical.
 */
export const OntologyExplorer: React.FC<OntologyExplorerProps> = ({
	appId,
	overlayId,
	limit: limitOverride,
	className,
	allowExpand = true,
	allowSearch = true,
	allowPaths = true,
	allowActions = true,
	allowCypher = false,
	allowStyleEdit = false,
	allowLimitChange = true,
	showToolbar = true,
	showLegend = true,
	onOverlayLoaded,
	onNodeSelect,
	onEdgeSelect,
	onError,
	renderError,
}) => {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const [overlay, setOverlay] = useState<GraphOverlay | null>(null);
	const [data, setData] = useState<SubgraphResult | null>(null);
	const [analytics, setAnalytics] = useState<GraphAnalyticsResult | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [cypherResults, setCypherResults] = useState<unknown[] | null>(null);
	const [cypherLoading, setCypherLoading] = useState(false);
	const [cypherError, setCypherError] = useState<string | null>(null);
	const [nodeLimit, setNodeLimit] = useState(
		limitOverride ?? GRAPH_DEFAULT_LIMIT,
	);
	const [actionTarget, setActionTarget] = useState<OntologyActionTarget | null>(
		null,
	);
	const [expandedChildren, setExpandedChildren] = useState<
		Map<string, Set<string>>
	>(new Map());

	const dataRef = useRef<SubgraphResult | null>(null);
	const overlayRef = useRef<GraphOverlay | null>(null);
	const styleTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
		new Map(),
	);
	const styleRevertRef = useRef<Map<string, LabelStyle>>(new Map());
	const initialLoadRequestRef = useRef(0);
	const overlayRequestRef = useRef(0);

	useEffect(() => {
		overlayRef.current = overlay;
	}, [overlay]);

	useEffect(() => {
		dataRef.current = data;
	}, [data]);

	const expandedChildParents = useMemo(
		() => new Set(expandedChildren.keys()),
		[expandedChildren],
	);

	useEffect(() => {
		const timers = styleTimersRef.current;
		return () => {
			for (const timer of timers.values()) clearTimeout(timer);
			timers.clear();
		};
	}, []);

	// Consumers routinely pass inline callbacks. Holding them in refs keeps the
	// loader identity stable, otherwise every parent render would refetch.
	const onErrorRef = useRef(onError);
	const onOverlayLoadedRef = useRef(onOverlayLoaded);
	useEffect(() => {
		onErrorRef.current = onError;
		onOverlayLoadedRef.current = onOverlayLoaded;
	}, [onError, onOverlayLoaded]);

	const reportError = useCallback((message: string) => {
		setError(message);
		onErrorRef.current?.(message);
	}, []);

	const loadInitialData = useCallback(
		async (currentOverlay: GraphOverlay, limit?: number) => {
			const requestId = ++initialLoadRequestRef.current;
			setLoading(true);
			setError(null);
			try {
				const graphLimit = Math.min(
					limit ?? currentOverlay.default_limit ?? GRAPH_DEFAULT_LIMIT,
					GRAPH_VIEW_LIMIT_MAX,
				);

				const result = await backend.graphState.subgraph(appId, overlayId, {
					seeds: [],
					depth: 1,
					limit: graphLimit,
				});
				if (initialLoadRequestRef.current !== requestId) return;
				setData(enrichSubgraphWithStyles(result, currentOverlay));

				// Deliberately after the paint, never alongside it: graph queries share
				// a small pool of connection permits, and the whole-population counts
				// are a caption on the view rather than a prerequisite for drawing it.
				void backend.graphState
					.analytics(appId, overlayId, GRAPH_ANALYTICS_EDGE_LIMIT)
					.then((result) => {
						if (initialLoadRequestRef.current === requestId) {
							setAnalytics(result);
						}
					})
					.catch(() => {
						// The census line is optional; the graph stays fully usable without it.
					});
			} catch (err) {
				if (initialLoadRequestRef.current !== requestId) return;
				reportError(extractGraphErrorMessage(err));
				setData({ nodes: [], edges: [], truncated: false });
			} finally {
				if (initialLoadRequestRef.current === requestId) {
					setLoading(false);
				}
			}
		},
		[backend.graphState, appId, overlayId, reportError],
	);

	const loadOverlay = useCallback(async () => {
		const requestId = ++overlayRequestRef.current;
		setError(null);
		setLoading(true);
		try {
			const currentOverlay = await backend.graphState.getOverlay(
				appId,
				overlayId,
			);
			if (overlayRequestRef.current !== requestId) return;
			setOverlay(currentOverlay);
			onOverlayLoadedRef.current?.(currentOverlay);
			const initialLimit = Math.min(
				limitOverride ?? currentOverlay.default_limit ?? GRAPH_DEFAULT_LIMIT,
				GRAPH_VIEW_LIMIT_MAX,
			);
			setNodeLimit(initialLimit);
			await loadInitialData(currentOverlay, initialLimit);
		} catch (err) {
			if (overlayRequestRef.current !== requestId) return;
			reportError(extractGraphErrorMessage(err));
			setLoading(false);
		}
	}, [
		backend.graphState,
		appId,
		overlayId,
		limitOverride,
		loadInitialData,
		reportError,
	]);

	useEffect(() => {
		void loadOverlay();
		return () => {
			overlayRequestRef.current += 1;
			initialLoadRequestRef.current += 1;
		};
	}, [loadOverlay]);

	const retry = useCallback(() => {
		void loadOverlay();
	}, [loadOverlay]);

	const handleRunCypher = useCallback(
		async (query: string) => {
			setCypherLoading(true);
			setCypherError(null);
			try {
				const results = await backend.graphState.cypher(appId, overlayId, {
					query,
				});
				setCypherResults(results);
			} catch (err) {
				setCypherError(extractGraphErrorMessage(err));
			} finally {
				setCypherLoading(false);
			}
		},
		[backend.graphState, appId, overlayId],
	);

	const handleExpandNode = useCallback(
		async (
			nodeId: string,
			label: string,
			rawId?: unknown,
			seedNode?: SubgraphNode,
			depth?: number,
			options?: ExpansionOptions,
		) => {
			if (!overlay) return;

			if (seedNode) {
				setData((prev) =>
					mergeSubgraphData(
						prev,
						enrichSubgraphWithStyles(
							{ nodes: [seedNode], edges: [], truncated: false },
							overlay,
						),
					),
				);
			}

			setLoading(true);
			try {
				const prefix = `${label}:`;
				const resolvedId =
					rawId ??
					(nodeId.startsWith(prefix) ? nodeId.slice(prefix.length) : nodeId);
				const resolvedDepth = Math.min(
					Math.max(1, depth ?? 1),
					GRAPH_MAX_EXPANSION_DEPTH,
				);
				const result = await backend.graphState.neighbors(appId, overlayId, {
					label,
					node_id: resolvedId,
					depth: resolvedDepth,
					direction: options?.direction ?? "both",
					limit: Math.min(
						options?.limit ?? GRAPH_NODE_EXPANSION_LIMIT,
						GRAPH_NODE_EXPANSION_LIMIT,
					),
					edge_labels: options?.edgeLabels,
				});
				const enriched = enrichSubgraphWithStyles(result, overlay);
				setData((prev) => mergeSubgraphData(prev, enriched));
			} catch (err) {
				toast.error(
					t(
						"failedToExpandNeighborsVal",
						"Failed to expand neighbors: {{val}}",
						{ val: extractGraphErrorMessage(err) },
					),
				);
			} finally {
				setLoading(false);
			}
		},
		[backend.graphState, appId, overlayId, overlay, t],
	);

	const handleExpandChildren = useCallback(
		async (nodeId: string, label: string, rawId?: unknown) => {
			if (!overlay) return;
			setLoading(true);
			try {
				const prefix = `${label}:`;
				const resolvedId =
					rawId ??
					(nodeId.startsWith(prefix) ? nodeId.slice(prefix.length) : nodeId);
				const result = await backend.graphState.children(appId, overlayId, {
					label,
					node_id: resolvedId,
					limit: GRAPH_NODE_EXPANSION_LIMIT,
				});

				const existingIds = new Set(
					(dataRef.current?.nodes ?? []).map((node) => node.id),
				);
				const insertedChildIds = new Set<string>();
				for (const edge of result.edges) {
					if (edge.source === nodeId && !existingIds.has(edge.target)) {
						insertedChildIds.add(edge.target);
					}
				}

				const enriched = enrichSubgraphWithStyles(result, overlay);
				setData((prev) => mergeSubgraphData(prev, enriched));

				if (insertedChildIds.size > 0) {
					setExpandedChildren((prev) => {
						const next = new Map(prev);
						const merged = new Set(next.get(nodeId) ?? []);
						for (const id of insertedChildIds) merged.add(id);
						next.set(nodeId, merged);
						return next;
					});
				}
			} catch (err) {
				toast.error(
					t("failedToExpandChildrenVal", "Failed to expand children: {{val}}", {
						val: extractGraphErrorMessage(err),
					}),
				);
			} finally {
				setLoading(false);
			}
		},
		[backend.graphState, appId, overlayId, overlay, t],
	);

	const handleCollapseChildren = useCallback(
		(parentNodeId: string) => {
			if (!expandedChildren.has(parentNodeId)) return;
			const removed = new Set<string>();
			collectSubtree(parentNodeId, expandedChildren, removed);
			if (removed.size > 0) {
				setData((prev) => removeSubtree(prev, removed));
			}
			setExpandedChildren((prev) => {
				const next = new Map(prev);
				next.delete(parentNodeId);
				for (const id of removed) next.delete(id);
				return next;
			});
		},
		[expandedChildren],
	);

	const handleSearchNodes = useCallback(
		async (query: string) =>
			backend.graphState.searchNodes(appId, overlayId, {
				query,
				limit: GRAPH_SEARCH_MATCH_LIMIT,
			}),
		[backend.graphState, appId, overlayId],
	);

	const handleLimitChange = useCallback(
		(newLimit: number) => {
			const clampedLimit = Math.min(newLimit, GRAPH_VIEW_LIMIT_MAX);
			setNodeLimit(clampedLimit);
			if (overlay) {
				void loadInitialData(overlay, clampedLimit);
			}
		},
		[overlay, loadInitialData],
	);

	const persistStyle = useCallback(
		async (label: string, type: "node" | "edge") => {
			const current = overlayRef.current;
			if (!current) return;
			const revertKey = `${type}:${label}`;
			try {
				const saved = await backend.graphState.updateOverlay(appId, overlayId, {
					expected_updated_at: current.updated_at,
					nodes: current.nodes,
					edges: current.edges,
				});
				styleRevertRef.current.delete(revertKey);
				overlayRef.current = saved;
				setOverlay(saved);
				setData((prev) =>
					prev ? enrichSubgraphWithStyles(prev, saved) : prev,
				);
			} catch (err) {
				const previousStyle = styleRevertRef.current.get(revertKey);
				styleRevertRef.current.delete(revertKey);
				const base = overlayRef.current;
				if (previousStyle && base) {
					const reverted = applyStyleToOverlay(
						base,
						label,
						type,
						previousStyle,
					);
					overlayRef.current = reverted;
					setOverlay(reverted);
					setData((prev) =>
						prev ? enrichSubgraphWithStyles(prev, reverted) : prev,
					);
				}
				toast.error(`Failed to save style: ${extractGraphErrorMessage(err)}`);
				if (isConflictError(err)) {
					try {
						const fresh = await backend.graphState.getOverlay(appId, overlayId);
						overlayRef.current = fresh;
						setOverlay(fresh);
						setData((prev) =>
							prev ? enrichSubgraphWithStyles(prev, fresh) : prev,
						);
					} catch {
						// The refetch is best-effort; the revert already restored a usable state.
					}
				}
			}
		},
		[backend.graphState, appId, overlayId],
	);

	const handleStyleChange = useCallback(
		(label: string, type: "node" | "edge", style: LabelStyle) => {
			const current = overlayRef.current;
			if (!current) return;
			const revertKey = `${type}:${label}`;

			if (!styleRevertRef.current.has(revertKey)) {
				const previousStyle =
					type === "node"
						? current.nodes.find((node) => node.label === label)?.style
						: current.edges.find((edge) => edge.label === label)?.style;
				if (previousStyle) styleRevertRef.current.set(revertKey, previousStyle);
			}

			const updatedOverlay = applyStyleToOverlay(current, label, type, style);
			overlayRef.current = updatedOverlay;
			setOverlay(updatedOverlay);
			setData((prev) =>
				prev ? enrichSubgraphWithStyles(prev, updatedOverlay) : prev,
			);

			const existingTimer = styleTimersRef.current.get(revertKey);
			if (existingTimer) clearTimeout(existingTimer);
			const timer = setTimeout(() => {
				styleTimersRef.current.delete(revertKey);
				void persistStyle(label, type);
			}, STYLE_PERSIST_DEBOUNCE_MS);
			styleTimersRef.current.set(revertKey, timer);
		},
		[persistStyle],
	);

	const handleFindPaths = useCallback(
		async (from: SubgraphNode, to: SubgraphNode): Promise<GraphPathsResult> => {
			const current = overlayRef.current;
			if (!current) {
				throw new Error("The overlay is still loading.");
			}
			try {
				const result = await backend.graphState.paths(appId, overlayId, {
					from_label: from.label,
					from_id: getNodeRawId(from, current),
					to_label: to.label,
					to_id: getNodeRawId(to, current),
					max_depth: 4,
				});
				if (result.nodes.length > 0 || result.edges.length > 0) {
					setData((prev) =>
						mergeSubgraphData(
							prev,
							enrichSubgraphWithStyles(
								{
									nodes: result.nodes,
									edges: result.edges,
									truncated: result.truncated,
									warnings: result.warnings,
								},
								current,
							),
						),
					);
				}
				return result;
			} catch (err) {
				toast.error(`Path search failed: ${extractGraphErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.graphState, appId, overlayId],
	);

	const handleRunAction = useCallback(
		(action: OntologyActionDefinition, node: SubgraphNode) => {
			setActionTarget({ action, node });
		},
		[],
	);

	const invokeNodeAction = useCallback(
		async (
			action: OntologyActionDefinition,
			node: SubgraphNode,
			parameters: Record<string, unknown>,
			onStatus?: (run: OntologyActionRun) => void,
		): Promise<OntologyActionRun> => {
			const current = overlayRef.current;
			if (!current) throw new Error("The overlay is still loading.");
			let payload: InvokeOntologyActionPayload = {
				object_refs: [
					{
						object_type: action.object_type,
						id: getNodeRawId(node, current),
					},
				],
				parameters,
				idempotency_key: createId(),
			};

			const isOffline = await backend.isOffline(appId);
			if (!isOffline && backend.eventState.checkOAuthRequirements) {
				const prerun = await backend.graphState.prerunOntologyAction(
					appId,
					overlayId,
					action.id,
				);
				const oauth = await backend.eventState.checkOAuthRequirements(
					appId,
					prerun.oauth_requirements,
				);
				if (oauth.missingProviders.length > 0) {
					window.dispatchEvent(
						new CustomEvent("flow:oauth-required", {
							detail: {
								missingProviders: oauth.missingProviders,
								appId,
								boardId: action.board_id ?? "",
								nodeId: action.start_node_id ?? "",
								payload,
							},
						}),
					);
					throw new Error(
						t(
							"oauthAuthorizationIsRequiredCompleteAuthorizationThenConfirmTheActionAgain",
							"OAuth authorization is required. Complete authorization, then confirm the action again.",
						),
					);
				}
				payload = { ...payload, oauth_tokens: oauth.tokens };
			}

			return backend.graphState.invokeOntologyAction(
				appId,
				overlayId,
				action.id,
				payload,
				onStatus,
			);
		},
		[appId, backend, backend.eventState, backend.graphState, overlayId, t],
	);

	if (error && !overlay) {
		if (renderError) return <>{renderError(error, retry)}</>;
		return (
			<div className="flex h-full min-h-0 items-center justify-center p-6">
				<div className="space-y-2 text-center">
					<p className="text-sm text-destructive">{error}</p>
					<Button variant="outline" onClick={retry}>
						{t("tryAgain", "Try again")}
					</Button>
				</div>
			</div>
		);
	}

	if (!overlay) {
		return (
			<div className="flex h-full min-h-0 items-center justify-center p-6">
				<span className="text-sm text-muted-foreground animate-pulse">
					{t("loadingOntology", "Loading ontology...")}
				</span>
			</div>
		);
	}

	return (
		<div className={className ?? "relative h-full min-h-0 w-full"}>
			{/* The overlay itself loaded — a failed subgraph fetch stays recoverable. */}
			{error && (
				<div
					role="alert"
					className="absolute inset-x-0 top-0 z-30 flex items-center justify-between gap-3 border-b border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
				>
					<span className="truncate">{error}</span>
					<Button
						variant="outline"
						size="sm"
						className="h-6 shrink-0 text-xs"
						onClick={retry}
					>
						{t("retry", "Retry")}
					</Button>
				</div>
			)}
			<GraphViewer
				overlay={overlay}
				data={data}
				loading={loading}
				truncated={data?.truncated}
				showToolbar={showToolbar}
				showSearch={allowSearch}
				showLegend={showLegend}
				onNodeSelect={onNodeSelect}
				onEdgeSelect={onEdgeSelect}
				onRunCypher={allowCypher ? handleRunCypher : undefined}
				cypherResults={cypherResults}
				cypherLoading={cypherLoading}
				cypherError={cypherError}
				onExpandNode={allowExpand ? handleExpandNode : undefined}
				onExpandChildren={allowExpand ? handleExpandChildren : undefined}
				onCollapseChildren={allowExpand ? handleCollapseChildren : undefined}
				expandedChildParents={expandedChildParents}
				onSearchNodes={allowSearch ? handleSearchNodes : undefined}
				onStyleChange={allowStyleEdit ? handleStyleChange : undefined}
				onLimitChange={allowLimitChange ? handleLimitChange : undefined}
				limit={nodeLimit}
				onFindPaths={allowPaths ? handleFindPaths : undefined}
				onRunAction={allowActions ? handleRunAction : undefined}
				analytics={analytics}
				enableClusterLayout
			/>
			<OntologyActionDialog
				target={actionTarget}
				overlay={overlay}
				onClose={() => setActionTarget(null)}
				onInvoke={invokeNodeAction}
			/>
		</div>
	);
};
