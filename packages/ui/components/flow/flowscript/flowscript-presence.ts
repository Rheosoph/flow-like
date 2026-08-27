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
import {
	type FlowScriptAnchorIndex,
	type FlowScriptAnchorKind,
	anchorAtOrAbove,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";
import { sameScopeNodeIds } from "./flowscript-panel-state";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	FLOWSCRIPT_SCOPE_FIELD,
	FLOWSCRIPT_VIEWPORT_FIELD,
	FLOWSCRIPT_VIEW_FIELD,
	type FlowScriptCursorPayload,
	type FlowScriptViewportPayload,
	MAX_CLAIM_ANCHORS,
	MAX_SELECTION_ANCHORS,
	MAX_WIRE_DLINE,
	sanitizeClaimsForWire,
	sanitizeCursorForWire,
	sanitizeScopeForWire,
	sanitizeViewForWire,
	sanitizeViewportForWire,
} from "./flowscript-presence-protocol";

/** A canvas "active node" click counts as fresh this long (matches the canvas hook). */
const ACTIVE_NODE_FRESH_MS = 3000;

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
			const anchorIds = coveredAnchorIds(index, selection);
			payload.sel = {
				endDLine: selection.selectionStartLineNumber - endAnchor.line,
				endColumn: selection.selectionStartColumn,
				...(endAnchor.id !== anchor.id ? { endAnchorId: endAnchor.id } : {}),
				...(anchorIds.length > 0 ? { anchorIds } : {}),
			};
		}
	}
	return sanitizeCursorForWire(payload);
}

/**
 * Node/layer anchors a text selection spans — the statement the range starts
 * inside plus every anchored line through its end. A range that stops at
 * column 1 of a line (whole-line selections end there) does not claim that
 * line's statement. Variables are not canvas entities and are skipped.
 */
export function coveredAnchorIds(
	index: FlowScriptAnchorIndex,
	selection: FlowScriptEditorSelection,
	max = MAX_SELECTION_ANCHORS,
): string[] {
	const startFirst =
		selection.selectionStartLineNumber < selection.positionLineNumber ||
		(selection.selectionStartLineNumber === selection.positionLineNumber &&
			selection.selectionStartColumn <= selection.positionColumn);
	const [fromLine, toLine, toColumn] = startFirst
		? [
				selection.selectionStartLineNumber,
				selection.positionLineNumber,
				selection.positionColumn,
			]
		: [
				selection.positionLineNumber,
				selection.selectionStartLineNumber,
				selection.selectionStartColumn,
			];
	const lastLine = toColumn === 1 && toLine > fromLine ? toLine - 1 : toLine;
	const ids: string[] = [];
	const push = (candidate?: { id: string; kind: FlowScriptAnchorKind }) => {
		if (!candidate || candidate.kind === "variable") return;
		if (ids.includes(candidate.id) || ids.length >= max) return;
		ids.push(candidate.id);
	};
	push(anchorAtOrAbove(index, fromLine, MAX_WIRE_DLINE));
	for (const anchor of index.anchors) {
		if (anchor.line < fromLine) continue;
		if (anchor.line > lastLine) break;
		push(anchor);
	}
	return ids;
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

/* ── Viewport translation (pure, scroll-follow) ────────────────────────── */

/**
 * Translate the local editor's first visible line into its anchor-relative
 * wire form. `undefined` while the viewport starts above the first anchor
 * (the `use` block) — the publisher clears the field then.
 */
export function viewportToWire(
	index: FlowScriptAnchorIndex,
	firstVisibleLine: number,
	now: number,
): FlowScriptViewportPayload | undefined {
	const anchor = anchorAtOrAbove(index, firstVisibleLine, MAX_WIRE_DLINE);
	if (!anchor) return undefined;
	return sanitizeViewportForWire({
		anchor: { id: anchor.id, kind: anchor.kind },
		dLine: firstVisibleLine - anchor.line,
		ts: now,
	});
}

/**
 * Resolve a wire viewport to the line THIS client's render should scroll to
 * its top. `undefined` when the anchor is not rendered here (other file or
 * scope, entity deleted locally).
 */
export function resolveWireViewport(
	index: FlowScriptAnchorIndex,
	payload: FlowScriptViewportPayload,
	maxLine?: number,
): number | undefined {
	const anchorLine = index.firstLineById.get(payload.anchor.id);
	if (!anchorLine) return undefined;
	const line = anchorLine + payload.dLine;
	return typeof maxLine === "number"
		? Math.min(Math.max(line, 1), maxLine)
		: line;
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
	/** Another session of the local user (shown as "You", like on the canvas). */
	self?: boolean;
	cursor: FlowScriptCursorPayload;
}

export interface FlowScriptRemoteClaims {
	clientId: number;
	sub?: string;
	self?: boolean;
	anchorIds: string[];
}

export interface FlowScriptRemoteCanvasSelection {
	clientId: number;
	sub?: string;
	self?: boolean;
	nodeIds: string[];
	/** The node the peer just clicked on the canvas — only while fresh. */
	activeNodeId?: string;
	activeNodeTs?: number;
}

export interface FlowScriptRemoteScope {
	sub?: string;
	self?: boolean;
	/** Node ids of the peer's shared "edit selection" scope. */
	nodeIds: string[];
}

export interface FlowScriptRemoteViewport {
	sub?: string;
	self?: boolean;
	/** Top of the peer's editor viewport, anchor-relative. */
	viewport: FlowScriptViewportPayload;
}

export interface FlowScriptPresenceSnapshot {
	cursors: FlowScriptRemoteCursor[];
	claims: FlowScriptRemoteClaims[];
	canvasSelections: FlowScriptRemoteCanvasSelection[];
	scopes: Map<number, FlowScriptRemoteScope>;
	/** Peers' editor viewports keyed by clientId (scroll-follow). */
	viewports: Map<number, FlowScriptRemoteViewport>;
}

export interface FlowScriptPresenceStore {
	subscribe: (listener: () => void) => () => void;
	getSnapshot: () => FlowScriptPresenceSnapshot;
}

export const EMPTY_FLOWSCRIPT_PRESENCE: FlowScriptPresenceSnapshot = {
	cursors: [],
	claims: [],
	canvasSelections: [],
	scopes: new Map(),
	viewports: new Map(),
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
			p.sel?.endColumn !== n.sel?.endColumn ||
			(p.sel?.anchorIds ?? []).join(",") !== (n.sel?.anchorIds ?? []).join(",")
			// ts deliberately excluded: heartbeat re-broadcasts must not re-render
		)
			return false;
	}
	return true;
}

function scopesEqual(
	a: Map<number, FlowScriptRemoteScope>,
	b: Map<number, FlowScriptRemoteScope>,
): boolean {
	if (a.size !== b.size) return false;
	for (const [clientId, scope] of a) {
		const other = b.get(clientId);
		if (
			!other ||
			other.sub !== scope.sub ||
			other.self !== scope.self ||
			other.nodeIds.length !== scope.nodeIds.length ||
			other.nodeIds.some((id, i) => id !== scope.nodeIds[i])
			// ts is not stored: scope heartbeats must not re-render
		)
			return false;
	}
	return true;
}

function viewportsEqual(
	a: Map<number, FlowScriptRemoteViewport>,
	b: Map<number, FlowScriptRemoteViewport>,
): boolean {
	if (a.size !== b.size) return false;
	for (const [clientId, entry] of a) {
		const other = b.get(clientId);
		if (
			!other ||
			other.sub !== entry.sub ||
			other.self !== entry.self ||
			other.viewport.anchor.id !== entry.viewport.anchor.id ||
			other.viewport.anchor.kind !== entry.viewport.anchor.kind ||
			other.viewport.dLine !== entry.viewport.dLine
			// ts deliberately excluded: viewport heartbeats must not re-render
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

function canvasSelectionsEqual(
	a: FlowScriptRemoteCanvasSelection[],
	b: FlowScriptRemoteCanvasSelection[],
): boolean {
	if (
		!idListsEqual(
			a.map((c) => ({ ...c, ids: c.nodeIds })),
			b.map((c) => ({ ...c, ids: c.nodeIds })),
		)
	)
		return false;
	return a.every(
		(entry, i) =>
			entry.activeNodeId === b[i].activeNodeId &&
			entry.activeNodeTs === b[i].activeNodeTs,
	);
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
		/**
		 * Local user's sub. Our own OTHER sessions stay in the snapshot (the
		 * canvas shows them too, and a second window is how presence gets
		 * tried out) — they are only flagged `self` so the UI can say "You".
		 */
		selfSub?: string;
		raf?: (cb: () => void) => number;
		caf?: (handle: number) => void;
		now?: () => number;
		schedule?: (cb: () => void, ms: number) => unknown;
		cancel?: (handle: unknown) => void;
	},
): FlowScriptPresenceStore & { dispose: () => void } {
	const raf =
		options?.raf ?? ((cb: () => void) => requestAnimationFrame(() => cb()));
	const caf =
		options?.caf ?? ((handle: number) => cancelAnimationFrame(handle));
	const now = options?.now ?? Date.now;
	const schedule =
		options?.schedule ??
		((cb: () => void, ms: number) => setTimeout(cb, ms) as unknown);
	const cancel =
		options?.cancel ??
		((handle: unknown) =>
			clearTimeout(handle as ReturnType<typeof setTimeout>));
	const listeners = new Set<() => void>();
	let snapshot = EMPTY_FLOWSCRIPT_PRESENCE;
	let rafId: number | null = null;
	let disposed = false;
	// A canvas click is "fresh" for a few seconds after THIS client first saw
	// its timestamp change — peers' wall clocks are never compared to ours, and
	// whatever was already there when the store came up is old news, not a flash.
	const activeClicks = new Map<number, { ts: number; seenAt: number }>();
	let seeded = false;
	let expiryTimer: unknown | null = null;

	const recompute = () => {
		const states = awareness.getStates();
		const invalidPeers = (awareness as { __invalidPeers?: Set<number> })
			.__invalidPeers;
		const cursors: FlowScriptRemoteCursor[] = [];
		const claims: FlowScriptRemoteClaims[] = [];
		const canvasSelections: FlowScriptRemoteCanvasSelection[] = [];
		const scopeEntries: [number, FlowScriptRemoteScope][] = [];
		const viewportEntries: [number, FlowScriptRemoteViewport][] = [];
		const at = now();
		let nextExpiry = Number.POSITIVE_INFINITY;
		const liveClients = new Set<number>();
		states.forEach((state, clientId) => {
			if (clientId === awareness.clientID) return;
			if (invalidPeers?.has(clientId)) return;
			liveClients.add(clientId);
			const sub = typeof state?.sub === "string" ? state.sub : undefined;
			const self = Boolean(options?.selfSub && sub === options.selfSub);
			const cursor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
			if (cursor) cursors.push({ clientId, sub, self, cursor });
			const claim = sanitizeClaimsForWire(state?.[FLOWSCRIPT_CLAIMS_FIELD]);
			if (claim)
				claims.push({ clientId, sub, self, anchorIds: claim.anchorIds });
			const nodeIds = normalizeSelectionNodes(
				(state?.selection as { nodes?: unknown } | undefined)?.nodes,
			).sort();
			if (nodeIds.length > 0) {
				const activeNodeTs =
					typeof state?.activeNodeTs === "number"
						? state.activeNodeTs
						: undefined;
				let activeNodeId: string | undefined;
				if (activeNodeTs !== undefined) {
					const known = activeClicks.get(clientId);
					if (!known || known.ts !== activeNodeTs) {
						activeClicks.set(clientId, {
							ts: activeNodeTs,
							seenAt: seeded ? at : Number.NEGATIVE_INFINITY,
						});
					}
					const seenAt = activeClicks.get(clientId)?.seenAt ?? 0;
					const freshFor = seenAt + ACTIVE_NODE_FRESH_MS - at;
					if (
						freshFor > 0 &&
						typeof state?.activeNodeId === "string" &&
						nodeIds.includes(state.activeNodeId)
					) {
						activeNodeId = state.activeNodeId;
						nextExpiry = Math.min(nextExpiry, freshFor);
					}
				}
				canvasSelections.push({
					clientId,
					sub,
					self,
					nodeIds,
					...(activeNodeId ? { activeNodeId, activeNodeTs } : {}),
				});
			}
			const scope = sanitizeScopeForWire(state?.[FLOWSCRIPT_SCOPE_FIELD]);
			if (scope)
				scopeEntries.push([clientId, { sub, self, nodeIds: scope.nodeIds }]);
			const viewport = sanitizeViewportForWire(
				state?.[FLOWSCRIPT_VIEWPORT_FIELD],
			);
			if (viewport) viewportEntries.push([clientId, { sub, self, viewport }]);
		});
		cursors.sort((a, b) => a.clientId - b.clientId);
		claims.sort((a, b) => a.clientId - b.clientId);
		canvasSelections.sort((a, b) => a.clientId - b.clientId);
		scopeEntries.sort((a, b) => a[0] - b[0]);
		viewportEntries.sort((a, b) => a[0] - b[0]);
		const scopes = new Map(scopeEntries);
		const viewports = new Map(viewportEntries);
		seeded = true;
		for (const clientId of activeClicks.keys()) {
			if (!liveClients.has(clientId)) activeClicks.delete(clientId);
		}
		// Nothing on the wire changes when a click merely ages out, so the store
		// wakes itself up to drop the flag.
		if (expiryTimer !== null) {
			cancel(expiryTimer);
			expiryTimer = null;
		}
		if (Number.isFinite(nextExpiry)) {
			expiryTimer = schedule(() => {
				expiryTimer = null;
				scheduleRecompute();
			}, nextExpiry + 1);
		}

		const unchanged =
			cursorsEqual(snapshot.cursors, cursors) &&
			idListsEqual(
				snapshot.claims.map((c) => ({ ...c, ids: c.anchorIds })),
				claims.map((c) => ({ ...c, ids: c.anchorIds })),
			) &&
			canvasSelectionsEqual(snapshot.canvasSelections, canvasSelections) &&
			scopesEqual(snapshot.scopes, scopes) &&
			viewportsEqual(snapshot.viewports, viewports);
		if (unchanged) return;
		snapshot = { cursors, claims, canvasSelections, scopes, viewports };
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
			if (expiryTimer !== null) {
				cancel(expiryTimer);
				expiryTimer = null;
			}
			try {
				awareness.off("change", scheduleRecompute);
			} catch {}
			listeners.clear();
		},
	};
}

/* ── Shared scoped sessions (rule 2: node ids only) ────────────────────── */

/**
 * Peers whose broadcast scope is the SAME node set as `ownNodeIds` (set
 * equality — render order is irrelevant). One entry per user: a user's second
 * session in the same scope is not another "with" name.
 */
export function peersSharingFlowScriptScope(
	scopes: ReadonlyMap<number, FlowScriptRemoteScope>,
	ownNodeIds: readonly string[],
): { clientId: number; sub?: string; self?: boolean }[] {
	if (ownNodeIds.length === 0) return [];
	const peers: { clientId: number; sub?: string; self?: boolean }[] = [];
	const seen = new Set<string>();
	for (const [clientId, scope] of scopes) {
		if (!sameScopeNodeIds(scope.nodeIds, ownNodeIds)) continue;
		const key = scope.sub ?? `client:${clientId}`;
		if (seen.has(key)) continue;
		seen.add(key);
		peers.push({ clientId, sub: scope.sub, self: scope.self });
	}
	return peers;
}

/** First scope per user (clientId order), keyed by sub for the presence bar. */
export function deriveScopesBySub(
	scopes: ReadonlyMap<number, FlowScriptRemoteScope>,
): Map<string, string[]> {
	const bySub = new Map<string, string[]>();
	for (const scope of scopes.values()) {
		if (!scope.sub || bySub.has(scope.sub)) continue;
		bySub.set(scope.sub, scope.nodeIds);
	}
	return bySub;
}

function scopesBySubEqual(
	a: Map<string, string[]>,
	b: Map<string, string[]>,
): boolean {
	if (a.size !== b.size) return false;
	for (const [sub, nodeIds] of a) {
		const other = b.get(sub);
		if (
			!other ||
			other.length !== nodeIds.length ||
			other.some((id, i) => id !== nodeIds[i])
		)
			return false;
	}
	return true;
}

/**
 * Broadcast the local panel's scoped-session node ids while it is open in
 * scoped mode. Deliberately NOT tied to editor focus: the scope persists
 * across blur and stays on the wire until scope exit, panel close, or unmount
 * (the effect cleanup withdraws the field).
 */
export function useFlowScriptScopeBroadcast({
	awareness,
	enabled,
	nodeIds,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	enabled: boolean;
	nodeIds: readonly string[] | undefined;
}): void {
	// Content-keyed (sorted, deduped) so re-renders and id reordering never
	// re-publish; node ids match WIRE_ANCHOR_ID_PATTERN, so "," is safe.
	const scopeKey =
		enabled && nodeIds && nodeIds.length > 0
			? [...new Set(nodeIds)].sort().join(",")
			: "";
	useEffect(() => {
		if (!awareness || !scopeKey) return;
		const payload = sanitizeScopeForWire({
			nodeIds: scopeKey.split(","),
			ts: Date.now(),
		});
		if (!payload) return;
		awareness.setLocalStateField(FLOWSCRIPT_SCOPE_FIELD, payload);
		return () => {
			awareness.setLocalStateField(FLOWSCRIPT_SCOPE_FIELD, undefined);
		};
	}, [awareness, scopeKey]);
}

/**
 * Broadcast which FlowScript file the local editor shows while the panel is
 * open — `main` or a module layer id, never a name (rule 2). Withdrawn on
 * panel close/unmount so a peer list never lists a closed editor.
 */
export function useFlowScriptViewBroadcast({
	awareness,
	enabled,
	file,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	enabled: boolean;
	file: string | undefined;
}): void {
	useEffect(() => {
		if (!awareness || !enabled || !file) return;
		const payload = sanitizeViewForWire({ file, ts: Date.now() });
		if (!payload) return;
		awareness.setLocalStateField(FLOWSCRIPT_VIEW_FIELD, payload);
		return () => {
			awareness.setLocalStateField(FLOWSCRIPT_VIEW_FIELD, undefined);
		};
	}, [awareness, enabled, file]);
}

const EMPTY_PEER_SCOPES: Map<string, string[]> = new Map();

/**
 * Peers' shared scopes keyed by sub, for the canvas presence bar's
 * "Join code scope" action. rAF-coalesced like every awareness consumer;
 * updates React state only when a scope actually changed.
 */
export function useFlowScriptPeerScopes({
	awareness,
	sub,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
}): Map<string, string[]> {
	const [scopes, setScopes] =
		useState<Map<string, string[]>>(EMPTY_PEER_SCOPES);
	useEffect(() => {
		if (!awareness) {
			setScopes(EMPTY_PEER_SCOPES);
			return;
		}
		const store = createFlowScriptPresenceStore(awareness, { selfSub: sub });
		const apply = () => {
			const next = deriveScopesBySub(store.getSnapshot().scopes);
			setScopes((prev) => (scopesBySubEqual(prev, next) ? prev : next));
		};
		const unsubscribe = store.subscribe(apply);
		apply();
		return () => {
			unsubscribe();
			store.dispose();
			setScopes(EMPTY_PEER_SCOPES);
		};
	}, [awareness, sub]);
	return scopes;
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
			// Never flush inside Monaco's own event dispatch: a keystroke fires the
			// selection change in the same stack as the content change, before
			// React has committed the new text — an immediate flush would resolve
			// the caret against the anchor lines of the PREVIOUS document.
			const elapsed = now() - lastCursorPublishAt;
			cursorTimer = schedule(
				flushCursor,
				Math.max(0, cursorInterval - elapsed),
			);
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

/** Viewport broadcasts are rate-limited to ≤ 5Hz — a follower only needs the gist. */
const VIEWPORT_PUBLISH_MIN_INTERVAL_MS = 200;

export interface FlowScriptViewportPublisher {
	/** Throttled; publishes only when the anchor-relative top line changed. */
	publish: (firstVisibleLine: number) => void;
	/** Clears the field and cancels the pending tick (unmount/disable). */
	dispose: () => void;
}

export function createFlowScriptViewportPublisher(options: {
	awareness: AwarenessFieldSetter;
	getAnchorIndex: () => FlowScriptAnchorIndex;
	now?: () => number;
	schedule?: (cb: () => void, ms: number) => unknown;
	cancel?: (handle: unknown) => void;
	minIntervalMs?: number;
}): FlowScriptViewportPublisher {
	const now = options.now ?? Date.now;
	const schedule =
		options.schedule ??
		((cb: () => void, ms: number) => setTimeout(cb, ms) as unknown);
	const cancel =
		options.cancel ??
		((handle: unknown) =>
			clearTimeout(handle as ReturnType<typeof setTimeout>));
	const minInterval = options.minIntervalMs ?? VIEWPORT_PUBLISH_MIN_INTERVAL_MS;

	const CLEARED = "";
	let lastKey = CLEARED;
	let lastPublishAt = 0;
	let pendingLine: number | undefined;
	let timer: unknown | null = null;

	const flush = () => {
		timer = null;
		const line = pendingLine;
		if (typeof line === "undefined") return;
		pendingLine = undefined;
		const payload = viewportToWire(options.getAnchorIndex(), line, now());
		const key = payload
			? `${payload.anchor.id}:${payload.anchor.kind}:${payload.dLine}`
			: CLEARED;
		if (key === lastKey) return;
		lastKey = key;
		lastPublishAt = now();
		options.awareness.setLocalStateField(FLOWSCRIPT_VIEWPORT_FIELD, payload);
	};

	return {
		publish: (firstVisibleLine) => {
			pendingLine = firstVisibleLine;
			if (timer !== null) return;
			// Trailing edge, like the cursor: a scroll burst collapses onto one
			// tick and the layout has settled by the time it fires.
			const elapsed = now() - lastPublishAt;
			timer = schedule(flush, Math.max(0, minInterval - elapsed));
		},
		dispose: () => {
			if (timer !== null) {
				cancel(timer);
				timer = null;
			}
			pendingLine = undefined;
			if (lastKey === CLEARED) return;
			lastKey = CLEARED;
			options.awareness.setLocalStateField(
				FLOWSCRIPT_VIEWPORT_FIELD,
				undefined,
			);
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
 * the local cursor/selection (anchor-relative, throttled) and dirty-buffer
 * claims, and exposes the remote presence store. The caret stays on the wire
 * for as long as the panel is open — a blur (window switch, Monaco's own find
 * widget, a click on the canvas) is not "left the editor"; only unmount and
 * `enabled` going false withdraw it. Everything degrades to single-user when
 * `awareness` is absent or `enabled` is false (read-only views).
 *
 * `enabled` must be a STABLE gate: it sits in the effect deps, and every
 * flip tears down the store (blanking peer decorations) and the publisher
 * (withdrawing the local caret from every peer's screen).
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
	const buffersRef = useRef({ text, baseline });
	buffersRef.current = { text, baseline };

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
		];
		if (editor.hasTextFocus()) publishCurrent();
		// A rebuilt publisher starts with no claims on the wire; a buffer that is
		// already dirty must announce them again without waiting for a keystroke.
		publisher.scheduleClaims(
			buffersRef.current.baseline,
			buffersRef.current.text,
		);
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

interface ViewportEditorLike {
	onDidScrollChange: (cb: () => void) => { dispose: () => void };
	getVisibleRanges: () => { startLineNumber: number }[];
}

/**
 * Broadcast the top of the local editor viewport (anchor-relative, ≤ 5Hz,
 * change-gated) so a teammate can scroll-follow this session. Unlike the
 * cursor it is not tied to focus — what is on screen is what a follower
 * wants. Withdrawn on unmount and whenever `enabled` drops (read-only views).
 */
export function useFlowScriptViewportBroadcast({
	awareness,
	enabled,
	editor,
	anchorIndexRef,
}: {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	enabled: boolean;
	editor: ViewportEditorLike | null;
	anchorIndexRef: React.RefObject<FlowScriptAnchorIndex>;
}): void {
	useEffect(() => {
		if (!awareness || !enabled || !editor) return;
		const publisher = createFlowScriptViewportPublisher({
			awareness,
			getAnchorIndex: () => anchorIndexRef.current,
		});
		const publishCurrent = () => {
			const line = editor.getVisibleRanges()[0]?.startLineNumber;
			if (typeof line === "number") publisher.publish(line);
		};
		const disposable = editor.onDidScrollChange(publishCurrent);
		publishCurrent();
		return () => {
			disposable.dispose();
			publisher.dispose();
		};
	}, [awareness, enabled, editor, anchorIndexRef]);
}

/* ── Canvas hook: project peer editor presence onto board nodes ────────── */

export interface RemoteEditorParticipant {
	clientId: number;
	sub?: string;
	/** Another session of the local user. */
	self?: boolean;
	/** True when the peer's text cursor sits on this node (vs a claim only). */
	active: boolean;
	/** True when the peer's text selection spans this node's statement. */
	selected: boolean;
}

/** Cursor beats selection beats claim when one peer touches a node several ways. */
function participantRank(participant: RemoteEditorParticipant): number {
	return participant.active ? 2 : participant.selected ? 1 : 0;
}

export function deriveRemoteEditorsByNode(
	snapshot: FlowScriptPresenceSnapshot,
): Map<string, RemoteEditorParticipant[]> {
	const byNode = new Map<string, Map<string, RemoteEditorParticipant>>();
	const add = (nodeId: string, participant: RemoteEditorParticipant) => {
		const key = participant.sub ?? `client:${participant.clientId}`;
		const participants = byNode.get(nodeId) ?? new Map();
		byNode.set(nodeId, participants);
		const existing = participants.get(key);
		if (!existing || participantRank(participant) > participantRank(existing))
			participants.set(key, participant);
	};
	for (const cursor of snapshot.cursors) {
		const { clientId, sub, self } = cursor;
		if (cursor.cursor.anchor.kind !== "variable") {
			add(cursor.cursor.anchor.id, {
				clientId,
				sub,
				self,
				active: true,
				selected: true,
			});
		}
		for (const anchorId of cursor.cursor.sel?.anchorIds ?? []) {
			add(anchorId, { clientId, sub, self, active: false, selected: true });
		}
	}
	for (const claim of snapshot.claims) {
		for (const anchorId of claim.anchorIds)
			add(anchorId, {
				clientId: claim.clientId,
				sub: claim.sub,
				self: claim.self,
				active: false,
				selected: false,
			});
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
				participants[i].active !== other[i].active ||
				participants[i].selected !== other[i].selected
			)
				return false;
		}
	}
	return true;
}

/**
 * Projects peers' FlowScript editor presence onto the canvas as a
 * `remoteEditors` entry in the node data — cursor holders (`active`), nodes
 * inside a peer's text selection (`selected`) and claim holders alike.
 * flow-node.tsx renders it exactly like a canvas selection (peer-colored ring
 * + avatar chip), so a selection of code IS a selection of nodes to everyone
 * else. Node data, never a DOM class: React Flow rewrites the wrapper's
 * className on select/drag and would wipe an imperative outline.
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

	useEffect(() => {
		if (!awareness) return;
		const store = createFlowScriptPresenceStore(awareness, { selfSub: sub });

		const apply = () => {
			const next = deriveRemoteEditorsByNode(store.getSnapshot());
			if (remoteEditorsMapsEqual(byNodeRef.current, next)) return;
			byNodeRef.current = next;

			// biome-ignore lint/suspicious/noExplicitAny: React Flow node shape
			setNodes((nodes: any[]) => {
				if (nodes.length === 0) return nodes;
				let changed = false;
				const updated = nodes.map((node) => {
					if (
						node.type !== "node" &&
						node.type !== "callFunctionNode" &&
						node.type !== "layerNode"
					)
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
