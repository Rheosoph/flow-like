import {
	AWAY_AFTER_MS,
	type PeerEditorPresence,
	type PeerPresence,
} from "../../lib/realtime/peer-presence";
import {
	type EditVerb,
	type LastEditPayload,
	type LastRunPayload,
	type RunStatus,
	editVerbs,
} from "../../lib/realtime/presence-signals";

/** A user is "idle" once none of their sessions did anything for this long. */
export const PRESENCE_IDLE_AFTER_MS = 60_000;
/** "Xm ago" labels stop counting here; older signals are simply stale. */
export const PRESENCE_AGO_CAP_MINUTES = 24 * 60;
/** How long a join/leave notice stays beside the count. */
export const PRESENCE_EVENT_TTL_MS = 4000;

/** Merged awareness sessions of one user, as the presence UI renders them. */
export interface PresenceCollaborator {
	sub: string;
	/** Another window of the local user. */
	self: boolean;
	sessions: number;
	/** Layer path of the most recently active session. */
	layerPath: string;
	/** Union of every session's canvas selection, sorted. */
	selectionNodeIds: string[];
	/** Editor cursor of the most recently active session that has one. */
	editor?: PeerEditorPresence;
	codeFile?: string;
	claimedAnchorIds: string[];
	scopeNodeIds: string[];
	executingNodeIds: string[];
	activeNodeId?: string;
	activeNodeTs?: number;
	/** Most recent command batch across the user's sessions. */
	lastEdit?: LastEditPayload;
	/** Most recent run outcome across the user's sessions. */
	lastRun?: LastRunPayload;
}

function union(...lists: readonly (readonly string[])[]): string[] {
	const out = new Set<string>();
	for (const list of lists) for (const id of list) out.add(id);
	return [...out].sort();
}

function normalizeLayerPath(layerPath: string | undefined): string {
	return layerPath || "root";
}

function latestOf<T extends { ts: number }>(
	current: T | undefined,
	candidate: T | undefined,
): T | undefined {
	if (!candidate) return current;
	if (!current) return candidate;
	return candidate.ts > current.ts ? candidate : current;
}

/**
 * One row per user: a user with several windows is still one collaborator.
 * Sessions without a `sub` cannot be attributed to anyone and are dropped.
 */
export function mergeCollaborators(
	peers: readonly PeerPresence[],
	ownSub: string | undefined,
): PresenceCollaborator[] {
	const byUser = new Map<
		string,
		PresenceCollaborator & { primaryTs: number; editorTs: number }
	>();
	for (const peer of peers) {
		if (!peer.sub) continue;
		const ts = peer.activeNodeTs ?? 0;
		const existing = byUser.get(peer.sub);
		if (!existing) {
			byUser.set(peer.sub, {
				sub: peer.sub,
				self: peer.sub === ownSub,
				sessions: 1,
				layerPath: normalizeLayerPath(peer.layerPath),
				selectionNodeIds: union(peer.selection.nodes),
				editor: peer.editor,
				codeFile: peer.codeFile,
				claimedAnchorIds: union(peer.claimedAnchorIds),
				scopeNodeIds: union(peer.scopeNodeIds),
				executingNodeIds: union(peer.executingNodeIds),
				activeNodeId: peer.activeNodeId,
				activeNodeTs: peer.activeNodeTs,
				lastEdit: peer.lastEdit,
				lastRun: peer.lastRun,
				primaryTs: ts,
				editorTs: peer.editor ? ts : -1,
			});
			continue;
		}
		existing.sessions += 1;
		existing.selectionNodeIds = union(
			existing.selectionNodeIds,
			peer.selection.nodes,
		);
		existing.claimedAnchorIds = union(
			existing.claimedAnchorIds,
			peer.claimedAnchorIds,
		);
		existing.scopeNodeIds = union(existing.scopeNodeIds, peer.scopeNodeIds);
		existing.executingNodeIds = union(
			existing.executingNodeIds,
			peer.executingNodeIds,
		);
		existing.lastEdit = latestOf(existing.lastEdit, peer.lastEdit);
		existing.lastRun = latestOf(existing.lastRun, peer.lastRun);
		const becamePrimary = ts > existing.primaryTs;
		if (becamePrimary) {
			existing.primaryTs = ts;
			existing.layerPath = normalizeLayerPath(peer.layerPath);
			existing.activeNodeId = peer.activeNodeId;
			existing.activeNodeTs = peer.activeNodeTs;
		}
		if (peer.editor && ts > existing.editorTs) {
			existing.editorTs = ts;
			existing.editor = peer.editor;
		}
		if (peer.codeFile && (!existing.codeFile || becamePrimary)) {
			existing.codeFile = peer.codeFile;
		}
	}
	return [...byUser.values()].map(
		({ primaryTs: _primaryTs, editorTs: _editorTs, ...collab }) => collab,
	);
}

/**
 * Real peers before the local user's other windows; within each group the
 * users on the current layer first, then by display name.
 */
export function sortCollaborators(
	collaborators: readonly PresenceCollaborator[],
	currentLayerPath: string,
	nameOf: (sub: string) => string,
): PresenceCollaborator[] {
	const here = normalizeLayerPath(currentLayerPath);
	return [...collaborators].sort((a, b) => {
		if (a.self !== b.self) return a.self ? 1 : -1;
		const aHere = a.layerPath === here;
		const bHere = b.layerPath === here;
		if (aHere !== bHere) return aHere ? -1 : 1;
		const byName = nameOf(a.sub).localeCompare(nameOf(b.sub));
		return byName !== 0 ? byName : a.sub.localeCompare(b.sub);
	});
}

export interface PresenceActivityContext {
	currentLayerPath: string;
	layerNames?: ReadonlyMap<string, string>;
	/** `main` and module layer ids → file labels. */
	fileLabels?: ReadonlyMap<string, string>;
	nodeName?: (nodeId: string) => string | undefined;
	/** Local-clock time of the user's last activity, if known. */
	lastActiveAt?: number;
	/** Live predicates, evaluated on the local clock by the caller. */
	typingInEditor?: boolean;
	typingInChat?: boolean;
	away?: boolean;
	now: number;
}

export interface PresenceLastEdit {
	verbs: EditVerb[];
	count: number;
	agoMinutes: number;
}

export interface PresenceLastRun {
	status: RunStatus;
	executed: number;
	agoMinutes: number;
}

export interface PresenceActivity {
	sameLayer: boolean;
	layerPath: string;
	/** `undefined` on the root layer. */
	layerLabel?: string;
	codeFileLabel?: string;
	/** What their text cursor sits on; variables are not shown. */
	editing?: { anchorId: string; kind: "node" | "layer"; label: string };
	selectedCount: number;
	firstSelectedNodeId?: string;
	running: boolean;
	typingInEditor: boolean;
	typingInChat: boolean;
	/** No activity for {@link AWAY_AFTER_MS}; `idleMinutes` is then always set. */
	away: boolean;
	/** Whole minutes since the last activity, once past the idle threshold. */
	idleMinutes?: number;
	lastEdit?: PresenceLastEdit;
	lastRun?: PresenceLastRun;
}

/**
 * Whole minutes between a peer-clock timestamp and the local clock, for an
 * "Xm ago" label only: clamped so skewed clocks never read negative or absurd.
 */
export function agoMinutes(ts: number, now: number): number {
	const minutes = Math.floor((now - ts) / 60_000);
	return Math.min(Math.max(minutes, 0), PRESENCE_AGO_CAP_MINUTES);
}

/** Untranslated summary of where a collaborator is and what they are doing. */
export function describeActivity(
	collab: PresenceCollaborator,
	ctx: PresenceActivityContext,
): PresenceActivity {
	const layerPath = normalizeLayerPath(collab.layerPath);
	const layerId =
		layerPath === "root" ? undefined : layerPath.split("/").at(-1);
	const layerLabel = layerId
		? (ctx.layerNames?.get(layerId) ?? layerId.slice(0, 10))
		: undefined;

	let editing: PresenceActivity["editing"];
	const editor = collab.editor;
	if (editor && editor.anchorKind !== "variable") {
		const label =
			editor.anchorKind === "layer"
				? ctx.layerNames?.get(editor.anchorId)
				: ctx.nodeName?.(editor.anchorId);
		editing = {
			anchorId: editor.anchorId,
			kind: editor.anchorKind,
			label: label ?? editor.anchorId.slice(0, 10),
		};
	}

	let idleMinutes: number | undefined;
	if (ctx.lastActiveAt !== undefined) {
		const idleMs = ctx.now - ctx.lastActiveAt;
		if (idleMs >= PRESENCE_IDLE_AFTER_MS) {
			idleMinutes = Math.max(1, Math.floor(idleMs / 60_000));
		}
	}
	const away = ctx.away ?? false;
	if (away && idleMinutes === undefined) {
		idleMinutes = Math.max(1, Math.floor(AWAY_AFTER_MS / 60_000));
	}

	let lastEdit: PresenceLastEdit | undefined;
	if (collab.lastEdit) {
		const verbs = editVerbs(collab.lastEdit.kinds);
		if (verbs.length > 0) {
			lastEdit = {
				verbs,
				count: collab.lastEdit.count,
				agoMinutes: agoMinutes(collab.lastEdit.ts, ctx.now),
			};
		}
	}

	const lastRun: PresenceLastRun | undefined = collab.lastRun
		? {
				status: collab.lastRun.status,
				executed: collab.lastRun.executed,
				agoMinutes: agoMinutes(collab.lastRun.ts, ctx.now),
			}
		: undefined;

	return {
		sameLayer: layerPath === normalizeLayerPath(ctx.currentLayerPath),
		layerPath,
		layerLabel,
		codeFileLabel: collab.codeFile
			? (ctx.fileLabels?.get(collab.codeFile) ?? collab.codeFile)
			: undefined,
		editing,
		selectedCount: collab.selectionNodeIds.length,
		firstSelectedNodeId: collab.selectionNodeIds[0],
		running: collab.executingNodeIds.length > 0,
		typingInEditor: ctx.typingInEditor ?? false,
		typingInChat: ctx.typingInChat ?? false,
		away,
		idleMinutes,
		lastEdit,
		lastRun,
	};
}

/** Canvas nodes to light up while a collaborator's row is hovered. */
export function presenceHighlightIds(collab: PresenceCollaborator): string[] {
	const editorIds = collab.editor
		? [
				...collab.editor.selectedAnchorIds,
				...(collab.editor.anchorKind === "node"
					? [collab.editor.anchorId]
					: []),
			]
		: [];
	return union(collab.selectionNodeIds, editorIds);
}

export interface PresenceStats {
	/** Distinct people, the local user included. */
	onlineCount: number;
	/** Users with the code editor open or a text cursor in it. */
	inCodeEditor: number;
}

export function presenceStats(
	collaborators: readonly PresenceCollaborator[],
): PresenceStats {
	let peers = 0;
	let inCodeEditor = 0;
	for (const collab of collaborators) {
		if (!collab.self) peers += 1;
		if (collab.codeFile !== undefined || collab.editor !== undefined)
			inCodeEditor += 1;
	}
	return { onlineCount: peers + 1, inCodeEditor };
}

export interface PresenceEvent {
	sub: string;
	kind: "joined" | "left";
	/** Local-clock time the event was observed. */
	at: number;
}

/** Milliseconds a join/leave notice has left on screen; 0 once it expired. */
export function presenceEventRemainingMs(
	event: PresenceEvent | undefined,
	now: number,
): number {
	if (!event) return 0;
	return Math.min(
		Math.max(event.at + PRESENCE_EVENT_TTL_MS - now, 0),
		PRESENCE_EVENT_TTL_MS,
	);
}
