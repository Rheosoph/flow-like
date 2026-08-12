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

const STORAGE_KEY = "flow-like.asset-urls.v2";
const LEGACY_STORAGE_KEY = "flow-like.asset-urls";
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
	confirmed: boolean;
	replacement?: {
		url: string;
		expiresAt: number;
	};
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
		if (start === undefined || !Number.isFinite(ttl) || ttl <= 0) continue;
		return start + ttl * 1000;
	}

	// Azure SAS states its deadline outright.
	const azureExpiry = params.get("se");
	if (azureExpiry) {
		const parsed = Date.parse(azureExpiry);
		if (!Number.isNaN(parsed)) return parsed;
	}

	// Older S3/GCS V2 signatures use an epoch deadline.
	const legacyExpiry = params.get("Expires");
	if (
		legacyExpiry &&
		params.has("Signature") &&
		(params.has("AWSAccessKeyId") || params.has("GoogleAccessId"))
	) {
		const parsed = Number(legacyExpiry);
		if (Number.isFinite(parsed) && parsed > 0) return parsed * 1000;
	}

	return undefined;
}

/**
 * Signing timestamps and signatures may change without changing the resource.
 * Everything else is identity-bearing: versions, image transforms, response
 * overrides, signed headers and credential/session identity must not collide.
 */
const VOLATILE_SIGNATURE_PARAMS = new Set([
	"x-amz-date",
	"x-amz-expires",
	"x-amz-signature",
	"x-goog-date",
	"x-goog-expires",
	"x-goog-signature",
	"expires",
	"signature",
	"se",
	"sig",
	"st",
]);

function assetIdentity(url: URL): string {
	const stableParams = [...url.searchParams.entries()]
		.filter(([name]) => !VOLATILE_SIGNATURE_PARAMS.has(name.toLowerCase()))
		.sort(([leftName, leftValue], [rightName, rightValue]) => {
			const nameOrder = leftName.localeCompare(rightName);
			return nameOrder || leftValue.localeCompare(rightValue);
		});
	const query = new URLSearchParams(stableParams).toString();
	return `${url.origin}${url.pathname}${query ? `?${query}` : ""}${url.hash}`;
}

function parseSignedUrl(raw: string):
	| {
			key: string;
			expiresAt: number;
	  }
	| undefined {
	try {
		const parsed = new URL(raw);
		const expiresAt = signatureExpiry(parsed);
		if (expiresAt === undefined) return undefined;
		return { key: assetIdentity(parsed), expiresAt };
	} catch {
		return undefined;
	}
}

function loadRegistry(): Registry {
	if (registry) return registry;

	registry = new Map();
	if (typeof window === "undefined") return registry;

	try {
		// v1 keyed only by origin/path and could therefore collapse versions,
		// transforms and credentials. Never carry those entries into v2.
		window.localStorage.removeItem(LEGACY_STORAGE_KEY);
		const raw = window.localStorage.getItem(STORAGE_KEY);
		if (!raw) return registry;
		const parsed = JSON.parse(raw) as Record<
			string,
			Pick<StoredUrl, "url" | "expiresAt">
		>;
		const now = Date.now();
		for (const [key, entry] of Object.entries(parsed)) {
			const signed =
				typeof entry?.url === "string" ? parseSignedUrl(entry.url) : undefined;
			if (
				signed &&
				signed.key === key &&
				typeof entry?.expiresAt === "number" &&
				signed.expiresAt === entry.expiresAt &&
				entry.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now
			) {
				registry.set(key, { ...entry, confirmed: true });
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
		const now = Date.now();
		const confirmed = [...registry.entries()]
			.filter(
				([, entry]) =>
					entry.confirmed && entry.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now,
			)
			.map(([key, { url, expiresAt }]) => [key, { url, expiresAt }]);
		window.localStorage.setItem(
			STORAGE_KEY,
			JSON.stringify(Object.fromEntries(confirmed)),
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

	const signed = parseSignedUrl(raw);
	if (!signed) return raw;

	const store = loadRegistry();
	const now = Date.now();
	const { key, expiresAt } = signed;

	const known = store.get(key);
	if (known && known.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now) {
		if (
			known.url !== raw &&
			expiresAt > known.expiresAt &&
			expiresAt - EXPIRY_SAFETY_MARGIN_MS > now &&
			(!known.replacement || known.replacement.expiresAt < expiresAt)
		) {
			// Keep the newest signature as an immediate retry, but continue
			// returning the already-confirmed/cached URL on the happy path.
			known.replacement = { url: raw, expiresAt };
		}
		return known.url as T;
	}

	if (expiresAt - EXPIRY_SAFETY_MARGIN_MS <= now) return raw;

	store.set(key, { url: raw, expiresAt, confirmed: false });
	prune(store, now);
	return raw;
}

/**
 * Marks a URL as browser-proven. Only proven URLs are persisted across reloads,
 * so a transient 403/404 can never become a durable cache entry.
 */
export function confirmStableAssetUrl(raw: string | null | undefined): void {
	if (!raw) return;
	const signed = parseSignedUrl(raw);
	if (!signed) return;

	const store = loadRegistry();
	const entry = store.get(signed.key);
	if (!entry || entry.url !== raw) return;

	entry.confirmed = true;
	schedulePersist();
}

/**
 * Evicts a URL that failed to load and promotes the newest signature observed
 * for the same resource. Image components can retry the returned URL
 * immediately instead of waiting for another query refetch.
 */
export function recoverStableAssetUrl(
	raw: string | null | undefined,
): string | undefined {
	if (!raw) return undefined;
	const signed = parseSignedUrl(raw);
	if (!signed) return undefined;

	const store = loadRegistry();
	const entry = store.get(signed.key);
	if (!entry) return undefined;

	const now = Date.now();
	if (entry.url !== raw) {
		return entry.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now
			? entry.url
			: undefined;
	}

	const replacement = entry.replacement;
	if (
		replacement &&
		replacement.url !== raw &&
		replacement.expiresAt - EXPIRY_SAFETY_MARGIN_MS > now
	) {
		store.set(signed.key, {
			...replacement,
			confirmed: false,
		});
		persist();
		return replacement.url;
	}

	store.delete(signed.key);
	persist();
	return undefined;
}

/**
 * True once a signature's deadline has passed. Such a URL is dead: the store
 * answers 403 however many times it is asked, so rendering it only buys a
 * broken image and a wasted request.
 *
 * Callers use this to decide what to *paint*, never to delete data. A dead URL
 * still records which object the asset lives in, and the next metadata refresh
 * signs that same object again.
 */
export function isExpiredAssetUrl(raw: string | null | undefined): boolean {
	if (!raw) return false;
	const signed = parseSignedUrl(raw);
	if (!signed) return false;
	return signed.expiresAt <= Date.now();
}

export interface AssetBearingMetadata {
	icon?: string | null;
	thumbnail?: string | null;
	preview_media?: string[];
}

/**
 * Picks the better of two values for one media field.
 *
 * A signature is a credential, not an address. A cached record froze whichever
 * signature was current when it was written, so its media links die within a
 * day even while the rest of the record stays perfectly good — and a record
 * written before the app had artwork carries no link at all. Anything unsigned
 * (Tauri's `asset://` form, a `data:` URL, a plain public URL) keeps working
 * forever and is worth holding on to, because re-pointing an `<img>` at a
 * differently signed copy of artwork the browser already has re-downloads it.
 */
function durableMedia(
	cached: string | null | undefined,
	fresh: string | null | undefined,
): string | null | undefined {
	if (cached && !parseSignedUrl(cached)) return cached;
	return fresh ?? cached;
}

/**
 * Takes the media fields of `fresh` into `cached` wherever `cached` cannot
 * stand on its own, and returns `cached` untouched when it can — callers rely
 * on that reference to tell "nothing changed" from "resync".
 *
 * Only media is merged. Everything else stays as `cached` has it, because the
 * cached record is the copy the rest of the app treats as authoritative and
 * adopting names or timestamps from a background sync reorders lists under the
 * user.
 */
export function mergeMetadataMedia<T extends AssetBearingMetadata>(
	cached: T,
	fresh: AssetBearingMetadata | undefined,
): T {
	if (!fresh) return cached;

	const icon = durableMedia(cached.icon, fresh.icon);
	const thumbnail = durableMedia(cached.thumbnail, fresh.thumbnail);
	// Preview galleries are ordered sets rather than slots, so they are taken
	// whole: a cached gallery survives only while every entry in it is durable.
	const previewMedia =
		cached.preview_media?.length &&
		cached.preview_media.every((url) => !parseSignedUrl(url))
			? cached.preview_media
			: (fresh.preview_media ?? cached.preview_media);

	if (
		icon === cached.icon &&
		thumbnail === cached.thumbnail &&
		previewMedia === cached.preview_media
	) {
		return cached;
	}

	return {
		...cached,
		icon,
		thumbnail,
		...(previewMedia ? { preview_media: previewMedia } : {}),
	};
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
			window.localStorage.removeItem(LEGACY_STORAGE_KEY);
		} catch {}
	}
}
