import { type Query, focusManager, onlineManager } from "@tanstack/react-query";
import {
	type RealtimeDelivery,
	realtimeDelivery,
	useRealtimeStatus,
} from "./realtime/realtime-status";

/** The longest a poll that keeps answering the same waits while realtime carries changes. */
export const ADAPTIVE_POLL_MAX_MS = 5 * 60_000;

export interface AdaptivePollOptions {
	/** The interval while realtime carries nothing, and the one a backoff starts over from. */
	readonly baseMs: number;
	/** Where a backoff starts while realtime carries changes; `baseMs` unless set. */
	readonly liveBaseMs?: number;
	readonly maxMs?: number;
	/**
	 * Back off only over a live push connection. Without it a poll also backs off while realtime
	 * polls for this account, whose notification replay then announces changes.
	 */
	readonly requireLive?: boolean;
}

/** `baseMs` for the first two quiet answers, then doubling up to `maxMs`. */
export function backedOffMs(
	baseMs: number,
	quietAnswers: number,
	maxMs = ADAPTIVE_POLL_MAX_MS,
): number {
	return Math.min(maxMs, baseMs * 2 ** Math.max(0, quietAnswers - 1));
}

let generation = 0;

/** Starts every adaptive poll over at its base interval: something says data may have moved. */
export function resetAdaptivePolling(): void {
	generation++;
}

let watching = false;

/** Subscribed on first use, so importing this module has no side effects. */
function watchForResets(): void {
	if (watching) return;
	watching = true;
	focusManager.subscribe((focused) => {
		if (focused) resetAdaptivePolling();
	});
	onlineManager.subscribe((online) => {
		if (online) resetAdaptivePolling();
	});
	useRealtimeStatus.subscribe((next, previous) => {
		if (realtimeDelivery(next.status) !== realtimeDelivery(previous.status))
			resetAdaptivePolling();
	});
}

interface Streak {
	updates: number;
	data: unknown;
	quiet: number;
	generation: number;
}

// biome-ignore lint/suspicious/noExplicitAny: any query can poll adaptively.
type AnyQuery = Query<any, any, any, any>;

const streaks = new WeakMap<AnyQuery, Streak>();

/**
 * How many answers in a row brought nothing new. Structural sharing keeps the previous data
 * reference when an answer equals it, so an unchanged reference after an update is quiet.
 */
function quietAnswers(query: AnyQuery): number {
	const { data, dataUpdateCount } = query.state;
	const streak = streaks.get(query);
	if (!streak || streak.generation !== generation) {
		streaks.set(query, {
			updates: dataUpdateCount,
			data,
			quiet: 0,
			generation,
		});
		return 0;
	}
	if (streak.updates !== dataUpdateCount) {
		streak.quiet = data === streak.data ? streak.quiet + 1 : 0;
		streak.updates = dataUpdateCount;
		streak.data = data;
	}
	return streak.quiet;
}

function carriesChanges(
	delivery: RealtimeDelivery,
	requireLive: boolean,
): boolean {
	return delivery === "live" || (!requireLive && delivery === "polling");
}

/** The next interval of `query`; exported for polls that compose it with their own condition. */
export function adaptiveInterval(
	query: AnyQuery,
	options: AdaptivePollOptions,
): number {
	watchForResets();
	const quiet = quietAnswers(query);
	const delivery = realtimeDelivery(useRealtimeStatus.getState().status);
	if (!carriesChanges(delivery, options.requireLive === true))
		return options.baseMs;
	return backedOffMs(
		options.liveBaseMs ?? options.baseMs,
		quiet,
		options.maxMs,
	);
}

/**
 * A `refetchInterval` that backs off while answers stay the same and realtime carries changes.
 * Window focus, reconnecting, a change in realtime delivery and {@link resetAdaptivePolling}
 * start it over; without realtime it keeps `baseMs`.
 */
export function adaptiveRefetchInterval(options: AdaptivePollOptions) {
	return (query: AnyQuery): number => adaptiveInterval(query, options);
}
