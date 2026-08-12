"use client";

import { Suspense, lazy, useCallback } from "react";
import { cn } from "../../../lib/utils";
import type {
	SubgraphEdge,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { useActionContext, useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, OntologyGraphComponent } from "../types";

// Shares the lazily-loaded sigma chunk with the `graph` element.
const OntologyExplorer = lazy(() =>
	import("../../ui/graph/ontology-explorer").then((module) => ({
		default: module.OntologyExplorer,
	})),
);

const DEFAULT_HEIGHT = "480px";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function Placeholder({ message }: { message: string }) {
	return (
		<div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
			{message}
		</div>
	);
}

export function A2UIOntologyGraph({
	component,
	style,
	componentId,
	surfaceId,
	appId,
	onAction,
}: ComponentProps<OntologyGraphComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const { appId: contextAppId, isPreviewMode } = useActionContext();
	const ontologyId = useResolved<string>(component.ontologyId);
	const overrideAppId = useResolved<string>(component.appId);
	const rawLimit = useResolved<unknown>(component.limit);
	// A bound limit can arrive as a string; NaN would poison the initial load.
	const limit = Number(rawLimit) || undefined;
	const allowExpand = useResolved<boolean>(component.allowExpand) ?? true;
	const allowSearch = useResolved<boolean>(component.allowSearch) ?? true;
	const allowPaths = useResolved<boolean>(component.allowPaths) ?? true;
	const allowActions = useResolved<boolean>(component.allowActions) ?? true;
	const allowCypher = useResolved<boolean>(component.allowCypher) ?? false;
	const allowStyleEdit =
		useResolved<boolean>(component.allowStyleEdit) ?? false;
	const allowLimitChange =
		useResolved<boolean>(component.allowLimitChange) ?? true;
	const showToolbar = useResolved<boolean>(component.showToolbar) ?? true;
	const showLegend = useResolved<boolean>(component.showLegend) ?? true;
	// An empty bound height must not collapse the box to nothing.
	const height = useResolved<string>(component.height) || DEFAULT_HEIGHT;

	// Renderers that thread the owning project explicitly win; the surface's
	// action context covers the ones that only provide it through the provider,
	// such as the builder canvas.
	const targetAppId = overrideAppId || appId || contextAppId;

	// Read-only surfaces — the edit canvas, the admin page viewer — render live
	// data, but ontology actions and legend style edits write straight through to
	// the project, so they follow the same live/inert flag every action uses.
	const allowWrites = isPreviewMode === true;

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
				name: "nodeClick",
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
				name: "edgeClick",
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
			className={cn(
				"relative w-full overflow-hidden rounded-lg border border-border/50",
				resolveStyle(style),
			)}
			style={{ height, ...resolveInlineStyle(style) }}
		>
			{!targetAppId ? (
				<Placeholder message="No project context — open this surface inside a project, or bind the appId property to the project that owns the ontology." />
			) : !ontologyId ? (
				<Placeholder message="Select an ontology to display." />
			) : (
				<Suspense
					fallback={<div className="h-full w-full animate-pulse bg-muted/30" />}
				>
					<OntologyExplorer
						appId={targetAppId}
						overlayId={ontologyId}
						limit={limit}
						allowExpand={allowExpand}
						allowSearch={allowSearch}
						allowPaths={allowPaths}
						allowActions={allowActions && allowWrites}
						allowCypher={allowCypher}
						allowStyleEdit={allowStyleEdit && allowWrites}
						allowLimitChange={allowLimitChange}
						showToolbar={showToolbar}
						showLegend={showLegend}
						onNodeSelect={handleNodeSelect}
						onEdgeSelect={handleEdgeSelect}
					/>
				</Suspense>
			)}
		</div>
	);
}
