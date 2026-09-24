"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	addNodeCommand,
	connectPinsCommand,
} from "../lib/command/generic-command";
import {
	NODE_WIDTH,
	PIN_MARGIN_TOP,
	isRerouteNode,
	measureNodeBox,
	pinOffsetY,
	visiblePinsOf,
} from "../lib/flow-layout/measure";
import {
	anchorPins,
	buildSuggestionContext,
} from "../lib/node-suggestions/extract";
import {
	type ResolvedGhost,
	resolveGhosts,
} from "../lib/node-suggestions/resolve";
import {
	GHOST_ALTERNATIVES,
	GHOST_MIN_CONFIDENCE,
	type SuggestionContext,
	type SuggestionResult,
} from "../lib/node-suggestions/types";
import type { IBoard } from "../lib/schema/flow/board";
import type { IGenericCommand } from "../lib/schema/flow/board/commands/generic-command";
import { type INode, type IPin, IPinType } from "../lib/schema/flow/node";

export const GHOST_DEBOUNCE_MS = 150;
/** Horizontal gap between the anchor node and the ghost. */
export const GHOST_GAP = 80;
/** Output pins of the anchor asked for a suggestion; the most confident one is shown. */
export const GHOST_ANCHOR_PINS = 3;
/** Ranked types requested per pin; headroom for the ones the pin-type filter drops. */
export const GHOST_CANDIDATE_POOL = GHOST_ALTERNATIVES * 3;

type Point = { x: number; y: number };

export interface GhostSuggestionEngine {
	suggest(
		context: SuggestionContext,
		limit?: number,
	): Promise<SuggestionResult | undefined>;
	readonly status?: { ngram: boolean; neural: boolean };
	/** Fires on every status change; the open anchor is asked again when a model appears. */
	subscribe?: (listener: () => void) => () => void;
}

const loadedModels = (engine: GhostSuggestionEngine) =>
	engine.status
		? `${engine.status.ngram}:${engine.status.neural}`
		: Symbol("unknown");

/** The slice of a React Flow `InternalNode` the ghost geometry reads. */
export interface GhostAnchorGeometry {
	measured: { width?: number; height?: number };
	dragging?: boolean;
	internals: {
		positionAbsolute: Point;
		handleBounds?: {
			source?: GhostHandle[] | null;
			target?: GhostHandle[] | null;
		};
	};
}

interface GhostHandle {
	id?: string | null;
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface GhostPlacement {
	/** Centre of the anchor pin's handle. */
	source: Point;
	/** Centre of the ghost's target pin, level with `source`. */
	target: Point;
	/** Top-left of the ghost, and of the node an accept places. */
	origin: Point;
	size: { width: number; height: number };
	/** Distance from the ghost's top edge to its target pin row. */
	targetOffset: number;
}

export interface NodeSuggestionGhostView {
	anchorNodeId: string;
	anchorPin: IPin;
	ghost: ResolvedGhost;
	index: number;
	count: number;
	busy: boolean;
}

export interface NodeSuggestionGhostOptions {
	engine?: GhostSuggestionEngine;
	board?: IBoard;
	catalog?: INode[];
	selectedNodeIds: readonly string[];
	currentLayer?: string;
	/** Old version or a board the user cannot edit. */
	readOnly: boolean;
	enabled: boolean;
	executeCommands: (commands: IGenericCommand[]) => Promise<unknown>;
	selectNodes: (nodeIds: string[]) => void;
	getInternalNode: (id: string) => GhostAnchorGeometry | undefined;
}

export type GhostKeyAction = "accept" | "dismiss" | "next" | "previous";

interface GhostState {
	anchorNodeId: string;
	pinId: string;
	ghosts: ResolvedGhost[];
	index: number;
}

const layerKey = (layer: string | null | undefined) => layer || "";
const dismissKey = (nodeId: string, pinId: string) => `${nodeId}\u0000${pinId}`;

export function findGhostAnchor(
	board: IBoard,
	selectedNodeIds: readonly string[],
	currentLayer: string | null | undefined,
): INode | undefined {
	if (selectedNodeIds.length !== 1) return undefined;
	const node = board.nodes[selectedNodeIds[0]];
	if (!node || isRerouteNode(node)) return undefined;
	if (layerKey(node.layer) !== layerKey(currentLayer)) return undefined;
	return node;
}

function handleCentre(
	anchor: GhostAnchorGeometry,
	pinId: string,
): Point | undefined {
	const bounds = anchor.internals.handleBounds;
	const handle = [...(bounds?.source ?? []), ...(bounds?.target ?? [])].find(
		(value) => value.id === pinId,
	);
	if (!handle) return undefined;
	const { x, y } = anchor.internals.positionAbsolute;
	return {
		x: x + handle.x + handle.width / 2,
		y: y + handle.y + handle.height / 2,
	};
}

export function ghostTargetPin(ghost: ResolvedGhost): IPin | undefined {
	return visiblePinsOf(ghost.node)
		.filter(
			(pin) =>
				pin.name === ghost.targetPinName && pin.pin_type === IPinType.Input,
		)
		.sort((a, b) => a.index - b.index)[0];
}

/** Puts the ghost right of the anchor with its target pin level with the anchor pin. */
export function ghostPlacement(
	anchor: GhostAnchorGeometry,
	pinId: string,
	ghost: ResolvedGhost,
): GhostPlacement | undefined {
	const source = handleCentre(anchor, pinId);
	if (!source) return undefined;
	const targetPin = ghostTargetPin(ghost);
	const targetOffset = targetPin ? pinOffsetY(targetPin) : PIN_MARGIN_TOP;
	const origin = {
		x: Math.round(
			anchor.internals.positionAbsolute.x +
				(anchor.measured.width ?? NODE_WIDTH) +
				GHOST_GAP,
		),
		y: Math.round(source.y - targetOffset),
	};
	return {
		source,
		target: { x: origin.x, y: origin.y + targetOffset },
		origin,
		size: measureNodeBox(ghost.node),
		targetOffset,
	};
}

export function ghostKeyAction(event: {
	key: string;
	altKey: boolean;
	ctrlKey: boolean;
	metaKey: boolean;
	shiftKey: boolean;
}): GhostKeyAction | undefined {
	if (event.ctrlKey || event.metaKey || event.shiftKey) return undefined;
	if (event.altKey) {
		if (event.key === "ArrowDown") return "next";
		if (event.key === "ArrowUp") return "previous";
		return undefined;
	}
	if (event.key === "Tab") return "accept";
	if (event.key === "Escape") return "dismiss";
	return undefined;
}

const EDITABLE_TARGET =
	'input, textarea, select, [contenteditable]:not([contenteditable="false"]), .monaco-editor, [role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]';

interface KeyTarget {
	closest(selector: string): unknown;
	isContentEditable?: boolean;
	ownerDocument?: { body?: unknown; documentElement?: unknown } | null;
}

const isKeyTarget = (value: unknown): value is KeyTarget =>
	typeof value === "object" &&
	value !== null &&
	typeof (value as KeyTarget).closest === "function";

/** Board keys fire from the page itself or the canvas, never from a field, editor or dialog. */
export function isGhostKeyTarget(target: unknown): boolean {
	if (!isKeyTarget(target)) return true;
	const document = target.ownerDocument;
	if (target === document?.body || target === document?.documentElement)
		return true;
	if (target.isContentEditable || target.closest(EDITABLE_TARGET)) return false;
	return Boolean(target.closest(".react-flow"));
}

/**
 * One AddNode + ConnectPin batch, so accept is a single undo step. `addNodeCommand` re-keys the
 * pins of the node it is given, which is why the template is cloned and the target pin looked up
 * by name afterwards.
 */
export function buildGhostAcceptBatch({
	ghost,
	anchorNodeId,
	anchorPinId,
	position,
	currentLayer,
}: {
	ghost: ResolvedGhost;
	anchorNodeId: string;
	anchorPinId: string;
	position: Point;
	currentLayer?: string;
}): { commands: IGenericCommand[]; nodeId: string } | undefined {
	const clone = structuredClone(ghost.node);
	const added = addNodeCommand({
		node: { ...clone, coordinates: [position.x, position.y, 0] },
		current_layer: currentLayer,
	});
	const targetPin = ghostTargetPin({ ...ghost, node: added.node });
	if (!targetPin) return undefined;
	const connect = connectPinsCommand({
		from_node: anchorNodeId,
		from_pin: anchorPinId,
		to_node: added.node.id,
		to_pin: targetPin.id,
	});
	return { commands: [added.command, connect], nodeId: added.node.id };
}

/** Keeps the alternative the user cycled to when a refresh still offers it. */
function mergeRefresh(
	previous: GhostState | undefined,
	next: GhostState | undefined,
): GhostState | undefined {
	if (
		!previous ||
		!next ||
		previous.anchorNodeId !== next.anchorNodeId ||
		previous.pinId !== next.pinId
	)
		return next;
	const shown = previous.ghosts[previous.index]?.node.name;
	const index = next.ghosts.findIndex((ghost) => ghost.node.name === shown);
	return { ...next, index: Math.max(0, index) };
}

/**
 * Asks pin by pin, never in parallel: the engine answers only its latest `suggest` call and
 * resolves superseded ones with `undefined`. For the same reason a stale run stops asking.
 */
export async function suggestForAnchor({
	engine,
	board,
	anchorNodeId,
	catalogByName,
	dismissed,
	isStale,
}: {
	engine: GhostSuggestionEngine;
	board: IBoard;
	anchorNodeId: string;
	catalogByName: ReadonlyMap<string, INode>;
	dismissed: ReadonlySet<string>;
	isStale: () => boolean;
}): Promise<GhostState | undefined> {
	const node = board.nodes[anchorNodeId];
	if (!node) return undefined;
	const pins = anchorPins(board, anchorNodeId)
		.filter(({ pinId }) => !dismissed.has(dismissKey(anchorNodeId, pinId)))
		.slice(0, GHOST_ANCHOR_PINS);
	let best: GhostState | undefined;
	for (const { pinId } of pins) {
		if (isStale()) return undefined;
		const pin = node.pins[pinId];
		const context = buildSuggestionContext(board, anchorNodeId, pinId);
		if (!pin || !context) continue;
		const result = await engine.suggest(context, GHOST_CANDIDATE_POOL);
		if (!result) continue;
		const ghosts = resolveGhosts(
			result,
			{ node, pin },
			catalogByName,
			board.refs ?? {},
		);
		const confidence = ghosts[0]?.confidence ?? 0;
		if (confidence < GHOST_MIN_CONFIDENCE) continue;
		if (best && best.ghosts[0].confidence >= confidence) continue;
		best = { anchorNodeId, pinId, ghosts, index: 0 };
	}
	return best;
}

export function useNodeSuggestionGhost(options: NodeSuggestionGhostOptions) {
	const latest = useRef(options);
	latest.current = options;
	const {
		engine,
		board,
		catalog,
		selectedNodeIds,
		currentLayer,
		readOnly,
		enabled,
	} = options;
	const [state, setState] = useState<GhostState>();
	const [busy, setBusy] = useState(false);
	const [dismissedSelection, setDismissedSelection] =
		useState<readonly string[]>();
	const [engineRevision, setEngineRevision] = useState(0);
	const pending = useRef(false);
	const epoch = useRef(0);
	const dismissed = useRef(new Set<string>());

	const anchor = useMemo(
		() =>
			enabled && !readOnly && engine && board
				? findGhostAnchor(board, selectedNodeIds, currentLayer)
				: undefined,
		[enabled, readOnly, engine, board, selectedNodeIds, currentLayer],
	);
	const anchorNodeId = anchor?.id;
	// Escape hides every ghost of the anchor until the selection changes, not just the dismissed pin.
	const suppressed = dismissedSelection === selectedNodeIds;
	const catalogByName = useMemo(
		() => new Map((catalog ?? []).map((node) => [node.name, node])),
		[catalog],
	);

	useEffect(() => {
		if (!engine?.subscribe) return;
		let models = loadedModels(engine);
		return engine.subscribe(() => {
			const next = loadedModels(engine);
			if (next === models) return;
			models = next;
			setEngineRevision((value) => value + 1);
		});
	}, [engine]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `engineRevision` asks the open anchor again once the models change.
	useEffect(() => {
		const request = ++epoch.current;
		if (
			!anchorNodeId ||
			!engine ||
			!board ||
			catalogByName.size === 0 ||
			suppressed
		) {
			setState(undefined);
			return;
		}
		setState((previous) =>
			previous?.anchorNodeId === anchorNodeId ? previous : undefined,
		);
		const timer = setTimeout(() => {
			suggestForAnchor({
				engine,
				board,
				anchorNodeId,
				catalogByName,
				dismissed: dismissed.current,
				isStale: () => request !== epoch.current,
			})
				.catch(() => undefined)
				.then((next) => {
					if (request !== epoch.current) return;
					setState((previous) => mergeRefresh(previous, next));
				});
		}, GHOST_DEBOUNCE_MS);
		return () => clearTimeout(timer);
	}, [anchorNodeId, engine, board, catalogByName, suppressed, engineRevision]);

	const view = useMemo<NodeSuggestionGhostView | undefined>(() => {
		if (!state || !anchor || state.anchorNodeId !== anchor.id) return;
		const anchorPin = anchor.pins[state.pinId];
		if (!anchorPin || anchorPin.connected_to.length > 0) return;
		const ghost = state.ghosts[state.index];
		if (!ghost) return;
		return {
			anchorNodeId: anchor.id,
			anchorPin,
			ghost,
			index: state.index,
			count: state.ghosts.length,
			busy,
		};
	}, [state, anchor, busy]);
	const viewRef = useRef(view);
	viewRef.current = view;

	const placementOf = useCallback((current: NodeSuggestionGhostView) => {
		const internal = latest.current.getInternalNode(current.anchorNodeId);
		if (!internal || internal.dragging) return undefined;
		return ghostPlacement(internal, current.anchorPin.id, current.ghost);
	}, []);

	const dismiss = useCallback(() => {
		const current = viewRef.current;
		if (!current || pending.current) return;
		dismissed.current.add(
			dismissKey(current.anchorNodeId, current.anchorPin.id),
		);
		epoch.current++;
		setDismissedSelection(latest.current.selectedNodeIds);
		setState(undefined);
	}, []);

	const cycle = useCallback((step: 1 | -1) => {
		setState((previous) => {
			if (!previous || previous.ghosts.length < 2) return previous;
			const count = previous.ghosts.length;
			return { ...previous, index: (previous.index + step + count) % count };
		});
	}, []);

	const accept = useCallback(async () => {
		const current = viewRef.current;
		const { board, currentLayer, readOnly, executeCommands, selectNodes } =
			latest.current;
		if (!current || !board || readOnly || pending.current) return;
		const placement = placementOf(current);
		if (!placement) return;
		const batch = buildGhostAcceptBatch({
			ghost: current.ghost,
			anchorNodeId: current.anchorNodeId,
			anchorPinId: current.anchorPin.id,
			position: placement.origin,
			currentLayer,
		});
		if (!batch) return;
		pending.current = true;
		setBusy(true);
		try {
			const result = await executeCommands(batch.commands);
			if (result === undefined) return;
			epoch.current++;
			setState(undefined);
			selectNodes([batch.nodeId]);
		} catch {
			// The command pipeline reports failures; the ghost stays so the user can retry.
		} finally {
			pending.current = false;
			setBusy(false);
		}
	}, [placementOf]);

	const visible = view !== undefined;
	useEffect(() => {
		if (!visible) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.defaultPrevented || event.isComposing) return;
			const action = ghostKeyAction(event);
			if (!action || !isGhostKeyTarget(event.target)) return;
			const current = viewRef.current;
			if (!current || !placementOf(current)) return;
			event.preventDefault();
			event.stopPropagation();
			if (action === "accept") {
				if (!event.repeat) void accept();
			} else if (action === "dismiss") dismiss();
			else cycle(action === "next" ? 1 : -1);
		};
		document.addEventListener("keydown", onKeyDown, true);
		return () => document.removeEventListener("keydown", onKeyDown, true);
	}, [visible, placementOf, accept, dismiss, cycle]);

	return { view, accept, dismiss, cycle };
}
