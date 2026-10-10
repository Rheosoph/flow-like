import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactFlowInstance } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import {
	type IBoard,
	type IComment,
	type ILayer,
	ILayerType,
} from "../lib/schema/flow/board";
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

/** Resolves a saved path against current ownership, retaining any surviving ancestor. */
export function resolveLayerPath(
	layers: Record<string, ILayer> | undefined,
	path: string | undefined,
): string | undefined {
	const segments =
		path?.split("/").filter((segment) => segment && segment !== "root") ?? [];
	if (!layers) return chainToPath(segments);
	for (let index = segments.length - 1; index >= 0; index--) {
		if (layers[segments[index]]) {
			return chainToPath(resolveLayerChain(layers, segments[index]));
		}
	}
	return undefined;
}

/** The path one level up, or undefined when `path` is already a top-level layer. */
export function parentPath(path: string): string | undefined {
	const segments = path.split("/");
	return segments.length > 1 ? segments.slice(0, -1).join("/") : undefined;
}

/** One step into a layer, including the caller to return to when leaving a function. */
export interface LayerVisit {
	from: string | undefined;
	to: string;
}

const MAX_TRAIL_LENGTH = 64;

/** Keeps only a connected trail. Old visits cannot become exits after a direct jump. */
export function recordVisit(
	trail: readonly LayerVisit[],
	visit: LayerVisit,
): LayerVisit[] {
	if (visit.from === visit.to) return [...trail];

	const connected = trail.at(-1)?.to === visit.from ? trail : [];
	const next = [...connected, visit];
	return next.slice(-MAX_TRAIL_LENGTH);
}

/** Returns to the caller when there is an active visit, otherwise to the owning layer. */
export function resolveExit(
	trail: readonly LayerVisit[],
	layerPath: string,
): { path: string | undefined; trail: LayerVisit[] } {
	const visit = trail.at(-1);
	if (visit?.to === layerPath) {
		return { path: visit.from, trail: trail.slice(0, -1) };
	}
	return { path: parentPath(layerPath), trail: [] };
}

/** Jumps within a layer branch keep its caller and follow the structural path inside it. */
export function reconcileLayerTrail(
	trail: readonly LayerVisit[],
	currentPath: string | undefined,
	targetPath: string | undefined,
): LayerVisit[] {
	if (currentPath === targetPath) return [...trail];
	if (!currentPath || !targetPath) return [];
	const currentSegments = currentPath.split("/");
	const targetSegments = targetPath.split("/");
	let sharedDepth = 0;
	while (
		sharedDepth < currentSegments.length &&
		currentSegments[sharedDepth] === targetSegments[sharedDepth]
	)
		sharedDepth++;
	if (sharedDepth === 0) return [];

	let parent = targetSegments.slice(0, sharedDepth).join("/");
	let next: LayerVisit[] = [];
	if (trail.at(-1)?.to === currentPath) {
		for (let index = trail.length - 1; index >= 0; index--) {
			if (trail[index].to !== parent) continue;
			next = trail.slice(0, index + 1);
			break;
		}
	}
	for (const segment of targetSegments.slice(sharedDepth)) {
		const to = `${parent}/${segment}`;
		next = recordVisit(next, { from: parent, to });
		parent = to;
	}
	return next;
}

export interface FocusTarget {
	/** Layer chain to open, outermost first. Empty means the board root. */
	chain: string[];
	/** Rendered id to centre on. Undefined frames the whole opened layer instead. */
	renderTargetId?: string;
}

/**
 * Turns any board id into "which layer to open, and what to centre there". Accepts a node
 * id, a layer id, a function id or a comment id — a go-to target can be any of the four,
 * and every one of them can sit arbitrarily deep inside layers and function bodies.
 */
export function resolveFocusTarget(
	nodes: Record<string, INode>,
	layers: Record<string, ILayer>,
	targetId: string,
	comments: Record<string, IComment> = {},
): FocusTarget | undefined {
	const node = nodes[targetId];
	if (node) {
		return {
			chain: resolveLayerChain(layers, node.layer),
			renderTargetId: node.id,
		};
	}

	// Comments are drawn by `parseBoard` in whichever layer owns them, so one is centred
	// exactly like a node once that layer is open.
	const comment = comments[targetId];
	if (comment) {
		return {
			chain: resolveLayerChain(layers, comment.layer ?? undefined),
			renderTargetId: comment.id,
		};
	}

	const layer = layers[targetId];
	if (!layer) return undefined;

	// Neither a function body nor a module is drawn on its parent's canvas, so the only way
	// to go to one is to open it — the layer itself is the destination. Every other layer is
	// a real node in its parent, so show it in context.
	if (layer.type === ILayerType.Function || layer.type === ILayerType.Module) {
		return { chain: resolveLayerChain(layers, layer.id) };
	}
	return {
		chain: resolveLayerChain(layers, layer.id).slice(0, -1),
		renderTargetId: layer.id,
	};
}

/**
 * The rendered id whose presence proves the focus target is on screen. `parseBoard` draws a
 * layer's `-input` boundary while that layer is open — but a layer with no pins has no
 * boundary to draw (a Module never does), and waiting for one that will never appear stalls
 * the focus until the timeout. Those fall back to [`isFocusRendered`]'s content check.
 */
export function focusSentinelId(
	layers: Record<string, ILayer>,
	targetLayer: string | undefined,
	renderTargetId: string | undefined,
): string | undefined {
	if (renderTargetId) return renderTargetId;
	if (!targetLayer) return undefined;
	const pins = layers[targetLayer]?.pins;
	return pins && Object.keys(pins).length > 0
		? `${targetLayer}-input`
		: undefined;
}

/**
 * Whether the canvas has caught up with the layer swap, so framing it lands on the right
 * content. Without a sentinel the only proof is the rendered set moving off what was on
 * screen when the focus started — fitting one frame too early would frame the layer the
 * user just left. Two empty canvases never differ, and an empty layer has nothing to frame
 * anyway, so that counts as arrived.
 */
export function isFocusRendered({
	renderedIds,
	sentinelId,
	baselineIds,
	switchesLayer,
}: {
	renderedIds: readonly string[];
	sentinelId: string | undefined;
	baselineIds: ReadonlySet<string>;
	switchesLayer: boolean;
}): boolean {
	if (sentinelId) return renderedIds.includes(sentinelId);
	if (!switchesLayer) return true;
	if (renderedIds.length !== baselineIds.size) return true;
	if (renderedIds.length === 0) return true;
	return renderedIds.some((id) => !baselineIds.has(id));
}

export interface NavigateToLayerOptions {
	/** A tab restore switches history before React renders the newly active tab. */
	navigationKey?: string;
	/** A new split starts with the caller trail of its source tab. */
	copyNavigationKey?: string;
	resetTrail?: boolean;
	saveViewport?: boolean;
}

interface LayerNavigationState {
	path: string | undefined;
	trail: LayerVisit[];
}

/** Module moves change paths while the visited functions keep their identity. */
function resolveNavigationState(
	history: LayerNavigationState,
	layers: Record<string, ILayer> | undefined,
): LayerNavigationState {
	let trail: LayerVisit[] = [];
	for (const visit of history.trail) {
		const from = resolveLayerPath(layers, visit.from);
		const to = resolveLayerPath(layers, visit.to);
		if (!to) {
			trail = [];
			continue;
		}
		trail = recordVisit(trail, { from, to });
	}
	return { path: resolveLayerPath(layers, history.path), trail };
}

interface UseLayerNavigationProps {
	navigationKey?: string;
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
	navigationKey = "board",
	board,
	layerPath,
	setCurrentLayer,
	setLayerPath,
	saveViewport,
	holdViewport,
	fitView,
	getNodes,
}: UseLayerNavigationProps) {
	const [navigationRevision, refreshNavigation] = useReducer(
		(revision: number) => revision + 1,
		0,
	);
	const histories = useRef(new Map<string, LayerNavigationState>());
	const activeKey = useRef(navigationKey);
	const renderedKey = useRef(navigationKey);
	const currentPath = useRef(layerPath);
	const pendingFocus = useRef<(() => void) | undefined>(undefined);
	const latest = useRef({
		board,
		saveViewport,
		holdViewport,
		fitView,
		getNodes,
	});
	latest.current = { board, saveViewport, holdViewport, fitView, getNodes };
	currentPath.current = layerPath;
	if (renderedKey.current !== navigationKey) {
		activeKey.current = navigationKey;
		renderedKey.current = navigationKey;
	}
	if (!histories.current.has(activeKey.current)) {
		histories.current.set(activeKey.current, { path: layerPath, trail: [] });
	}

	const cancelFocus = useCallback(() => {
		pendingFocus.current?.();
		pendingFocus.current = undefined;
	}, []);
	useEffect(() => cancelFocus, [cancelFocus]);

	const currentHistory = useCallback(() => {
		const layers = latest.current.board.data?.layers;
		const history = resolveNavigationState(
			histories.current.get(activeKey.current) ?? {
				path: currentPath.current,
				trail: [],
			},
			layers,
		);
		const path = resolveLayerPath(layers, currentPath.current);
		if (history.path !== path) {
			history.trail = reconcileLayerTrail(history.trail, history.path, path);
			history.path = path;
		}
		return history;
	}, []);

	const commitPath = useCallback(
		(path: string | undefined, history: LayerNavigationState) => {
			histories.current.set(activeKey.current, { path, trail: history.trail });
			currentPath.current = path;
			setCurrentLayer(path?.split("/").pop());
			setLayerPath(path);
			// A repeated function can have the same path at two different trail positions.
			refreshNavigation();
		},
		[setCurrentLayer, setLayerPath],
	);

	const navigateToLayer = useCallback(
		(target: string | undefined, options: NavigateToLayerOptions = {}) => {
			cancelFocus();
			const path = resolveLayerPath(latest.current.board.data?.layers, target);
			if (path !== currentPath.current && options.saveViewport !== false) {
				void latest.current.saveViewport();
			}
			let history = currentHistory();
			if (
				options.navigationKey &&
				options.navigationKey !== activeKey.current
			) {
				histories.current.set(activeKey.current, history);
				activeKey.current = options.navigationKey;
				const existing = histories.current.get(options.navigationKey);
				const source = options.copyNavigationKey
					? histories.current.get(options.copyNavigationKey)
					: undefined;
				history = source
					? { path: source.path, trail: [...source.trail] }
					: (existing ?? { path, trail: [] });
			}
			history = resolveNavigationState(
				history,
				latest.current.board.data?.layers,
			);
			history.trail = options.resetTrail
				? []
				: reconcileLayerTrail(history.trail, history.path, path);
			commitPath(path, history);
		},
		[cancelFocus, commitPath, currentHistory],
	);

	const forgetNavigation = useCallback((key: string) => {
		histories.current.delete(key);
	}, []);

	// Keep an open function and its callers attached when it moves to another module.
	useEffect(() => {
		const path = resolveLayerPath(board.data?.layers, currentPath.current);
		if (path !== currentPath.current) {
			cancelFocus();
			commitPath(path, currentHistory());
		}
	}, [board.data?.layers, cancelFocus, commitPath, currentHistory]);

	const returnToVisit = useCallback(
		(index: number) => {
			const history = currentHistory();
			if (
				!Number.isInteger(index) ||
				index < 0 ||
				index >= history.trail.length ||
				history.trail.at(-1)?.to !== history.path
			)
				return;
			cancelFocus();
			void latest.current.saveViewport();
			const path = history.trail[index].from;
			history.trail = history.trail.slice(0, index);
			commitPath(path, history);
		},
		[cancelFocus, commitPath, currentHistory],
	);

	/**
	 * Navigates to anything addressable on the canvas: a node (in whichever layer or
	 * function body owns it), a layer, a function, or a comment. Every "go to" entry
	 * point — run logs, traces, search, function references, the comments sidebar, deep
	 * links, the assistant — funnels through here.
	 */
	const focusNode = useCallback(
		(targetId: string) => {
			const { board, getNodes, holdViewport, fitView, saveViewport } =
				latest.current;
			const boardData = board.data;
			if (!boardData) return;

			const target = resolveFocusTarget(
				boardData.nodes,
				boardData.layers ?? {},
				targetId,
				boardData.comments ?? {},
			);
			if (!target) {
				console.error("Focus target not found:", targetId);
				return;
			}

			const { chain, renderTargetId } = target;
			const targetPath = chainToPath(chain);
			const targetLayer =
				chain.length > 0 ? chain[chain.length - 1] : undefined;
			const switchesLayer = targetPath !== currentPath.current;

			// Leaving a layer discards what is on screen; keep its viewport so coming back
			// lands where the user left off.
			if (switchesLayer) void saveViewport();

			cancelFocus();
			const history = currentHistory();
			history.trail =
				boardData.layers[targetId]?.type === ILayerType.Function && targetPath
					? recordVisit(history.trail, { from: history.path, to: targetPath })
					: reconcileLayerTrail(history.trail, history.path, targetPath);
			const baselineIds = new Set(getNodes().map((rendered) => rendered.id));

			const release = holdViewport();
			let cancelled = false;
			let frame: number | undefined;
			let releaseTimer: ReturnType<typeof setTimeout> | undefined;
			const cancel = () => {
				cancelled = true;
				if (frame !== undefined) cancelAnimationFrame(frame);
				if (releaseTimer !== undefined) clearTimeout(releaseTimer);
				release();
			};
			pendingFocus.current = cancel;
			commitPath(targetPath, history);

			const sentinelId = focusSentinelId(
				boardData.layers ?? {},
				targetLayer,
				renderTargetId,
			);
			const deadline = performance.now() + FOCUS_RENDER_TIMEOUT_MS;

			const focusRenderedNode = () => {
				if (cancelled) return;
				const renderedIds = getNodes().map((rendered) => rendered.id);
				const ready = isFocusRendered({
					renderedIds,
					sentinelId,
					baselineIds,
					switchesLayer,
				});

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
					releaseTimer = setTimeout(() => {
						release();
						if (pendingFocus.current === cancel)
							pendingFocus.current = undefined;
					}, FOCUS_ANIMATION_MS + 100);
					return;
				}

				if (performance.now() >= deadline) {
					console.warn("Failed to focus rendered node:", targetId);
					release();
					if (pendingFocus.current === cancel) pendingFocus.current = undefined;
					// The hold has already suppressed this layer's viewport restore, so frame
					// whatever did render rather than leaving the canvas wherever it was.
					fitView({ duration: 300 });
					return;
				}

				frame = requestAnimationFrame(focusRenderedNode);
			};

			frame = requestAnimationFrame(focusRenderedNode);
		},
		[cancelFocus, commitPath, currentHistory],
	);

	const pushLayer = useCallback(
		async (pushedLayer: ILayer) => {
			cancelFocus();
			const { board, saveViewport } = latest.current;
			const targetPath = chainToPath(
				resolveLayerChain(
					{
						...board.data?.layers,
						[pushedLayer.id]:
							board.data?.layers?.[pushedLayer.id] ?? pushedLayer,
					},
					pushedLayer.id,
				),
			);
			if (!targetPath || targetPath === currentPath.current) return;
			const saving = saveViewport();
			const history = currentHistory();
			history.trail = recordVisit(history.trail, {
				from: history.path,
				to: targetPath,
			});
			// Viewport persistence can finish later without overwriting a newer navigation.
			commitPath(targetPath, history);
			await saving;
		},
		[cancelFocus, commitPath, currentHistory],
	);

	const popLayer = useCallback(() => {
		cancelFocus();
		const path = currentPath.current;
		if (!path) return;
		void latest.current.saveViewport();
		const history = currentHistory();
		const exit = resolveExit(history.trail, history.path ?? path);
		history.trail = exit.trail;
		commitPath(
			resolveLayerPath(latest.current.board.data?.layers, exit.path),
			history,
		);
	}, [cancelFocus, commitPath, currentHistory]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: history lives in refs; these values invalidate its rendered snapshot.
	const navigationTrail = useMemo(
		() => currentHistory().trail,
		[
			board.data?.layers,
			layerPath,
			navigationKey,
			navigationRevision,
			currentHistory,
		],
	);

	return {
		focusNode,
		pushLayer,
		popLayer,
		navigateToLayer,
		forgetNavigation,
		navigationTrail,
		returnToVisit,
	};
}
