"use client";

import { useTranslation } from "@flow-like/locales";
import { Suspense, lazy, useCallback, useMemo } from "react";
import { cn } from "../../../lib/utils";
import type {
	LabelStyle,
	SubgraphEdge,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import {
	DEFAULT_LABEL_STYLE,
	buildOverlayFromSubgraph,
	enrichSubgraphWithStyles,
} from "../../ui/graph/subgraph-utils";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type {
	BoundValue,
	GraphComponent,
	GraphEdgeDef,
	GraphLabelStyleDef,
	GraphNodeDef,
} from "../types";

// Sigma pulls in a WebGL renderer, graphology and a stylesheet. Loading it on
// demand keeps it out of every page that merely imports the a2ui registry.
const GraphViewer = lazy(() =>
	import("../../ui/graph/graph-viewer").then((module) => ({
		default: module.GraphViewer,
	})),
);

const DEFAULT_HEIGHT = "480px";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function toLabelStyle(style: GraphLabelStyleDef | undefined): LabelStyle {
	return {
		color: style?.color ?? DEFAULT_LABEL_STYLE.color,
		icon: style?.icon ?? DEFAULT_LABEL_STYLE.icon,
		size:
			typeof style?.size === "number"
				? { mode: "fixed", value: style.size }
				: DEFAULT_LABEL_STYLE.size,
	};
}

/** Accepts both the keyed object form and the array form the update node emits. */
function normalizeLabelStyles(
	raw: unknown,
): Record<string, LabelStyle> | undefined {
	if (!raw || typeof raw !== "object") return undefined;

	if (Array.isArray(raw)) {
		const entries = raw.flatMap((entry) => {
			const style = entry as GraphLabelStyleDef & { label?: string };
			if (!style?.label) return [];
			return [[style.label, toLabelStyle(style)] as const];
		});
		return entries.length > 0 ? Object.fromEntries(entries) : undefined;
	}

	return Object.fromEntries(
		Object.entries(raw as Record<string, GraphLabelStyleDef>).map(
			([label, style]) => [label, toLabelStyle(style)],
		),
	);
}

function toSubgraphNodes(raw: GraphNodeDef[] | undefined): SubgraphNode[] {
	if (!Array.isArray(raw)) return [];
	return raw.flatMap((node) =>
		node?.id
			? [
					{
						id: String(node.id),
						label: node.label ?? "Node",
						caption: node.caption,
						props: node.props ?? {},
					},
				]
			: [],
	);
}

function toSubgraphEdges(raw: GraphEdgeDef[] | undefined): SubgraphEdge[] {
	if (!Array.isArray(raw)) return [];
	return raw.flatMap((edge, index) =>
		edge?.source && edge?.target
			? [
					{
						id: String(edge.id ?? `${edge.source}->${edge.target}#${index}`),
						source: String(edge.source),
						target: String(edge.target),
						label: edge.label ?? "",
						props: edge.props ?? {},
					},
				]
			: [],
	);
}

function GraphFallback({ height }: { height: string }) {
	return (
		<div
			className="w-full animate-pulse rounded-lg border border-border/50 bg-muted/30"
			style={{ height }}
		/>
	);
}

export function A2UIGraph({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps<GraphComponent>) {
	const { t } = useTranslation("common");
	const triggerEvent = useComponentEventTrigger(componentId);
	const nodes = useResolved<GraphNodeDef[]>(component.nodes);
	const edges = useResolved<GraphEdgeDef[]>(component.edges);
	const labelStyles = useResolved<unknown>(component.labelStyles);
	const showToolbar = useResolved<boolean>(component.showToolbar) ?? true;
	const showSearch = useResolved<boolean>(component.showSearch) ?? true;
	const showLegend = useResolved<boolean>(component.showLegend) ?? true;
	const showInspector = useResolved<boolean>(component.showInspector) ?? true;
	const height = useResolved<string>(component.height) ?? DEFAULT_HEIGHT;

	// resolve() re-parses literalJson into fresh arrays every render, so the
	// graph is rebuilt from value identity rather than object identity.
	const dataKey = useMemo(
		() => JSON.stringify([nodes ?? [], edges ?? [], labelStyles ?? null]),
		[nodes, edges, labelStyles],
	);

	const { overlay, data } = useMemo(() => {
		const [rawNodes, rawEdges, rawStyles] = JSON.parse(dataKey) as [
			GraphNodeDef[],
			GraphEdgeDef[],
			unknown,
		];
		const subgraphNodes = toSubgraphNodes(rawNodes);
		const subgraphEdges = toSubgraphEdges(rawEdges);
		const builtOverlay = buildOverlayFromSubgraph(
			subgraphNodes,
			subgraphEdges,
			{
				labelStyles: normalizeLabelStyles(rawStyles),
			},
		);
		return {
			overlay: builtOverlay,
			data: enrichSubgraphWithStyles(
				{ nodes: subgraphNodes, edges: subgraphEdges, truncated: false },
				builtOverlay,
			),
		};
	}, [dataKey]);

	const handleNodeSelect = useCallback(
		(node: SubgraphNode | null) => {
			const context = node
				? {
						nodeId: node.id,
						label: node.label,
						caption: node.caption ?? null,
						props: node.props,
					}
				: { nodeId: null };
			onAction?.({
				type: "userAction",
				name: `nodeClick`,
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context,
			});
			void triggerEvent("nodeClick", component, {
				event: "nodeClick",
				...context,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleEdgeSelect = useCallback(
		(edge: SubgraphEdge | null) => {
			const context = edge
				? {
						edgeId: edge.id,
						label: edge.label,
						source: edge.source,
						target: edge.target,
						props: edge.props,
					}
				: { edgeId: null };
			onAction?.({
				type: "userAction",
				name: `edgeClick`,
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context,
			});
			void triggerEvent("edgeClick", component, {
				event: "edgeClick",
				...context,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	return (
		<div
			ref={elementRef}
			className={cn(
				"relative w-full overflow-hidden rounded-lg border border-border/50",
				resolveStyle(style),
			)}
			style={{ height, ...resolveInlineStyle(style) }}
		>
			<Suspense fallback={<GraphFallback height="100%" />}>
				<GraphViewer
					overlay={overlay}
					data={data}
					showToolbar={showToolbar}
					showSearch={showSearch}
					showLegend={showLegend}
					showInspector={showInspector}
					onNodeSelect={handleNodeSelect}
					onEdgeSelect={handleEdgeSelect}
				/>
			</Suspense>
		</div>
	);
}
