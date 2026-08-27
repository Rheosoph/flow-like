/**
 * Transient canvas-level presence signals over the board's Yjs awareness.
 *
 * Same contract as the FlowScript presence protocol (collab rule 2): every
 * payload is ids, bounded numbers, closed enums and timestamps — never board
 * content, node names or free text. Every field is sanitized on BOTH sides
 * (before publishing, and when reading a peer's state).
 */

import { ICommandType } from "../schema/flow/board/commands/generic-command";

/** Nodes the local user is dragging right now, with their live positions. */
export const DRAG_FIELD = "drag";
/** A transient "look here" marker (optionally an emoji reaction). */
export const PING_FIELD = "ping";
/** "Bring everyone here": a viewport + layer for peers to jump to. */
export const SUMMON_FIELD = "summon";
/** What the local user's last board command batch did — kinds and counts only. */
export const LAST_EDIT_FIELD = "lastEdit";
/** Outcome of the local user's last board run. */
export const LAST_RUN_FIELD = "lastRun";
/** Heartbeat while the local user types in the board chat. */
export const CHAT_TYPING_FIELD = "chatTyping";

/** Board entity ids (cuid2-style); the same shape the FlowScript protocol accepts. */
const ID_PATTERN = /^[A-Za-z0-9_-]{10,32}$/;
/** Canvas coordinates are unbounded in principle; this bound is far beyond any real board. */
const MAX_COORDINATE = 1e7;
export const MAX_DRAG_NODES = 32;
export const MAX_EDIT_KINDS = 8;
/** Reactions are a closed set so the wire never carries arbitrary text. */
export const PING_EMOJI = [
	"👍",
	"👀",
	"🎉",
	"❤️",
	"🔥",
	"🤔",
	"👋",
	"✅",
] as const;
export type PingEmoji = (typeof PING_EMOJI)[number];
export const RUN_STATUSES = ["ok", "error"] as const;
export type RunStatus = (typeof RUN_STATUSES)[number];
/** How long a ping stays on screen, on the receiver's clock from first sight. */
export const PING_TTL_MS = 2500;
/** A summon older than this on arrival (peer clock) is still shown — freshness is first-sight local. */
export const SUMMON_TTL_MS = 15_000;
/** A typing heartbeat counts as "typing" this long after it was last seen changing. */
export const TYPING_TTL_MS = 3000;

export interface DragPayload {
	nodes: { id: string; x: number; y: number }[];
	ts: number;
}

export interface PingPayload {
	x: number;
	y: number;
	/** Layer path the ping was made on; peers elsewhere only get the edge hint. */
	layerPath: string;
	emoji?: PingEmoji;
	/** Monotonic per session so an identical spot pinged twice is two pings. */
	seq: number;
	ts: number;
}

export interface SummonPayload {
	x: number;
	y: number;
	zoom: number;
	layerPath: string;
	seq: number;
	ts: number;
}

export interface LastEditPayload {
	/** Distinct command kinds in the batch (closed enum, ≤ MAX_EDIT_KINDS). */
	kinds: ICommandType[];
	/** Commands in the batch. */
	count: number;
	ts: number;
}

export interface LastRunPayload {
	runId: string;
	status: RunStatus;
	/** Node executions completed. */
	executed: number;
	ts: number;
}

export interface ChatTypingPayload {
	ts: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function coordinate(value: unknown): number | undefined {
	if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
	return Math.max(-MAX_COORDINATE, Math.min(MAX_COORDINATE, value));
}

function nonNegativeInt(value: unknown, max: number): number | undefined {
	if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
	return Math.min(Math.max(Math.round(value), 0), max);
}

function wireTs(value: unknown): number {
	return nonNegativeInt(value, Number.MAX_SAFE_INTEGER) ?? 0;
}

function wireId(value: unknown): string | undefined {
	return typeof value === "string" && ID_PATTERN.test(value)
		? value
		: undefined;
}

/** `root` or `/`-joined layer ids. Anything else is not a layer path. */
export function wireLayerPath(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	if (value === "" || value === "root") return "root";
	const segments = value.split("/");
	if (segments.length > 16) return undefined;
	return segments.every((segment) => ID_PATTERN.test(segment))
		? value
		: undefined;
}

export function sanitizeDrag(value: unknown): DragPayload | undefined {
	if (!isRecord(value) || !Array.isArray(value.nodes)) return undefined;
	const nodes: DragPayload["nodes"] = [];
	const seen = new Set<string>();
	for (const entry of value.nodes) {
		if (!isRecord(entry)) continue;
		const id = wireId(entry.id);
		const x = coordinate(entry.x);
		const y = coordinate(entry.y);
		if (!id || x === undefined || y === undefined || seen.has(id)) continue;
		seen.add(id);
		nodes.push({ id, x, y });
		if (nodes.length >= MAX_DRAG_NODES) break;
	}
	if (nodes.length === 0) return undefined;
	return { nodes, ts: wireTs(value.ts) };
}

export function sanitizePing(value: unknown): PingPayload | undefined {
	if (!isRecord(value)) return undefined;
	const x = coordinate(value.x);
	const y = coordinate(value.y);
	const layerPath = wireLayerPath(value.layerPath);
	const seq = nonNegativeInt(value.seq, Number.MAX_SAFE_INTEGER);
	if (x === undefined || y === undefined || !layerPath || seq === undefined)
		return undefined;
	const payload: PingPayload = { x, y, layerPath, seq, ts: wireTs(value.ts) };
	if (PING_EMOJI.includes(value.emoji as PingEmoji))
		payload.emoji = value.emoji as PingEmoji;
	return payload;
}

export function sanitizeSummon(value: unknown): SummonPayload | undefined {
	if (!isRecord(value)) return undefined;
	const x = coordinate(value.x);
	const y = coordinate(value.y);
	const layerPath = wireLayerPath(value.layerPath);
	const seq = nonNegativeInt(value.seq, Number.MAX_SAFE_INTEGER);
	const zoom =
		typeof value.zoom === "number" && Number.isFinite(value.zoom)
			? Math.min(Math.max(value.zoom, 0.05), 10)
			: undefined;
	if (
		x === undefined ||
		y === undefined ||
		!layerPath ||
		seq === undefined ||
		zoom === undefined
	)
		return undefined;
	return { x, y, zoom, layerPath, seq, ts: wireTs(value.ts) };
}

const COMMAND_KINDS = new Set<string>(Object.values(ICommandType));

export function sanitizeLastEdit(value: unknown): LastEditPayload | undefined {
	if (!isRecord(value) || !Array.isArray(value.kinds)) return undefined;
	const kinds: ICommandType[] = [];
	for (const kind of value.kinds) {
		if (typeof kind !== "string" || !COMMAND_KINDS.has(kind)) continue;
		if (kinds.includes(kind as ICommandType)) continue;
		kinds.push(kind as ICommandType);
		if (kinds.length >= MAX_EDIT_KINDS) break;
	}
	const count = nonNegativeInt(value.count, 100_000);
	if (kinds.length === 0 || !count) return undefined;
	return { kinds, count, ts: wireTs(value.ts) };
}

export function sanitizeLastRun(value: unknown): LastRunPayload | undefined {
	if (!isRecord(value)) return undefined;
	const runId = wireId(value.runId);
	const executed = nonNegativeInt(value.executed, 10_000_000);
	if (!runId || executed === undefined) return undefined;
	if (!RUN_STATUSES.includes(value.status as RunStatus)) return undefined;
	return {
		runId,
		status: value.status as RunStatus,
		executed,
		ts: wireTs(value.ts),
	};
}

export function sanitizeChatTyping(
	value: unknown,
): ChatTypingPayload | undefined {
	if (!isRecord(value)) return undefined;
	const ts = nonNegativeInt(value.ts, Number.MAX_SAFE_INTEGER);
	return ts ? { ts } : undefined;
}

/** Human-readable grouping of a command batch for the activity ticker. */
export type EditVerb =
	| "added"
	| "moved"
	| "connected"
	| "disconnected"
	| "removed"
	| "updated"
	| "commented"
	| "layered"
	| "variables";

const KIND_VERB: Record<ICommandType, EditVerb> = {
	[ICommandType.AddNode]: "added",
	[ICommandType.CopyPaste]: "added",
	[ICommandType.MoveNode]: "moved",
	[ICommandType.MoveToLayer]: "moved",
	[ICommandType.ConnectPin]: "connected",
	[ICommandType.DisconnectPin]: "disconnected",
	[ICommandType.RemoveNode]: "removed",
	[ICommandType.RemoveLayer]: "removed",
	[ICommandType.RemoveComment]: "removed",
	[ICommandType.RemoveVariable]: "removed",
	[ICommandType.UpdateNode]: "updated",
	[ICommandType.UpsertPin]: "updated",
	[ICommandType.UpsertComment]: "commented",
	[ICommandType.UpsertLayer]: "layered",
	[ICommandType.UpsertVariable]: "variables",
};

/** Distinct verbs of a batch, in the order they first appear. */
export function editVerbs(kinds: readonly ICommandType[]): EditVerb[] {
	const verbs: EditVerb[] = [];
	for (const kind of kinds) {
		const verb = KIND_VERB[kind];
		if (verb && !verbs.includes(verb)) verbs.push(verb);
	}
	return verbs;
}
