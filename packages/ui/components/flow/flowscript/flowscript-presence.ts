"use client";

/**
 * FlowScript realtime presence: anchor-relative editor cursors, soft edit
 * claims, and cross-surface (canvas ↔ editor) presence projection.
 *
 * Positions travel anchor-relative (todo/flowscript-collab.md rule 2): a board
 * entity id plus a small line/column offset, resolved back to concrete editor
 * lines PER CLIENT against that client's own render — two peers' texts may
 * differ (scoped renders, different `use` blocks, local edits), so absolute
 * line numbers are never exchanged.
 */

import { useEffect, useRef, useState } from "react";
import { PEER_COLOR_COUNT, peerColorSlot } from "../../../hooks/use-peer-users";
import {
	type FlowScriptAnchorIndex,
	anchorAtOrAbove,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	type FlowScriptCursorPayload,
	MAX_CLAIM_ANCHORS,
	MAX_WIRE_DLINE,
	sanitizeClaimsForWire,
	sanitizeCursorForWire,
} from "./flowscript-presence-protocol";

/* ── Anchor-relative translation (pure) ────────────────────────────────── */

/** The four numbers Monaco exposes on a cursor selection. */
export interface FlowScriptEditorSelection {
	positionLineNumber: number;
	positionColumn: number;
	selectionStartLineNumber: number;
	selectionStartColumn: number;
}

/**
 * Translate a local editor cursor/selection into its anchor-relative wire form.
 * Returns `undefined` when no anchor owns the cursor line (e.g. inside the
 * `use` block above the first statement) — the publisher clears the field then.
 */
export function cursorToWire(
	index: FlowScriptAnchorIndex,
	selection: FlowScriptEditorSelection,
	now: number,
): FlowScriptCursorPayload | undefined {
	const anchor = anchorAtOrAbove(
		index,
		selection.positionLineNumber,
		MAX_WIRE_DLINE,
	);
	if (!anchor) return undefined;
	const payload: FlowScriptCursorPayload = {
		anchor: { id: anchor.id, kind: anchor.kind },
		dLine: selection.positionLineNumber - anchor.line,
		column: selection.positionColumn,
		ts: now,
	};
	const hasRange =
		selection.selectionStartLineNumber !== selection.positionLineNumber ||
		selection.selectionStartColumn !== selection.positionColumn;
	if (hasRange) {
		const endAnchor = anchorAtOrAbove(
			index,
			selection.selectionStartLineNumber,
			MAX_WIRE_DLINE,
		);
		if (endAnchor) {
			payload.sel = {
				endDLine: selection.selectionStartLineNumber - endAnchor.line,
				endColumn: selection.selectionStartColumn,
				...(endAnchor.id !== anchor.id ? { endAnchorId: endAnchor.id } : {}),
			};
		}
	}
	return sanitizeCursorForWire(payload);
}

export interface ResolvedFlowScriptCursor {
	lineNumber: number;
	column: number;
	/** Normalized so start ≤ end; present only for non-empty ranges. */
	selection?: {
		startLineNumber: number;
		startColumn: number;
		endLineNumber: number;
		endColumn: number;
	};
}

/**
 * Resolve a wire cursor against THIS client's render. Returns `undefined` when
 * the anchor is not present locally (other scope, entity deleted here).
 */
export function resolveWireCursor(
	index: FlowScriptAnchorIndex,
	payload: FlowScriptCursorPayload,
	maxLine?: number,
): ResolvedFlowScriptCursor | undefined {
	const clampLine = (line: number) =>
		typeof maxLine === "number" ? Math.min(Math.max(line, 1), maxLine) : line;
	const anchorLine = index.firstLineById.get(payload.anchor.id);
	if (!anchorLine) return undefined;
	const lineNumber = clampLine(anchorLine + payload.dLine);
	const resolved: ResolvedFlowScriptCursor = {
		lineNumber,
		column: payload.column,
	};
	if (payload.sel) {
		const endAnchorLine = index.firstLineById.get(
			payload.sel.endAnchorId ?? payload.anchor.id,
		);
		if (endAnchorLine) {
			const endLineNumber = clampLine(endAnchorLine + payload.sel.endDLine);
			const endColumn = payload.sel.endColumn;
			const cursorFirst =
				lineNumber < endLineNumber ||
				(lineNumber === endLineNumber && payload.column <= endColumn);
			resolved.selection = cursorFirst
				? {
						startLineNumber: lineNumber,
						startColumn: payload.column,
						endLineNumber,
						endColumn,
					}
				: {
						startLineNumber: endLineNumber,
						startColumn: endColumn,
						endLineNumber: lineNumber,
						endColumn: payload.column,
					};
		}
	}
	return resolved;
}

/* ── Claim derivation (pure) ───────────────────────────────────────────── */

/**
 * Anchors whose statements differ between the baseline render and the current
 * buffer: the changed line-span (common prefix/suffix line diff) is mapped to
 * its owning anchors in BOTH texts — edits claim the statement they touch,
 * deletions claim the statement that vanished.
 */
export function deriveClaimedAnchorIds(
	baseline: string,
	current: string,
	max = MAX_CLAIM_ANCHORS,
): string[] {
	if (baseline === current) return [];
	const baseLines = baseline.split("\n");
	const currentLines = current.split("\n");
	let prefix = 0;
	const maxPrefix = Math.min(baseLines.length, currentLines.length);
	while (prefix < maxPrefix && baseLines[prefix] === currentLines[prefix])
		prefix++;
	let suffix = 0;
	while (
		suffix < maxPrefix - prefix &&
		baseLines[baseLines.length - 1 - suffix] ===
			currentLines[currentLines.length - 1 - suffix]
	)
		suffix++;

	const claimed: string[] = [];
	const claim = (id: string | undefined) => {
		if (id && !claimed.includes(id) && claimed.length < max) claimed.push(id);
	};

	const currentIndex = parseFlowScriptAnchors(current);
	const currentFrom = prefix + 1;
	const currentTo = currentLines.length - suffix;
	for (let line = currentFrom; line <= currentTo; line++) {
		claim(anchorAtOrAbove(currentIndex, line)?.id);
	}
	// A pure insertion/deletion has an empty span on one side; the owning
	// statement still sits above the seam.
	if (currentTo < currentFrom) claim(anchorAtOrAbove(currentIndex, prefix)?.id);

	const baselineIndex = parseFlowScriptAnchors(baseline);
	const baseTo = baseLines.length - suffix;
	for (const anchor of baselineIndex.anchors) {
		if (anchor.line >= prefix + 1 && anchor.line <= baseTo) claim(anchor.id);
	}
	return claimed;
}

/* ── Claim collisions (pure) ───────────────────────────────────────────── */

/**
 * Board entity ids a command batch touches — the same id space claims live in
 * (node/variable/layer ids double as anchor ids). Comments have no anchors
 * and are ignored. Used by the advisory undo/apply collision toast (rule 3).
 */
export function collectCommandEntityIds(
	commands: readonly Record<string, unknown>[],
): Set<string> {
	const ids = new Set<string>();
	const addId = (value: unknown) => {
		if (typeof value === "string" && value.length > 0) ids.add(value);
	};
	const addEntity = (value: unknown) => {
		if (typeof value === "object" && value !== null) {
			addId((value as { id?: unknown }).id);
		}
	};
	const addEntities = (value: unknown) => {
		if (Array.isArray(value)) for (const entry of value) addEntity(entry);
	};
	for (const command of commands) {
		addId(command.node_id);
		addId(command.from_node);
		addId(command.to_node);
		if (Array.isArray(command.node_ids))
			for (const id of command.node_ids) addId(id);
		addEntity(command.node);
		addEntity(command.old_node);
		addEntity(command.variable);
		addEntity(command.old_variable);
		addEntity(command.layer);
		addEntity(command.old_layer);
		addEntities(command.new_nodes);
		addEntities(command.original_nodes);
		addEntities(command.connected_nodes);
		addEntities(command.nodes);
		addEntities(command.layers);
		addEntities(command.original_layers);
		addEntities(command.new_layers);
	}
	return ids;
}

/** First peer whose claimed anchors intersect `entityIds`; advisory only. */
export function findClaimCollision(
	claims: readonly FlowScriptRemoteClaims[],
	entityIds: ReadonlySet<string>,
): FlowScriptRemoteClaims | undefined {
	if (entityIds.size === 0) return undefined;
	return claims.find((claim) =>
		claim.anchorIds.some((anchorId) => entityIds.has(anchorId)),
	);
}

/**
 * One-shot snapshot of every peer's current claims straight from awareness —
 * for callers outside the panel (the board's undo path) that don't hold a
 * presence store. Same sanitization and self-filtering as the store.
 */
export function readPeerFlowScriptClaims(
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any,
	selfSub?: string,
): FlowScriptRemoteClaims[] {
	if (!awareness) return [];
	const claims: FlowScriptRemoteClaims[] = [];
	const states = (awareness as AwarenessLike).getStates();
	const invalidPeers = (awareness as { __invalidPeers?: Set<number> })
		.__invalidPeers;
	states.forEach((state, clientId) => {
		if (clientId === (awareness as AwarenessLike).clientID) return;
		if (invalidPeers?.has(clientId)) return;
		const sub = typeof state?.sub === "string" ? state.sub : undefined;
		if (selfSub && sub === selfSub) return;
		const claim = sanitizeClaimsForWire(state?.[FLOWSCRIPT_CLAIMS_FIELD]);
		if (claim) claims.push({ clientId, sub, anchorIds: claim.anchorIds });
	});
	return claims;
}

/* ── Remote presence store (rAF-coalesced, useSyncExternalStore-ready) ─── */

export interface FlowScriptRemoteCursor {
	clientId: number;
	sub?: string;
	cursor: FlowScriptCursorPayload;
}

export interface FlowScriptRemoteClaims {
	clientId: number;
	sub?: string;
	anchorIds: string[];
}

export interface FlowScriptRemoteCanvasSelection {
	clientId: number;
	sub?: string;
	nodeIds: string[];
}

export interface FlowScriptPresenceSnapshot {
	cursors: FlowScriptRemoteCursor[];
	claims: FlowScriptRemoteClaims[];
	canvasSelections: FlowScriptRemoteCanvasSelection[];
}

export interface FlowScriptPresenceStore {
	subscribe: (listener: () => void) => () => void;
	getSnapshot: () => FlowScriptPresenceSnapshot;
}

export const EMPTY_FLOWSCRIPT_PRESENCE: FlowScriptPresenceSnapshot = {
	cursors: [],
	claims: [],
	canvasSelections: [],
};

export const EMPTY_FLOWSCRIPT_PRESENCE_STORE: FlowScriptPresenceStore = {
	subscribe: () => () => {},
	getSnapshot: () => EMPTY_FLOWSCRIPT_PRESENCE,
};

interface AwarenessLike {
	clientID: number;
	getStates: () => Map<number, Record<string, unknown>>;
	on: (event: "change", cb: () => void) => void;
	off: (event: "change", cb: () => void) => void;
}

/** Same shape guard as lib/flow-board-utils' normalizeSelectionNodes — kept
 *  local because that module's import graph would cycle back into flow-node. */
function normalizeSelectionNodes(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter(
		(nodeId: unknown): nodeId is string => typeof nodeId === "string",
	);
}

function cursorsEqual(
	a: FlowScriptRemoteCursor[],
	b: FlowScriptRemoteCursor[],
): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		const p = a[i].cursor;
		const n = b[i].cursor;
		if (
			a[i].clientId !== b[i].clientId ||
			a[i].sub !== b[i].sub ||
			p.anchor.id !== n.anchor.id ||
			p.anchor.kind !== n.anchor.kind ||
			p.dLine !== n.dLine ||
			p.column !== n.column ||
			p.sel?.endAnchorId !== n.sel?.endAnchorId ||
			p.sel?.endDLine !== n.sel?.endDLine ||
			p.sel?.endColumn !== n.sel?.endColumn
			// ts deliberately excluded: heartbeat re-broadcasts must not re-render
		)
			return false;
	}
	return true;
}

function idListsEqual(
	a: { clientId: number; sub?: string; ids: string[] }[],
	b: { clientId: number; sub?: string; ids: string[] }[],
): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		if (
			a[i].clientId !== b[i].clientId ||
			a[i].sub !== b[i].sub ||
			a[i].ids.length !== b[i].ids.length ||
			a[i].ids.some((id, j) => id !== b[i].ids[j])
		)
			return false;
	}
	return true;
}

/**
 * Subscribable snapshot of every peer's FlowScript presence plus their canvas
 * selection. Mirrors the canvas cursorStore contract: awareness "change" bursts
 * are coalesced to one recompute per animation frame, and listeners only fire
 * when a relevant key actually changed (never on cursor heartbeats).
 */
export function createFlowScriptPresenceStore(
	awareness: AwarenessLike,
	options?: {
		/** Local user's sub — filters our own other sessions, not just this client. */
		selfSub?: string;
		raf?: (cb: () => void) => number;
		caf?: (handle: number) => void;
	},
): FlowScriptPresenceStore & { dispose: () => void } {
	const raf =
		options?.raf ?? ((cb: () => void) => requestAnimationFrame(() => cb()));
	const caf =
		options?.caf ?? ((handle: number) => cancelAnimationFrame(handle));
	const listeners = new Set<() => void>();
	let snapshot = EMPTY_FLOWSCRIPT_PRESENCE;
	let rafId: number | null = null;
	let disposed = false;

	const recompute = () => {
		const states = awareness.getStates();
		const invalidPeers = (awareness as { __invalidPeers?: Set<number> })
			.__invalidPeers;
		const cursors: FlowScriptRemoteCursor[] = [];
		const claims: FlowScriptRemoteClaims[] = [];
		const canvasSelections: FlowScriptRemoteCanvasSelection[] = [];
		states.forEach((state, clientId) => {
			if (clientId === awareness.clientID) return;
			if (invalidPeers?.has(clientId)) return;
			const sub = typeof state?.sub === "string" ? state.sub : undefined;
			if (options?.selfSub && sub === options.selfSub) return;
			const cursor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
			if (cursor) cursors.push({ clientId, sub, cursor });
			const claim = sanitizeClaimsForWire(state?.[FLOWSCRIPT_CLAIMS_FIELD]);
			if (claim) claims.push({ clientId, sub, anchorIds: claim.anchorIds });
			const nodeIds = normalizeSelectionNodes(
				(state?.selection as { nodes?: unknown } | undefined)?.nodes,
			).sort();
			if (nodeIds.length > 0) canvasSelections.push({ clientId, sub, nodeIds });
		});
		cursors.sort((a, b) => a.clientId - b.clientId);
		claims.sort((a, b) => a.clientId - b.clientId);
		canvasSelections.sort((a, b) => a.clientId - b.clientId);

		const unchanged =
			cursorsEqual(snapshot.cursors, cursors) &&
			idListsEqual(
				snapshot.claims.map((c) => ({ ...c, ids: c.anchorIds })),
				claims.map((c) => ({ ...c, ids: c.anchorIds })),
			) &&
			idListsEqual(
				snapshot.canvasSelections.map((c) => ({ ...c, ids: c.nodeIds })),
				canvasSelections.map((c) => ({ ...c, ids: c.nodeIds })),
			);
		if (unchanged) return;
		snapshot = { cursors, claims, canvasSelections };
		for (const listener of listeners) listener();
	};

	const scheduleRecompute = () => {
		if (rafId !== null || disposed) return;
		rafId = raf(() => {
			rafId = null;
			recompute();
		});
	};

	awareness.on("change", scheduleRecompute);
	recompute();

	return {
		subscribe: (listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		getSnapshot: () => snapshot,
		dispose: () => {
			disposed = true;
			if (rafId !== null) caf(rafId);
			try {
				awareness.off("change", scheduleRecompute);
			} catch {}
			listeners.clear();
		},
	};
}

/* ── Local presence publisher (throttled, change-gated) ────────────────── */

/** Cursor broadcasts are rate-limited to ≤ 20Hz (matching the canvas cursor). */
const CURSOR_PUBLISH_MIN_INTERVAL_MS = 50;
/** Claims re-derive on a lazy debounce — they only move on real edits. */
const CLAIMS_DEBOUNCE_MS = 500;

interface AwarenessFieldSetter {
	setLocalStateField: (field: string, value: unknown) => void;
}

export interface FlowScriptPresencePublisher {
	/** Throttled; publishes only when the anchor-relative position changed. */
	publishCursor: (selection: FlowScriptEditorSelection) => void;
	/** Blur/close: withdraw the cursor from the wire immediately. */
	clearCursor: () => void;
	/** Debounced claim recompute; a clean buffer clears any published claims. */
	scheduleClaims: (baseline: string, text: string) => void;
	/** Clears both fields and cancels timers (unmount/panel close). */
	dispose: () => void;
}

export function createFlowScriptPresencePublisher(options: {
	awareness: AwarenessFieldSetter;
	getAnchorIndex: () => FlowScriptAnchorIndex;
	now?: () => number;
	schedule?: (cb: () => void, ms: number) => unknown;
	cancel?: (handle: unknown) => void;
	cursorMinIntervalMs?: number;
	claimsDebounceMs?: number;
}): FlowScriptPresencePublisher {
	const now = options.now ?? Date.now;
	const schedule =
		options.schedule ??
		((cb: () => void, ms: number) => setTimeout(cb, ms) as unknown);
	const cancel =
		options.cancel ??
		((handle: unknown) =>
			clearTimeout(handle as ReturnType<typeof setTimeout>));
	const cursorInterval =
		options.cursorMinIntervalMs ?? CURSOR_PUBLISH_MIN_INTERVAL_MS;
	const claimsDebounce = options.claimsDebounceMs ?? CLAIMS_DEBOUNCE_MS;

	const CLEARED = "";
	let lastCursorKey = CLEARED;
	let lastCursorPublishAt = 0;
	let pendingSelection: FlowScriptEditorSelection | undefined;
	let cursorTimer: unknown | null = null;
	let lastClaimsKey = CLEARED;
	let pendingClaims: { baseline: string; text: string } | undefined;
	let claimsTimer: unknown | null = null;

	const cursorKeyOf = (payload: FlowScriptCursorPayload | undefined) =>
		payload
			? [
					payload.anchor.id,
					payload.anchor.kind,
					payload.dLine,
					payload.column,
					payload.sel?.endAnchorId ?? "",
					payload.sel?.endDLine ?? "",
					payload.sel?.endColumn ?? "",
				].join(":")
			: CLEARED;

	const flushCursor = () => {
		cursorTimer = null;
		const selection = pendingSelection;
		if (!selection) return;
		pendingSelection = undefined;
		const payload = cursorToWire(options.getAnchorIndex(), selection, now());
		const key = cursorKeyOf(payload);
		if (key === lastCursorKey) return;
		lastCursorKey = key;
		lastCursorPublishAt = now();
		options.awareness.setLocalStateField(FLOWSCRIPT_CURSOR_FIELD, payload);
	};

	const flushClaims = () => {
		claimsTimer = null;
		const pending = pendingClaims;
		if (!pending) return;
		pendingClaims = undefined;
		const ids =
			pending.baseline === pending.text
				? []
				: deriveClaimedAnchorIds(pending.baseline, pending.text);
		const payload =
			ids.length > 0
				? sanitizeClaimsForWire({ anchorIds: ids, ts: now() })
				: undefined;
		const key = payload ? payload.anchorIds.join(",") : CLEARED;
		if (key === lastClaimsKey) return;
		lastClaimsKey = key;
		options.awareness.setLocalStateField(FLOWSCRIPT_CLAIMS_FIELD, payload);
	};

	return {
		publishCursor: (selection) => {
			pendingSelection = selection;
			if (cursorTimer !== null) return;
			const elapsed = now() - lastCursorPublishAt;
			if (elapsed >= cursorInterval) {
				flushCursor();
				return;
			}
			cursorTimer = schedule(flushCursor, cursorInterval - elapsed);
		},
		clearCursor: () => {
			if (cursorTimer !== null) {
				cancel(cursorTimer);
				cursorTimer = null;
			}
			pendingSelection = undefined;
			if (lastCursorKey === CLEARED) return;
			lastCursorKey = CLEARED;
			options.awareness.setLocalStateField(FLOWSCRIPT_CURSOR_FIELD, undefined);
		},
		scheduleClaims: (baseline, text) => {
			pendingClaims = { baseline, text };
			if (claimsTimer !== null) cancel(claimsTimer);
			claimsTimer = schedule(flushClaims, claimsDebounce);
		},
		dispose: () => {
			if (cursorTimer !== null) {
				cancel(cursorTimer);
				cursorTimer = null;
			}
			if (claimsTimer !== null) {
				cancel(claimsTimer);
				claimsTimer = null;
			}
			pendingSelection = undefined;
			pendingClaims = undefined;
			if (lastCursorKey !== CLEARED) {
				lastCursorKey = CLEARED;
				options.awareness.setLocalStateField(
					FLOWSCRIPT_CURSOR_FIELD,
					undefined,
				);
			}
			if (lastClaimsKey !== CLEARED) {
				lastClaimsKey = CLEARED;
				options.awareness.setLocalStateField(
					FLOWSCRIPT_CLAIMS_FIELD,
					undefined,
				);
			}
		},
	};
}

/* ── Panel hook ────────────────────────────────────────────────────────── */

interface PresenceEditorLike {
	onDidChangeCursorSelection: (
		cb: (event: { selection: FlowScriptEditorSelection }) => void,
	) => { dispose: () => void };
	onDidFocusEditorText: (cb: () => void) => { dispose: () => void };
	onDidBlurEditorText: (cb: () => void) => { dispose: () => void };
	getSelection: () => FlowScriptEditorSelection | null;
	hasTextFocus: () => boolean;
}

/**
 * Wires the mounted FlowScript editor into the presence exchange: publishes
 * the local cursor/selection (anchor-relative, throttled, cleared on blur and
 * unmount) and dirty-buffer claims, and exposes the remote presence store.
 * Everything degrades to single-user when `awareness` is absent or `enabled`
 * is false (read-only views, the hidden twin of the double-mounted panel).
 */
export function useFlowScriptPresence({
	awareness,
	sub,
	enabled,
	editor,
	anchorIndexRef,
	text,
	baseline,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
	enabled: boolean;
	editor: PresenceEditorLike | null;
	anchorIndexRef: React.RefObject<FlowScriptAnchorIndex>;
	text: string;
	baseline: string;
}): { store: FlowScriptPresenceStore } {
	const [store, setStore] = useState<FlowScriptPresenceStore>(
		EMPTY_FLOWSCRIPT_PRESENCE_STORE,
	);
	const publisherRef = useRef<FlowScriptPresencePublisher | null>(null);

	useEffect(() => {
		if (!awareness || !enabled) {
			setStore(EMPTY_FLOWSCRIPT_PRESENCE_STORE);
			return;
		}
		const created = createFlowScriptPresenceStore(awareness, {
			selfSub: sub,
		});
		setStore(created);
		return () => {
			created.dispose();
			setStore(EMPTY_FLOWSCRIPT_PRESENCE_STORE);
		};
	}, [awareness, enabled, sub]);

	useEffect(() => {
		if (!awareness || !enabled || !editor) return;
		const publisher = createFlowScriptPresencePublisher({
			awareness,
			getAnchorIndex: () => anchorIndexRef.current,
		});
		publisherRef.current = publisher;
		const publishCurrent = () => {
			const selection = editor.getSelection();
			if (selection) publisher.publishCursor(selection);
		};
		const disposables = [
			editor.onDidChangeCursorSelection((event) => {
				// Programmatic moves (canvas-driven reveals, re-renders) are not the
				// user editing here — only a focused editor broadcasts its cursor.
				if (!editor.hasTextFocus()) return;
				publisher.publishCursor(event.selection);
			}),
			editor.onDidFocusEditorText(publishCurrent),
			editor.onDidBlurEditorText(() => publisher.clearCursor()),
		];
		if (editor.hasTextFocus()) publishCurrent();
		return () => {
			for (const disposable of disposables) disposable.dispose();
			publisher.dispose();
			publisherRef.current = null;
		};
	}, [awareness, enabled, editor, anchorIndexRef]);

	useEffect(() => {
		publisherRef.current?.scheduleClaims(baseline, text);
	}, [baseline, text]);

	return { store };
}

/* ── Canvas hook: project peer editor presence onto board nodes ────────── */

export interface RemoteEditorParticipant {
	clientId: number;
	sub?: string;
	/** True when the peer's text cursor sits on this node (vs a claim only). */
	active: boolean;
}

export function deriveRemoteEditorsByNode(
	snapshot: FlowScriptPresenceSnapshot,
): Map<string, RemoteEditorParticipant[]> {
	const byNode = new Map<string, Map<string, RemoteEditorParticipant>>();
	const add = (
		nodeId: string,
		clientId: number,
		sub: string | undefined,
		active: boolean,
	) => {
		const key = sub ?? `client:${clientId}`;
		const participants = byNode.get(nodeId) ?? new Map();
		byNode.set(nodeId, participants);
		const existing = participants.get(key);
		if (!existing || (active && !existing.active))
			participants.set(key, { clientId, sub, active });
	};
	for (const cursor of snapshot.cursors) {
		if (cursor.cursor.anchor.kind === "variable") continue;
		add(cursor.cursor.anchor.id, cursor.clientId, cursor.sub, true);
	}
	for (const claim of snapshot.claims) {
		for (const anchorId of claim.anchorIds)
			add(anchorId, claim.clientId, claim.sub, false);
	}
	const result = new Map<string, RemoteEditorParticipant[]>();
	for (const [nodeId, participants] of byNode) {
		result.set(
			nodeId,
			[...participants.values()].sort((a, b) =>
				(a.sub ?? String(a.clientId)).localeCompare(
					b.sub ?? String(b.clientId),
				),
			),
		);
	}
	return result;
}

function remoteEditorsMapsEqual(
	a: Map<string, RemoteEditorParticipant[]>,
	b: Map<string, RemoteEditorParticipant[]>,
): boolean {
	if (a.size !== b.size) return false;
	for (const [nodeId, participants] of a) {
		const other = b.get(nodeId);
		if (!other || other.length !== participants.length) return false;
		for (let i = 0; i < participants.length; i++) {
			if (
				participants[i].sub !== other[i].sub ||
				participants[i].clientId !== other[i].clientId ||
				participants[i].active !== other[i].active
			)
				return false;
		}
	}
	return true;
}

const PEER_OUTLINE_BASE_CLASS = "flowscript-peer-outline";

function outlineClassesFor(participant: RemoteEditorParticipant): string[] {
	const slot =
		peerColorSlot(participant.sub) ?? participant.clientId % PEER_COLOR_COUNT;
	return [PEER_OUTLINE_BASE_CLASS, `flowscript-peer-slot-${slot}`];
}

function setNodeOutline(nodeId: string, classes: string[] | undefined) {
	if (typeof document === "undefined") return;
	const element = document.querySelector(
		`.react-flow__node[data-id="${nodeId}"]`,
	);
	if (!element) return;
	element.classList.remove(
		PEER_OUTLINE_BASE_CLASS,
		...Array.from(
			{ length: PEER_COLOR_COUNT },
			(_, i) => `flowscript-peer-slot-${i}`,
		),
	);
	if (classes) element.classList.add(...classes);
}

/**
 * Projects peers' FlowScript editor presence onto the canvas: a peer-colored
 * outline (DOM class toggle, same mechanism as `flowscript-nav-highlight`) on
 * the node whose anchor holds their cursor, and a `remoteEditors` entry in the
 * node data (rendered as the "✎ name" badge in flow-node.tsx) for cursor and
 * claim holders alike.
 */
export function useFlowScriptCanvasPresence({
	awareness,
	sub,
	setNodes,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
	// biome-ignore lint/suspicious/noExplicitAny: React Flow setNodes updater
	setNodes: (updater: (nodes: any[]) => any[]) => void;
}): void {
	const byNodeRef = useRef<Map<string, RemoteEditorParticipant[]>>(new Map());
	const outlinedRef = useRef<Set<string>>(new Set());

	useEffect(() => {
		if (!awareness) return;
		const store = createFlowScriptPresenceStore(awareness, { selfSub: sub });

		const apply = () => {
			const next = deriveRemoteEditorsByNode(store.getSnapshot());
			if (remoteEditorsMapsEqual(byNodeRef.current, next)) return;
			byNodeRef.current = next;

			// Outline only the nodes actively holding a peer's text cursor.
			const nextOutlined = new Set<string>();
			for (const [nodeId, participants] of next) {
				const active = participants.find((p) => p.active);
				if (!active) continue;
				nextOutlined.add(nodeId);
				setNodeOutline(nodeId, outlineClassesFor(active));
			}
			for (const nodeId of outlinedRef.current) {
				if (!nextOutlined.has(nodeId)) setNodeOutline(nodeId, undefined);
			}
			outlinedRef.current = nextOutlined;

			// biome-ignore lint/suspicious/noExplicitAny: React Flow node shape
			setNodes((nodes: any[]) => {
				if (nodes.length === 0) return nodes;
				let changed = false;
				const updated = nodes.map((node) => {
					if (node.type !== "node" && node.type !== "callFunctionNode")
						return node;
					const participants = next.get(node.id);
					if (!participants && !node.data.remoteEditors) return node;
					if (participants === node.data.remoteEditors) return node;
					changed = true;
					return {
						...node,
						data: { ...node.data, remoteEditors: participants },
					};
				});
				return changed ? updated : nodes;
			});
		};

		const unsubscribe = store.subscribe(apply);
		apply();

		return () => {
			unsubscribe();
			store.dispose();
			for (const nodeId of outlinedRef.current)
				setNodeOutline(nodeId, undefined);
			outlinedRef.current = new Set();
			if (byNodeRef.current.size > 0) {
				byNodeRef.current = new Map();
				// biome-ignore lint/suspicious/noExplicitAny: React Flow node shape
				setNodes((nodes: any[]) => {
					if (nodes.length === 0) return nodes;
					let changed = false;
					const updated = nodes.map((node) => {
						if (!node.data?.remoteEditors) return node;
						changed = true;
						return {
							...node,
							data: { ...node.data, remoteEditors: undefined },
						};
					});
					return changed ? updated : nodes;
				});
			}
		};
	}, [awareness, sub, setNodes]);
}
