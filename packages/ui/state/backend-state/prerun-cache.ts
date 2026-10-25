// Stale-while-revalidate cache for prerun analysis responses.
//
// The server-side prerun cache (in flow-like-api) already collapses repeat
// requests on its end, but a client-side cache eliminates the round-trip
// entirely on rapid re-checks (think: user clicks Run, then Run again, or
// switches between two workflow tabs). Each cached entry carries the
// server's signature. When a background revalidation returns a different
// signature, listeners are notified so the UI can prompt the user.

import { isRecord } from "../../lib/response-shape";

interface PrerunLike {
	signature?: string;
}

interface CacheEntry<T extends PrerunLike> {
	data: T;
	fetchedAt: number;
}

interface PrerunSwrOptions<T extends PrerunLike> {
	/**
	 * Fired when a background revalidation returns a payload whose signature
	 * differs from the cached entry. Both signatures are passed so the UI
	 * can decide whether to surface the drift (e.g. via a toast).
	 */
	onDrift?: (key: string, previous: T, fresh: T) => void;
}

const cache = new Map<string, CacheEntry<PrerunLike>>();
const inflight = new Map<string, Promise<PrerunLike>>();

// Entries younger than 15 seconds skip revalidation. Older entries remain
// available while one request refreshes them.
const REVALIDATE_AFTER_MS = 15_000;

export function prerunBoardKey(
	appId: string,
	boardId: string,
	version?: [number, number, number],
): string {
	const v = version ? `${version[0]}_${version[1]}_${version[2]}` : "latest";
	return `board:${appId}:${boardId}:${v}`;
}

export function prerunEventKey(
	appId: string,
	eventId: string,
	version?: [number, number, number],
): string {
	const v = version ? `${version[0]}_${version[1]}_${version[2]}` : "latest";
	return `event:${appId}:${eventId}:${v}`;
}

/**
 * Run `fetcher` through a stale-while-revalidate cache. On cache hit, the
 * cached value is returned without waiting for a fetch. If it's older than
 * the revalidate threshold, a background fetch starts and `onDrift` fires
 * when the resulting signature differs from the cached one.
 */
export async function prerunSwr<T extends PrerunLike>(
	key: string,
	fetcher: () => Promise<T>,
	opts?: PrerunSwrOptions<T>,
): Promise<T> {
	const now = Date.now();
	const entry = cache.get(key) as CacheEntry<T> | undefined;
	const pending = inflight.get(key) as Promise<T> | undefined;

	if (entry && (now - entry.fetchedAt <= REVALIDATE_AFTER_MS || pending)) {
		return entry.data;
	}
	if (pending) return pending;

	// Register the request before invoking the fetcher, including synchronous failures.
	const request: Promise<T> = Promise.resolve()
		.then(fetcher)
		.then((fresh) => {
			if (!isRecord(fresh)) {
				if (entry) {
					throw new Error(`Prerun revalidation for ${key} returned no record`);
				}
				// A malformed cold response is returned to its callers, never cached.
				return fresh;
			}
			// Invalidation detaches old requests so they cannot repopulate the cache.
			if (inflight.get(key) !== request) return fresh;
			cache.set(key, { data: fresh, fetchedAt: Date.now() });
			if (
				entry &&
				opts?.onDrift &&
				fresh.signature &&
				entry.data.signature &&
				fresh.signature !== entry.data.signature
			) {
				opts.onDrift(key, entry.data, fresh);
			}
			return fresh;
		})
		.finally(() => {
			if (inflight.get(key) === request) inflight.delete(key);
		});
	inflight.set(key, request);

	if (entry) {
		// Revalidation failures leave the stale data available and allow a retry.
		void request.catch((err) => {
			console.warn(`[prerunSwr] Revalidating ${key} failed:`, err);
		});
		return entry.data;
	}
	return request;
}

/** Drop a single key (or the whole cache). Use after explicit board edits. */
export function invalidatePrerunCache(key?: string): void {
	if (key === undefined) {
		cache.clear();
		inflight.clear();
	} else {
		cache.delete(key);
		inflight.delete(key);
	}
}
