import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactFlowInstance } from "@xyflow/react";
import { useCallback, useRef } from "react";
import { type IBoard, type ILayer, ILayerType } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import type { ViewportHold } from "./use-viewport-manager";

const MAX_LAYER_DEPTH = 40;
/** How long a focus waits for the layer swap to reach the canvas before giving up. */
const FOCUS_RENDER_TIMEOUT_MS = 3_000;
/** Matches the fitView animation; the viewport hold outlives it so nothing overrides the landing. */
const FOCUS_ANIMATION_MS = 500;

/**
 * Ancestors of `layerId`, outermost first and ending with the layer itself — the exact
 * shape `layerPath` is stored in. Stops on a missing parent or a cycle so a damaged
 * `parent_id` chain degrades to a short path instead of looping.
 */
export function resolveLayerChain(
	layers: Record<string, ILayer>,
	layerId: string | null | undefined,
): string[] {
	const chain: string[] = [];
	const seen = new Set<string>();
	let currentId = layerId || undefined;

	while (currentId && chain.length < MAX_LAYER_DEPTH && !seen.has(currentId)) {
		const layer = layers[currentId];
		if (!layer) break;
		seen.add(currentId);
		chain.unshift(layer.id);
		currentId = layer.parent_id || undefined;
	}

	return chain;
}

function chainToPath(chain: string[]): string | undefined {
	return chain.length > 0 ? chain.join("/") : undefined;
}

/** The path one level up, or undefined when `path` is already a top-level layer. */
export function parentPath(path: string): string | undefined {
	const segments = path.split("/");
	return segments.length > 1 ? segments.slice(0, -1).join("/") : undefined;
}

/** One step the user took into a layer: the path they were on, and the one they opened. */
export interface LayerVisit {
	from: string | undefined;
	to: string;
}

/** Long enough for any real trail; a goto that never gets popped must not grow it forever. */
const MAX_TRAIL_LENGTH = 64;

/**
 * Appends a step, keeping the trail bounded. Re-opening the layer that is already on screen
 * is not a step — recording it would make that layer its own way out.
 */
export function recordVisit(
	trail: readonly LayerVisit[],
	visit: LayerVisit,
): LayerVisit[] {
	if (visit.from === visit.to) return [...trail];

	const next = [...trail, visit];
	return next.length > MAX_TRAIL_LENGTH
		? next.slice(next.length - MAX_TRAIL_LENGTH)
		: next;
}

/**
 * Where "layer up" lands, and the trail that is left behind. Functions hang off the board
 * root no matter which Call Function node opened them, so their parent chain leads to the
 * root rather than back to the caller — a function opened from inside another function
 * would drop the user all the way out. The trail retraces the way in instead, and the most
 * recent visit wins so entering the same function twice still unwinds one step at a time.
 *
 * Falls back to the parent chain whenever the trail does not describe where the user
 * currently is — they got there through a breadcrumb, a goto or a peer jump.
 */
export function resolveExit(
	trail: readonly LayerVisit[],
	layerPath: string,
): { path: string | undefined; trail: LayerVisit[] } {
	for (let index = trail.length - 1; index >= 0; index--) {
		if (trail[index].to !== layerPath) continue;
		return { path: trail[index].from, trail: trail.slice(0, index) };
	}

	return { path: parentPath(layerPath), trail: [] };
}

/**
 * Drops the trail from the last visit to `path` onward. A goto lands on a layer without
 * walking into it, so the steps recorded for it no longer describe how it was reached.
 */
export function dropVisitsTo(
	trail: readonly LayerVisit[],
	path: string,
): LayerVisit[] {
	for (let index = trail.length - 1; index >= 0; index--) {
		if (trail[index].to === path) return trail.slice(0, index);
	}
	return [...trail];
}

export interface FocusTarget {
	/** Layer chain to open, outermost first. Empty means the board root. */
	chain: string[];
	/** Rendered id to centre on. Undefined frames the whole opened layer instead. */
	renderTargetId?: string;
}

/**
 * Turns any board id into "which layer to open, and what to centre there". Accepts a node
 * id, a layer id or a function id — a go-to target can be any of the three, and every one
 * of them can sit arbitrarily deep inside layers and function bodies.
 */
export function resolveFocusTarget(
	nodes: Record<string, INode>,
	layers: Record<string, ILayer>,
	targetId: string,
): FocusTarget | undefined {
	const node = nodes[targetId];
	if (node) {
		return {
			chain: resolveLayerChain(layers, node.layer),
			renderTargetId: node.id,
		};
	}

	const layer = layers[targetId];
	if (!layer) return undefined;

	// A function body is never drawn on its parent's canvas, so the only way to go to one
	// is to open it. Every other layer is a real node in its parent — show it in context.
	if (layer.type === ILayerType.Function) {
		return { chain: resolveLayerChain(layers, layer.id) };
	}
	return {
		chain: resolveLayerChain(layers, layer.id).slice(0, -1),
		renderTargetId: layer.id,
	};
}

interface UseLayerNavigationProps {
	board: UseQueryResult<IBoard>;
	layerPath: string | undefined;
	setCurrentLayer: (layer: string | undefined) => void;
	setLayerPath: (path: string | undefined | ((old?: string) => string)) => void;
	saveViewport: () => Promise<void>;
	holdViewport: () => ViewportHold;
	fitView: ReactFlowInstance["fitView"];
	getNodes: ReactFlowInstance["getNodes"];
}

export function useLayerNavigation({
	board,
	layerPath,
	setCurrentLayer,
	setLayerPath,
	saveViewport,
	holdViewport,
	fitView,
	getNodes,
}: UseLayerNavigationProps) {
	/** The layers the user walked into, in order, so leaving them retraces the way back. */
	const trail = useRef<LayerVisit[]>([]);

	/**
	 * Navigates to anything addressable on the canvas: a node (in whichever layer or
	 * function body owns it), a layer, or a function. Every "go to" entry point — run
	 * logs, traces, search, function references, deep links, the assistant — funnels
	 * through here.
	 */
	const focusNode = useCallback(
		(targetId: string) => {
			const boardData = board.data;
			if (!boardData) return;

			const target = resolveFocusTarget(
				boardData.nodes,
				boardData.layers ?? {},
				targetId,
			);
			if (!target) {
				console.error("Node not found:", targetId);
				return;
			}

			const { chain, renderTargetId } = target;
			const targetPath = chainToPath(chain);
			const targetLayer =
				chain.length > 0 ? chain[chain.length - 1] : undefined;
			const switchesLayer = targetPath !== layerPath;

			// Leaving a layer discards what is on screen; keep its viewport so coming back
			// lands where the user left off.
			if (switchesLayer) void saveViewport();

			// A goto into another layer lands on its real place in the hierarchy, so the steps
			// recorded for it no longer describe how it was reached. Staying in the current
			// layer — jumping to a node inside it — leaves the way in untouched.
			if (switchesLayer && targetPath) {
				trail.current = dropVisitsTo(trail.current, targetPath);
			}

			const release = holdViewport();
			setCurrentLayer(targetLayer);
			setLayerPath(targetPath);

			// Proof the target layer is actually rendered: parseBoard always draws a layer's
			// `-input` boundary while that layer is open, and the target node itself in every
			// other case.
			const sentinelId =
				renderTargetId ?? (targetLayer ? `${targetLayer}-input` : undefined);
			const deadline = performance.now() + FOCUS_RENDER_TIMEOUT_MS;

			const focusRenderedNode = () => {
				const rendered = getNodes();
				const ready = sentinelId
					? rendered.some((renderedNode) => renderedNode.id === sentinelId)
					: rendered.length > 0;

				if (ready) {
					if (renderTargetId) {
						fitView({
							nodes: [{ id: renderTargetId }],
							padding: 0.35,
							duration: FOCUS_ANIMATION_MS,
							maxZoom: 1.2,
						});
					} else {
						fitView({
							padding: 0.2,
							duration: FOCUS_ANIMATION_MS,
							maxZoom: 1.2,
						});
					}
					// Held past the animation: the layer swap also changes the node count, and
					// that effect can still be queued behind this frame.
					setTimeout(release, FOCUS_ANIMATION_MS + 100);
					return;
				}

				if (performance.now() >= deadline) {
					console.warn("Failed to focus rendered node:", targetId);
					release();
					// The hold has already suppressed this layer's viewport restore, so frame
					// whatever did render rather than leaving the canvas wherever it was.
					fitView({ duration: 300 });
					return;
				}

				requestAnimationFrame(focusRenderedNode);
			};

			requestAnimationFrame(focusRenderedNode);
		},
		[
			board.data,
			layerPath,
			fitView,
			getNodes,
			holdViewport,
			saveViewport,
			setCurrentLayer,
			setLayerPath,
		],
	);

	const pushLayer = useCallback(
		async (pushedLayer: ILayer) => {
			await saveViewport();

			// Resolved rather than appended: functions are entered from the sidebar and from
			// Call Function nodes anywhere on the board, so the layer being opened is often
			// not a child of the one currently open.
			const chain = resolveLayerChain(board.data?.layers ?? {}, pushedLayer.id);
			const targetPath =
				chainToPath(chain) ??
				// Layer created in this session and not in the query cache yet.
				(layerPath ? `${layerPath}/${pushedLayer.id}` : pushedLayer.id);

			trail.current = recordVisit(trail.current, {
				from: layerPath,
				to: targetPath,
			});

			setCurrentLayer(pushedLayer.id);
			setLayerPath(targetPath);
		},
		[
			board.data?.layers,
			layerPath,
			saveViewport,
			setCurrentLayer,
			setLayerPath,
		],
	);

	const popLayer = useCallback(() => {
		if (!layerPath) return;

		void saveViewport();

		const exit = resolveExit(trail.current, layerPath);
		trail.current = exit.trail;

		setLayerPath(exit.path);
		setCurrentLayer(exit.path?.split("/").pop());
	}, [layerPath, saveViewport, setCurrentLayer, setLayerPath]);

	return {
		focusNode,
		pushLayer,
		popLayer,
	};
}
