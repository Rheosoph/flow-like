// Stale-while-revalidate cache for prerun analysis responses.
//
// The server-side prerun cache (in flow-like-api) already collapses repeat
// requests on its end, but a client-side cache eliminates the round-trip
// entirely on rapid re-checks (think: user clicks Run, then Run again, or
// switches between two workflow tabs). Each cached entry carries the
// server's signature — when a background revalidation returns a different
// signature, listeners are notified so the UI can prompt the user.

interface PrerunLike {
	signature?: string;
}

interface CacheEntry<T extends PrerunLike> {
	data: T;
	fetchedAt: number;
	inflight?: Promise<T>;
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

// Revalidate after 15s of cached age. Anything fresher than that is served
// straight from cache without spawning a background fetch — keeps burst
// re-renders cheap. The server-side cache invalidates instantly on board
// saves, so 15s of staleness is the worst case in practice.
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
 * cached value is returned synchronously; if it's older than the revalidate
 * threshold, a background fetch is kicked off and `onDrift` fires when the
 * resulting signature differs from the cached one.
 */
export async function prerunSwr<T extends PrerunLike>(
	key: string,
	fetcher: () => Promise<T>,
	opts?: PrerunSwrOptions<T>,
): Promise<T> {
	const now = Date.now();
	const entry = cache.get(key) as CacheEntry<T> | undefined;

	if (entry) {
		const age = now - entry.fetchedAt;
		if (age > REVALIDATE_AFTER_MS && !entry.inflight) {
			const inflight = fetcher()
				.then((fresh) => {
					const previous = cache.get(key) as CacheEntry<T> | undefined;
					cache.set(key, { data: fresh, fetchedAt: Date.now() });
					if (
						opts?.onDrift &&
						previous &&
						fresh.signature &&
						previous.data.signature &&
						fresh.signature !== previous.data.signature
					) {
						opts.onDrift(key, previous.data, fresh);
					}
					return fresh;
				})
				.catch((err) => {
					// Transient failure — keep the existing cache entry intact,
					// just clear the inflight marker so the next call retries.
					const existing = cache.get(key) as CacheEntry<T> | undefined;
					if (existing) {
						cache.set(key, { ...existing, inflight: undefined });
					}
					throw err;
				});
			cache.set(key, { ...entry, inflight: inflight as Promise<PrerunLike> });
		}
		return entry.data;
	}

	const data = await fetcher();
	cache.set(key, { data, fetchedAt: now });
	return data;
}

/** Drop a single key (or the whole cache). Use after explicit board edits. */
export function invalidatePrerunCache(key?: string): void {
	if (key === undefined) {
		cache.clear();
	} else {
		cache.delete(key);
	}
}
