import type { IBotTurn } from "../../state/backend-state/bot-state";
import type {
	ConversationTurnStatus,
	IConversationTurn,
} from "../../state/backend-state/inbox-state";
import { CHAT_TYPING_FIELD, TYPING_TTL_MS } from "./presence-signals";

export interface ConversationBotActivity {
	botId: string;
	status: ConversationTurnStatus;
	since: number;
}

const STATUS_RANK: Record<ConversationTurnStatus, number> = {
	queued: 0,
	waiting: 1,
	running: 2,
};

const MAX_SUB_LENGTH = 128;

function isActiveStatus(status: unknown): status is ConversationTurnStatus {
	return typeof status === "string" && status in STATUS_RANK;
}

function timeMs(value: string | null | undefined): number | undefined {
	if (!value) return undefined;
	const parsed = Date.parse(value);
	return Number.isFinite(parsed) ? parsed : undefined;
}

/**
 * Merges API turns with live host turns by turn id (the live status wins, so a
 * finished live turn drops the API's stale entry), keeps unfinished turns of
 * bots in `botIds`, and collapses them to the most advanced status per bot.
 */
export function mergeBotActivity(
	apiTurns: readonly IConversationTurn[],
	liveTurns: readonly IBotTurn[],
	botIds: ReadonlySet<string>,
	now: number,
): ConversationBotActivity[] {
	const byId = new Map<
		string,
		{ botId: string; status: unknown; since: number }
	>();
	for (const turn of apiTurns) {
		byId.set(turn.id, {
			botId: turn.bot_id,
			status: turn.status,
			since: timeMs(turn.created_at) ?? now,
		});
	}
	for (const turn of liveTurns) {
		const known = byId.get(turn.id);
		byId.set(turn.id, {
			botId: turn.bot_id,
			status: turn.status,
			since: timeMs(turn.started_at) ?? known?.since ?? now,
		});
	}

	const perBot = new Map<string, ConversationBotActivity>();
	for (const turn of byId.values()) {
		if (!isActiveStatus(turn.status) || !botIds.has(turn.botId)) continue;
		const best = perBot.get(turn.botId);
		if (best && STATUS_RANK[best.status] >= STATUS_RANK[turn.status]) continue;
		perBot.set(turn.botId, {
			botId: turn.botId,
			status: turn.status,
			since: turn.since,
		});
	}
	return [...perBot.values()].sort((a, b) => a.since - b.since);
}

export function hasActiveTurn(turns: readonly IConversationTurn[]): boolean {
	return turns.some((turn) => isActiveStatus(turn.status));
}

interface TypingEntry {
	sub: string;
	ts: number;
	seenAt: number;
	startedAt: number;
}

export interface TypingSnapshot {
	typing: string[];
	/** Local time at which the next typer stops counting; null when nobody types. */
	nextExpiryAt: number | null;
}

function readTypingState(
	state: unknown,
): { sub: string; ts: number } | undefined {
	if (typeof state !== "object" || state === null) return undefined;
	const record = state as Record<string, unknown>;
	const sub = record.sub;
	if (
		typeof sub !== "string" ||
		sub.length === 0 ||
		sub.length > MAX_SUB_LENGTH
	)
		return undefined;
	const typing = record[CHAT_TYPING_FIELD];
	if (typeof typing !== "object" || typing === null) return undefined;
	const ts = (typing as Record<string, unknown>).ts;
	if (typeof ts !== "number" || !Number.isFinite(ts) || ts <= 0)
		return undefined;
	return { sub, ts };
}

/**
 * Derives who is typing from raw awareness states. A peer types while its
 * heartbeat keeps changing within TYPING_TTL_MS on the local clock; peers are
 * untrusted, so only subs in `allowed` count.
 */
export function createTypingTracker() {
	const entries = new Map<number, TypingEntry>();

	return {
		observe(
			states: ReadonlyMap<number, unknown>,
			selfClientId: number,
			allowed: ReadonlySet<string>,
			now: number,
		): TypingSnapshot {
			for (const clientId of [...entries.keys()]) {
				if (!states.has(clientId)) entries.delete(clientId);
			}
			const active = new Map<string, number>();
			let nextExpiryAt: number | null = null;
			states.forEach((state, clientId) => {
				if (clientId === selfClientId) return;
				const read = readTypingState(state);
				if (!read || !allowed.has(read.sub)) {
					entries.delete(clientId);
					return;
				}
				const known = entries.get(clientId);
				let entry: TypingEntry;
				if (!known || known.sub !== read.sub) {
					entry = { ...read, seenAt: now, startedAt: now };
				} else if (known.ts !== read.ts) {
					const wasTyping = now - known.seenAt < TYPING_TTL_MS;
					entry = {
						...read,
						seenAt: now,
						startedAt: wasTyping ? known.startedAt : now,
					};
				} else {
					entry = known;
				}
				entries.set(clientId, entry);
				const expiresAt = entry.seenAt + TYPING_TTL_MS;
				if (expiresAt <= now) return;
				const earliest = active.get(entry.sub);
				if (earliest === undefined || entry.startedAt < earliest) {
					active.set(entry.sub, entry.startedAt);
				}
				nextExpiryAt =
					nextExpiryAt === null ? expiresAt : Math.min(nextExpiryAt, expiresAt);
			});
			const typing = [...active.entries()]
				.sort((a, b) => a[1] - b[1] || a[0].localeCompare(b[0]))
				.map(([sub]) => sub);
			return { typing, nextExpiryAt };
		},
		reset() {
			entries.clear();
		},
	};
}
