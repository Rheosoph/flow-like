import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	FLOWSCRIPT_SCOPE_FIELD,
	FLOWSCRIPT_VIEW_FIELD,
	type FlowScriptAnchorWireKind,
	sanitizeClaimsForWire,
	sanitizeCursorForWire,
	sanitizeScopeForWire,
	sanitizeViewForWire,
} from "../../components/flow/flowscript/flowscript-presence-protocol";
import {
	CHAT_TYPING_FIELD,
	LAST_EDIT_FIELD,
	LAST_RUN_FIELD,
	type LastEditPayload,
	type LastRunPayload,
	TYPING_TTL_MS,
	sanitizeChatTyping,
	sanitizeLastEdit,
	sanitizeLastRun,
} from "./presence-signals";

/** A canvas "active node" click stays fresh this long. */
export const ACTIVE_NODE_FRESH_MS = 3000;
/** A user with no activity for this long reads as away. */
export const AWAY_AFTER_MS = 5 * 60_000;

export interface PeerEditorPresence {
	/** Board entity the peer's text cursor sits on. */
	anchorId: string;
	anchorKind: FlowScriptAnchorWireKind;
	/** Node/layer ids the peer's text selection spans (empty = caret only). */
	selectedAnchorIds: string[];
}

/**
 * One awareness session of a peer, read through the wire sanitizers. Cursor
 * coordinates are deliberately NOT part of this shape (they live in the
 * high-frequency cursor store) so the board only re-renders on real changes.
 */
export interface PeerPresence {
	clientId: number;
	cursor?: { x: number; y: number };
	/** The sub (subject) from the auth token - use this to resolve user info via API */
	sub?: string;
	layerPath: string;
	selection: { nodes: string[] };
	/** The node the user just clicked/focused — cleared after a short timeout */
	activeNodeId?: string;
	/** Timestamp of the last active node click for freshness detection */
	activeNodeTs?: number;
	/** Present while the peer's FlowScript editor holds a focused cursor. */
	editor?: PeerEditorPresence;
	/** `"main"` or a module layer id while the peer has the code editor open. */
	codeFile?: string;
	/** Statements the peer's unapplied code buffer touches. */
	claimedAnchorIds: string[];
	/** Node ids of the peer's shared code scope, when they broadcast one. */
	scopeNodeIds: string[];
	/** Nodes the peer is currently executing. */
	executingNodeIds: string[];
	/** Kinds/count of the peer's last command batch (activity ticker). */
	lastEdit?: LastEditPayload;
	/** Outcome of the peer's last run. */
	lastRun?: LastRunPayload;
}

function stringList(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter((entry): entry is string => typeof entry === "string");
}

/**
 * Build the low-frequency presence record for one peer's awareness state.
 * `activeSeenAt` is the LOCAL clock time this client first saw the session's
 * current `activeNodeTs` (from {@link createPeerActivityTracker}); the peer's
 * own wall clock is never compared to ours. Absent → the click is not fresh.
 */
export function readPeerPresence(
	state: Record<string, unknown> | undefined,
	clientId: number,
	now: number,
	activeSeenAt?: number,
): PeerPresence {
	const activeNodeTs =
		typeof state?.activeNodeTs === "number" ? state.activeNodeTs : undefined;
	const activeNodeFresh =
		activeNodeTs !== undefined &&
		activeSeenAt !== undefined &&
		now - activeSeenAt < ACTIVE_NODE_FRESH_MS;
	const cursor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
	const claims = sanitizeClaimsForWire(state?.[FLOWSCRIPT_CLAIMS_FIELD]);
	const scope = sanitizeScopeForWire(state?.[FLOWSCRIPT_SCOPE_FIELD]);
	const view = sanitizeViewForWire(state?.[FLOWSCRIPT_VIEW_FIELD]);
	const execution = state?.executionPresence as
		| { executingNodes?: unknown }
		| undefined;
	return {
		clientId,
		sub: typeof state?.sub === "string" ? state.sub : undefined,
		layerPath:
			typeof state?.layerPath === "string" && state.layerPath
				? state.layerPath
				: "root",
		selection: {
			// sorted so presence equality is order-independent (getStates() order
			// and node-id order are not guaranteed stable)
			nodes: stringList(
				(state?.selection as { nodes?: unknown } | undefined)?.nodes,
			).sort(),
		},
		activeNodeId:
			activeNodeFresh && typeof state?.activeNodeId === "string"
				? state.activeNodeId
				: undefined,
		activeNodeTs: activeNodeFresh ? activeNodeTs : undefined,
		editor: cursor
			? {
					anchorId: cursor.anchor.id,
					anchorKind: cursor.anchor.kind,
					selectedAnchorIds: cursor.sel?.anchorIds ?? [],
				}
			: undefined,
		codeFile: view?.file,
		claimedAnchorIds: claims?.anchorIds ?? [],
		scopeNodeIds: scope?.nodeIds ?? [],
		executingNodeIds: stringList(execution?.executingNodes).sort(),
		lastEdit: sanitizeLastEdit(state?.[LAST_EDIT_FIELD]),
		lastRun: sanitizeLastRun(state?.[LAST_RUN_FIELD]),
	};
}

function sameList(a: readonly string[], b: readonly string[]): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
	return true;
}

/** Ignores cursor coordinates and the active-node timestamp: only what the UI renders. */
export function peerPresenceEqual(a: PeerPresence, b: PeerPresence): boolean {
	return (
		a.clientId === b.clientId &&
		a.sub === b.sub &&
		a.layerPath === b.layerPath &&
		a.activeNodeId === b.activeNodeId &&
		a.codeFile === b.codeFile &&
		a.editor?.anchorId === b.editor?.anchorId &&
		a.editor?.anchorKind === b.editor?.anchorKind &&
		sameList(
			a.editor?.selectedAnchorIds ?? [],
			b.editor?.selectedAnchorIds ?? [],
		) &&
		sameList(a.selection.nodes, b.selection.nodes) &&
		sameList(a.claimedAnchorIds, b.claimedAnchorIds) &&
		sameList(a.scopeNodeIds, b.scopeNodeIds) &&
		sameList(a.executingNodeIds, b.executingNodeIds) &&
		a.lastEdit?.ts === b.lastEdit?.ts &&
		a.lastRun?.ts === b.lastRun?.ts &&
		a.lastRun?.runId === b.lastRun?.runId
	);
}

export function peerPresenceListEqual(
	a: readonly PeerPresence[],
	b: readonly PeerPresence[],
): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		if (!peerPresenceEqual(a[i], b[i])) return false;
	}
	return true;
}

/**
 * A fingerprint of everything that counts as "the peer did something" —
 * pointer motion, selection, typing position, layer, file. Compared on the
 * LOCAL clock only; remote timestamps are never trusted for idleness.
 */
export function peerActivityKey(
	state: Record<string, unknown> | undefined,
): string {
	const cursor = state?.cursor as { x?: unknown; y?: unknown } | undefined;
	const editor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
	const viewport = state?.viewport as
		| { x?: unknown; y?: unknown; zoom?: unknown }
		| undefined;
	return [
		cursor ? `${String(cursor.x)}:${String(cursor.y)}` : "",
		stringList((state?.selection as { nodes?: unknown } | undefined)?.nodes)
			.sort()
			.join(","),
		typeof state?.activeNodeTs === "number" ? state.activeNodeTs : "",
		editor
			? `${editor.anchor.id}:${editor.dLine}:${editor.column}:${editor.ts}`
			: "",
		typeof state?.layerPath === "string" ? state.layerPath : "",
		viewport
			? `${String(viewport.x)}:${String(viewport.y)}:${String(viewport.zoom)}`
			: "",
	].join("|");
}

/**
 * Tracks when each session last changed, on the local clock. `observe` is
 * called with the full state map on every awareness change; `lastActiveAt`
 * answers per user (max over their sessions) for idle badges.
 */
/** Fingerprint of the editor-typing signal: the cursor payload including its ts. */
function editorTypingKey(state: Record<string, unknown> | undefined): string {
	const editor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
	return editor
		? `${editor.anchor.id}:${editor.dLine}:${editor.column}:${editor.ts}`
		: "";
}

export function createPeerActivityTracker(now: () => number = Date.now) {
	const sessions = new Map<number, { key: string; at: number; sub?: string }>();
	// Per session: the activeNodeTs value last seen and WHEN we first saw it.
	// Whatever is on the wire at the first observe is history, not a click.
	const clicks = new Map<number, { ts: number; seenAt: number }>();
	// Per session: when the editor cursor / chat typing heartbeat last changed.
	const typing = new Map<
		number,
		{ editorKey: string; editorAt: number; chatTs: number; chatAt: number }
	>();
	let seeded = false;
	const latestFor = (
		sub: string,
		pick: (entry: { editorAt: number; chatAt: number }) => number,
	): number | undefined => {
		let latest: number | undefined;
		for (const [clientId, entry] of typing) {
			if (sessions.get(clientId)?.sub !== sub) continue;
			const at = pick(entry);
			if (Number.isFinite(at) && (latest === undefined || at > latest))
				latest = at;
		}
		return latest;
	};
	return {
		observe(
			states: Map<number, Record<string, unknown>>,
			selfClientId: number,
		) {
			const at = now();
			for (const [clientId, state] of states) {
				if (clientId === selfClientId) continue;
				// A session seen for the first time (late join, reconnect) carries
				// history, not activity: only CHANGES on a known session are fresh.
				const wasKnown = sessions.has(clientId);
				const key = peerActivityKey(state);
				const sub = typeof state?.sub === "string" ? state.sub : undefined;
				const previous = sessions.get(clientId);
				if (!previous || previous.key !== key || previous.sub !== sub)
					sessions.set(clientId, { key, at, sub });
				const editorKey = editorTypingKey(state);
				const chatTs = sanitizeChatTyping(state?.[CHAT_TYPING_FIELD])?.ts ?? 0;
				const known = typing.get(clientId);
				const fresh = seeded && wasKnown;
				typing.set(clientId, {
					editorKey,
					editorAt:
						known && known.editorKey === editorKey
							? known.editorAt
							: fresh && editorKey
								? at
								: Number.NEGATIVE_INFINITY,
					chatTs,
					chatAt:
						known && known.chatTs === chatTs
							? known.chatAt
							: fresh && chatTs
								? at
								: Number.NEGATIVE_INFINITY,
				});
				const ts =
					typeof state?.activeNodeTs === "number"
						? state.activeNodeTs
						: undefined;
				if (ts === undefined) {
					clicks.delete(clientId);
				} else if (clicks.get(clientId)?.ts !== ts) {
					clicks.set(clientId, {
						ts,
						seenAt: fresh ? at : Number.NEGATIVE_INFINITY,
					});
				}
			}
			for (const clientId of sessions.keys()) {
				if (!states.has(clientId)) {
					sessions.delete(clientId);
					clicks.delete(clientId);
					typing.delete(clientId);
				}
			}
			seeded = true;
		},
		/** True while the user's editor cursor kept moving within TYPING_TTL_MS. */
		isTypingInEditor(sub: string): boolean {
			const at = latestFor(sub, (entry) => entry.editorAt);
			return at !== undefined && now() - at < TYPING_TTL_MS;
		},
		/** True while the user's chat typing heartbeat kept changing within TYPING_TTL_MS. */
		isTypingInChat(sub: string): boolean {
			const at = latestFor(sub, (entry) => entry.chatAt);
			return at !== undefined && now() - at < TYPING_TTL_MS;
		},
		/** True once none of the user's sessions did anything for AWAY_AFTER_MS. */
		isAway(sub: string): boolean {
			let latest: number | undefined;
			for (const session of sessions.values()) {
				if (session.sub !== sub) continue;
				if (latest === undefined || session.at > latest) latest = session.at;
			}
			return latest !== undefined && now() - latest >= AWAY_AFTER_MS;
		},
		lastActiveAt(sub: string): number | undefined {
			let latest: number | undefined;
			for (const session of sessions.values()) {
				if (session.sub !== sub) continue;
				if (latest === undefined || session.at > latest) latest = session.at;
			}
			return latest;
		},
		/** Local time this client first saw the session's current canvas click. */
		activeClickSeenAt(clientId: number): number | undefined {
			const seenAt = clicks.get(clientId)?.seenAt;
			return seenAt === undefined || !Number.isFinite(seenAt)
				? undefined
				: seenAt;
		},
		reset() {
			sessions.clear();
			clicks.clear();
			typing.clear();
			seeded = false;
		},
	};
}

export type PeerActivityTracker = ReturnType<typeof createPeerActivityTracker>;
