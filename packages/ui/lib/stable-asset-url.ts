/**
 * Collapses re-signed storage URLs back onto the first URL we saw for the same
 * object.
 *
 * Cloud signers stamp the current time into every signature, so asking for the
 * same icon twice yields two different URL strings. That breaks two things at
 * once: the browser treats the new string as a new resource and re-downloads
 * artwork it already holds, and cached query payloads compare unequal, so lists
 * re-render and reorder on every background refetch.
 *
 * Servers cannot reliably solve this alone — on serverless deployments each
 * request may be handled by a different process, so there is no shared place to
 * remember what was handed out. The client can: it is the one party that sees
 * every URL for a given object over time.
 *
 * Only the query string differs between two signatures of the same object, so
 * the object's origin and path make a stable identity. Entries persist to
 * localStorage, otherwise every reload would re-download everything once —
 * which matters here because the query cache itself is persisted and replays
 * yesterday's URLs on startup.
 *
 * This relies on media being immutable per path: replacing an app icon mints a
 * new id and writes a new object rather than overwriting the old one, so a
 * remembered URL can never point at content that has since changed. Do not
 * reuse this for paths that are written in place.
 */

const STORAGE_KEY = "flow-like.asset-urls";
const MAX_ENTRIES = 600;
const PERSIST_THROTTLE_MS = 2000;

/**
 * Stop reusing a URL before it actually expires. A signature that dies while a
 * page is open would show a broken image until the next refetch.
 */
const EXPIRY_SAFETY_MARGIN_MS = 10 * 60 * 1000;

interface StoredUrl {
	url: string;
	expiresAt: number;
}

type Registry = Map<string, StoredUrl>;

let registry: Registry | null = null;
let pendingPersist: ReturnType<typeof setTimeout> | null = null;

function parseCompactUtc(value: string): number | undefined {
	// AWS/GCP sign with ISO8601 basic format: 20260726T120000Z
	const match = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(value);
	if (!match) return undefined;
	const [, year, month, day, hour, minute, second] = match;
	return Date.UTC(
		Number(year),
		Number(month) - 1,
		Number(day),
		Number(hour),
		Number(minute),
		Number(second),
	);
}

/**
 * When the signature in this URL stops being valid, or `undefined` if the URL
 * carries no signature at all — unsigned URLs are already stable and are handed
 * back untouched.
 */
function signatureExpiry(url: URL): number | undefined {
	const params = url.searchParams;

	for (const [dateKey, ttlKey] of [
		["X-Amz-Date", "X-Amz-Expires"],
		["X-Goog-Date", "X-Goog-Expires"],
	]) {
		const signedAt = params.get(dateKey);
		const ttlSeconds = params.get(ttlKey);
		if (!signedAt || !ttlSeconds) continue;
		const start = parseCompactUtc(signedAt);
		const ttl = Number(ttlSeconds);
		if (start === undefined || !Number.isFinite(ttl)) continue;
		return start + ttl * 1000;
	}

	// Azure SAS states its deadline outright.
	const azureExpiry = params.get("se");
	if (azureExpiry) {
		const parsed = Date.parse(azureExpiry);
		if (!Number.isNaN(parsed)) return parsed;
	}

	return undefined;
}

function loadRegistry(): Registry {
	if (registry) return registry;

	registry = new Map();
	if (typeof window === "undefined") return registry;

	try {
		const raw = window.localStorage.getItem(STORAGE_KEY);
		if (!raw) return registry;
		const parsed = JSON.parse(raw) as Record<string, StoredUrl>;
		const now = Date.now();
		for (const [key, entry] of Object.entries(parsed)) {
			if (
				typeof entry?.url === "string" &&
				typeof entry?.expiresAt === "number" &&
				entry.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now
			) {
				registry.set(key, entry);
			}
		}
	} catch {
		// A corrupt or oversized store is not worth failing a render over; the
		// worst case is that assets are fetched under their newest URL once.
	}

	return registry;
}

function persist() {
	if (typeof window === "undefined" || !registry) return;
	try {
		window.localStorage.setItem(
			STORAGE_KEY,
			JSON.stringify(Object.fromEntries(registry)),
		);
	} catch {
		// Quota exhausted or storage disabled — keep the in-memory registry.
	}
}

function schedulePersist() {
	if (typeof window === "undefined" || pendingPersist !== null) return;
	pendingPersist = setTimeout(() => {
		pendingPersist = null;
		persist();
	}, PERSIST_THROTTLE_MS);
}

function prune(store: Registry, now: number) {
	for (const [key, entry] of store) {
		if (entry.expiresAt - EXPIRY_SAFETY_MARGIN_MS <= now) store.delete(key);
	}

	if (store.size <= MAX_ENTRIES) return;

	// Still too many live entries: drop the ones closest to expiring, since
	// those are the soonest to stop being reusable anyway.
	const byExpiry = [...store.entries()].sort(
		(a, b) => a[1].expiresAt - b[1].expiresAt,
	);
	for (const [key] of byExpiry.slice(0, store.size - MAX_ENTRIES)) {
		store.delete(key);
	}
}

/**
 * Returns a previously seen, still-valid URL for the same object, otherwise
 * remembers and returns the given one. Unsigned URLs (`asset://`, `data:`,
 * relative paths, plain HTTP) pass straight through — they never change.
 */
export function stableAssetUrl<T extends string | null | undefined>(raw: T): T {
	if (!raw || typeof raw !== "string") return raw;

	let parsed: URL;
	try {
		parsed = new URL(raw);
	} catch {
		return raw;
	}

	const expiresAt = signatureExpiry(parsed);
	if (expiresAt === undefined) return raw;

	const key = `${parsed.origin}${parsed.pathname}`;
	const store = loadRegistry();
	const now = Date.now();

	const known = store.get(key);
	if (known && known.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now) {
		return known.url as T;
	}

	store.set(key, { url: raw, expiresAt });
	prune(store, now);
	schedulePersist();
	return raw;
}

interface AssetBearingMetadata {
	icon?: string | null;
	thumbnail?: string | null;
	preview_media?: string[];
}

/**
 * Applies {@link stableAssetUrl} to a metadata record's media fields, returning
 * the original object untouched when nothing needed rewriting so callers keep
 * referential equality.
 */
export function stabilizeMetadata<T extends AssetBearingMetadata>(
	metadata: T,
): T;
export function stabilizeMetadata<T extends AssetBearingMetadata>(
	metadata: T | undefined,
): T | undefined;
export function stabilizeMetadata<T extends AssetBearingMetadata>(
	metadata: T | undefined,
): T | undefined {
	if (!metadata) return metadata;

	const icon = stableAssetUrl(metadata.icon);
	const thumbnail = stableAssetUrl(metadata.thumbnail);
	const previewMedia = metadata.preview_media?.map(stableAssetUrl);

	const previewChanged = previewMedia?.some(
		(url, index) => url !== metadata.preview_media?.[index],
	);

	if (
		icon === metadata.icon &&
		thumbnail === metadata.thumbnail &&
		!previewChanged
	) {
		return metadata;
	}

	return {
		...metadata,
		icon,
		thumbnail,
		...(previewMedia ? { preview_media: previewMedia } : {}),
	};
}

/** Convenience for the `{ url }` shape the storage download endpoints return. */
export function stabilizeSignedUrls<T extends { url?: string | null }>(
	items: T[],
): T[] {
	return items.map((item) => {
		const url = stableAssetUrl(item.url);
		return url === item.url ? item : { ...item, url };
	});
}

/** Convenience for the `[app, metadata][]` shape the app listings return. */
export function stabilizeMetadataEntries<A, M extends AssetBearingMetadata>(
	entries: [A, M | undefined][],
): [A, M | undefined][] {
	return entries.map(([app, metadata]) => [app, stabilizeMetadata(metadata)]);
}

/** Test seam: drops both the in-memory registry and its persisted copy. */
export function resetStableAssetUrls() {
	registry = null;
	if (pendingPersist !== null) {
		clearTimeout(pendingPersist);
		pendingPersist = null;
	}
	if (typeof window !== "undefined") {
		try {
			window.localStorage.removeItem(STORAGE_KEY);
		} catch {}
	}
}
