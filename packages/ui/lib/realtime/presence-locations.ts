import type { PeerPresence } from "./peer-presence";

/** One user at one place, however many windows they have open there. */
export interface PresenceMark {
	sub: string;
	/** Another window of the local user. */
	self: boolean;
	sessions: number;
}

export interface NodeWatchers {
	/** Users whose canvas selection contains the node. */
	selected: PresenceMark[];
	/** Users whose code editor cursor, selection or unapplied buffer touches the node. */
	editing: PresenceMark[];
}

type MarkIndex = Map<string, Map<string, PresenceMark>>;

function addMark(
	index: MarkIndex,
	key: string,
	sub: string,
	ownSub: string | undefined,
): void {
	let bucket = index.get(key);
	if (!bucket) {
		bucket = new Map();
		index.set(key, bucket);
	}
	const existing = bucket.get(sub);
	if (existing) existing.sessions += 1;
	else bucket.set(sub, { sub, self: sub === ownSub, sessions: 1 });
}

/** Real peers first, the local user's own windows last, stable by sub within each. */
export function sortPresenceMarks(
	marks: Iterable<PresenceMark>,
): PresenceMark[] {
	return [...marks].sort(
		(a, b) => Number(a.self) - Number(b.self) || a.sub.localeCompare(b.sub),
	);
}

function finish(index: MarkIndex): Map<string, PresenceMark[]> {
	const out = new Map<string, PresenceMark[]>();
	for (const [key, bucket] of index) {
		out.set(key, sortPresenceMarks(bucket.values()));
	}
	return out;
}

function indexBy(
	peers: readonly PeerPresence[],
	ownSub: string | undefined,
	keyOf: (peer: PeerPresence) => string | undefined,
): Map<string, PresenceMark[]> {
	const index: MarkIndex = new Map();
	for (const peer of peers) {
		if (!peer.sub) continue;
		const key = keyOf(peer);
		if (!key) continue;
		addMark(index, key, peer.sub, ownSub);
	}
	return finish(index);
}

/**
 * Who has which file open in the code editor, keyed by file id (`main` or a
 * module layer id). Sessions without a file or without a sub are dropped.
 */
export function presenceByFile(
	peers: readonly PeerPresence[],
	ownSub: string | undefined,
): Map<string, PresenceMark[]> {
	return indexBy(peers, ownSub, (peer) => peer.codeFile);
}

/** The layer id a wire path points at; `undefined` for the root. */
export function layerIdOfPath(
	layerPath: string | undefined,
): string | undefined {
	if (!layerPath || layerPath === "root") return undefined;
	const segments = layerPath.split("/");
	const last = segments[segments.length - 1];
	return last && last !== "root" ? last : undefined;
}

/**
 * Who has which layer open on the canvas, keyed by the innermost layer id of
 * their path. The root is not a layer and is never a key.
 */
export function presenceByLayer(
	peers: readonly PeerPresence[],
	ownSub: string | undefined,
): Map<string, PresenceMark[]> {
	return indexBy(peers, ownSub, (peer) => layerIdOfPath(peer.layerPath));
}

/**
 * One list out of several for the same place, deduped by user. A session that
 * shows up in more than one input (a module is both a file and a layer) is not
 * counted twice, so the session count is the largest any input reported.
 */
export function mergePresenceMarks(
	...lists: readonly (readonly PresenceMark[] | undefined)[]
): PresenceMark[] {
	const bySub = new Map<string, PresenceMark>();
	for (const list of lists) {
		for (const mark of list ?? []) {
			const existing = bySub.get(mark.sub);
			if (!existing) bySub.set(mark.sub, { ...mark });
			else existing.sessions = Math.max(existing.sessions, mark.sessions);
		}
	}
	return sortPresenceMarks(bySub.values());
}

function editsNode(peer: PeerPresence, nodeId: string): boolean {
	return (
		peer.editor?.anchorId === nodeId ||
		peer.editor?.selectedAnchorIds.includes(nodeId) === true ||
		peer.claimedAnchorIds.includes(nodeId)
	);
}

/** Who has `nodeId` selected on the canvas and who is touching it in code. */
export function nodeWatchers(
	peers: readonly PeerPresence[],
	nodeId: string,
	ownSub: string | undefined,
): NodeWatchers {
	const selected: MarkIndex = new Map();
	const editing: MarkIndex = new Map();
	for (const peer of peers) {
		if (!peer.sub) continue;
		if (peer.selection.nodes.includes(nodeId)) {
			addMark(selected, nodeId, peer.sub, ownSub);
		}
		if (editsNode(peer, nodeId)) {
			addMark(editing, nodeId, peer.sub, ownSub);
		}
	}
	return {
		selected: finish(selected).get(nodeId) ?? [],
		editing: finish(editing).get(nodeId) ?? [],
	};
}

/** A stable fingerprint of a mark list, for memo keys and change detection. */
export function presenceMarksKey(marks: readonly PresenceMark[]): string {
	return marks
		.map((mark) => `${mark.sub}:${mark.self ? 1 : 0}:${mark.sessions}`)
		.join("|");
}
