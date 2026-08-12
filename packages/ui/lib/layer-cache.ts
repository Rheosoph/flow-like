import { type ILayerCache, ILayerCacheScope } from "./schema/flow/board";

const TTL_UNITS: readonly [number, string, string][] = [
	[86400, "d", "day"],
	[3600, "h", "hour"],
	[60, "m", "minute"],
	[1, "s", "second"],
];

/** Compact lifetime for tight spots like a node header — "15m", "2h", "∞". */
export function formatCacheTtl(seconds?: number | null): string {
	if (!seconds || seconds <= 0) return "∞";
	for (const [size, short] of TTL_UNITS) {
		if (seconds >= size) {
			return `${Math.round((seconds / size) * 10) / 10}${short}`;
		}
	}
	return `${seconds}s`;
}

/** Full sentence for the settings dialog. */
export function describeCacheLifetime(seconds?: number | null): string {
	if (!seconds || seconds <= 0)
		return "Never expires — entries live until invalidated.";
	for (const [size, , long] of TTL_UNITS) {
		if (seconds >= size) {
			const amount = Math.round((seconds / size) * 10) / 10;
			return `Expires after ~${amount} ${long}${amount === 1 ? "" : "s"}.`;
		}
	}
	return `Expires after ${seconds} seconds.`;
}

/** Tooltip explaining why a call node is marked as cached. */
export function cacheIndicatorLabel(cache: ILayerCache): string {
	const parts = [
		`Cached results (${cache.scope === ILayerCacheScope.User ? "per user" : "shared across the app"})`,
		cache.ttl_seconds && cache.ttl_seconds > 0
			? `for ${formatCacheTtl(cache.ttl_seconds)}`
			: "until invalidated",
	];
	if (cache.prefix?.trim()) parts.push(`under "${cache.prefix.trim()}"`);
	return `${parts.join(" ")}. A hit skips the function body.`;
}
