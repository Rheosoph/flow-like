/**
 * One live signed URL per storage path.
 *
 * Surfaces store durable storage paths (`media/logo.jpg`), never signed URLs: a
 * signature is a credential, and a credential written into content is content
 * that stops working. On AWS/GCP/R2 a download link is signed with a scoped
 * session credential and therefore dies inside the hour
 * (`RuntimeCredentials::signing_ttl`), which is far shorter than a dashboard
 * stays open. Anything that bakes one into a page, a cached surface or a
 * workflow message paints a broken image the moment it lapses, with no way back
 * — the path it was signed from is no longer anywhere in the record.
 *
 * So the path stays, and the URL lives here instead. Two things make that cheap
 * enough to do per component:
 *
 *  - **Reuse is bounded by the signature itself.** Every entry reads its own
 *    deadline out of the URL it was given, so a path is signed again only when
 *    its current link is genuinely close to death — not on a guessed interval,
 *    and not once per component.
 *  - **Concurrent asks become one request.** Thirty images mounting in the same
 *    commit resolve in the same tick, so they are collected and signed in a
 *    single batch. The storage backends split that at the route's own cap.
 *
 * Entries are per session and in memory. Persisting them would only trade a
 * request for the risk of serving a path whose object was replaced meanwhile;
 * `stableAssetUrl` already keeps the *browser's* image cache warm across
 * reloads by collapsing re-signed links back onto the string it handed out
 * before.
 */

import type { IStorageState } from "../state/backend-state/storage-state";
import type { IStorageItemActionResult } from "../state/backend-state/types";
import { signedUrlExpiry } from "./stable-asset-url";

/**
 * Sign again this long before the signature dies. It covers the round trip plus
 * the time a throttled background tab may sit on its timers, and it is wider
 * than `stableAssetUrl`'s own pin window, so the re-sign returns a genuinely
 * fresh link rather than the one about to lapse.
 */
const REFRESH_MARGIN_MS = 5 * 60 * 1000;

/** How long a path that could not be signed is left alone before retrying. */
const FAILURE_BACKOFF_MS = 30 * 1000;

/**
 * Floor on how often one path is signed. A credential nearly out of life yields
 * a URL that is already inside {@link REFRESH_MARGIN_MS}; without this floor
 * such a URL would ask to be replaced on every render.
 */
const MIN_REFRESH_INTERVAL_MS = 30 * 1000;

/**
 * Entries are small and a session touches few paths, but a long-lived board
 * cycling through generated media should still not grow without bound.
 */
const MAX_ENTRIES = 512;

export interface ResolvedAssetUrl {
	/** What to hand the element. The path itself when signing failed. */
	readonly url: string;
	/** The signature's own deadline, or `Infinity` when it carries none. */
	readonly expiresAt: number;
	/** When to resolve again — already inset by the refresh margin. */
	readonly usableUntil: number;
	/** False when this is the raw path handed back after a failed sign. */
	readonly resolved: boolean;
}

interface QueuedBatch {
	readonly storageState: IStorageState;
	readonly paths: string[];
	readonly settle: Map<string, (entry: ResolvedAssetUrl) => void>;
	timer: ReturnType<typeof setTimeout> | null;
}

const cache = new Map<string, ResolvedAssetUrl>();
const inFlight = new Map<string, Promise<ResolvedAssetUrl>>();
const lastSignedAt = new Map<string, number>();
const queues = new Map<string, QueuedBatch>();

const DIRECT_URL_PREFIXES = [
	"http://",
	"https://",
	"data:",
	"blob:",
	"asset://",
	"tauri://",
	"file://",
];

function isDirectUrl(value: string): boolean {
	return DIRECT_URL_PREFIXES.some((prefix) => value.startsWith(prefix));
}

/**
 * A path that is rooted rather than app-relative: `/Users/…` on disk, `C:\…` on
 * Windows, or an origin-relative web path like `/images/logo.png`. Nothing here
 * can name an object in app storage, whose paths are always relative.
 */
export function isRootedPath(value: string): boolean {
	return value.startsWith("/") || /^[A-Za-z]:[/\\]/.test(value);
}

/**
 * True for a value that names an object in app storage rather than addressing
 * one directly. Storage paths are app-relative and carry no scheme, except for
 * the optional `storage://` marker that {@link normalizeStorageAssetPath}
 * strips.
 */
export function isStorageAssetPath(
	value: string | null | undefined,
): value is string {
	if (!value) return false;
	return !isDirectUrl(value) && !isRootedPath(value);
}

export function normalizeStorageAssetPath(value: string): string {
	return value.replace(/^storage:\/\//, "");
}

/**
 * How a local file is addressed inside the desktop shell. Tauri serves the
 * filesystem over its own protocol; a bare path would resolve against the page.
 */
export function localFileAssetUrl(path: string): string {
	return `asset://localhost${path}`;
}

function cacheKey(appId: string, path: string): string {
	return `${appId}\u0000${path}`;
}

function isUsable(
	entry: ResolvedAssetUrl | undefined,
): entry is ResolvedAssetUrl {
	return entry !== undefined && entry.usableUntil > Date.now();
}

function signedEntry(url: string, now: number): ResolvedAssetUrl {
	const expiresAt = signedUrlExpiry(url) ?? Number.POSITIVE_INFINITY;
	const usableUntil =
		expiresAt === Number.POSITIVE_INFINITY
			? Number.POSITIVE_INFINITY
			: Math.max(expiresAt - REFRESH_MARGIN_MS, now + MIN_REFRESH_INTERVAL_MS);
	return { url, expiresAt, usableUntil, resolved: true };
}

function failedEntry(path: string, now: number): ResolvedAssetUrl {
	return {
		url: path,
		expiresAt: now,
		usableUntil: now + FAILURE_BACKOFF_MS,
		resolved: false,
	};
}

function prune(now: number) {
	if (cache.size <= MAX_ENTRIES) return;

	for (const [key, entry] of cache) {
		if (entry.usableUntil <= now) cache.delete(key);
	}
	if (cache.size <= MAX_ENTRIES) return;

	// Still over budget: drop the entries closest to needing a new signature,
	// since those are the soonest to stop being reusable anyway.
	const byDeadline = [...cache.entries()].sort(
		(left, right) => left[1].usableUntil - right[1].usableUntil,
	);
	for (const [key] of byDeadline.slice(0, cache.size - MAX_ENTRIES)) {
		cache.delete(key);
	}
}

async function flush(appId: string) {
	const queue = queues.get(appId);
	if (!queue) return;
	queues.delete(appId);

	let results: IStorageItemActionResult[] = [];
	try {
		// The backends split this at the route's per-request cap themselves.
		results = await queue.storageState.downloadStorageItems(appId, queue.paths);
	} catch (error) {
		console.warn("[assetUrlCache] Failed to sign storage assets:", error);
	}

	const signed = new Map<string, string>();
	for (const result of results) {
		if (result.url && !result.error) signed.set(result.prefix, result.url);
	}

	const now = Date.now();
	for (const path of queue.paths) {
		const url = signed.get(path);
		const entry = url ? signedEntry(url, now) : failedEntry(path, now);
		const key = cacheKey(appId, path);
		cache.set(key, entry);
		inFlight.delete(key);
		queue.settle.get(path)?.(entry);
	}
	prune(now);
}

function enqueue(
	appId: string,
	path: string,
	storageState: IStorageState,
): Promise<ResolvedAssetUrl> {
	const key = cacheKey(appId, path);
	const pending = inFlight.get(key);
	if (pending) return pending;

	let queue = queues.get(appId);
	if (!queue) {
		queue = { storageState, paths: [], settle: new Map(), timer: null };
		queues.set(appId, queue);
		// A macrotask, so every component that mounted in this commit lands in
		// the same request. Effects of one commit all run before it fires.
		queue.timer = setTimeout(() => void flush(appId), 0);
	}

	const batch = queue;
	const promise = new Promise<ResolvedAssetUrl>((resolve) => {
		batch.settle.set(path, resolve);
	});
	batch.paths.push(path);
	inFlight.set(key, promise);
	lastSignedAt.set(key, Date.now());
	return promise;
}

/**
 * A usable URL for `path`, from cache when one is still comfortably alive and
 * from a coalesced signing request otherwise.
 *
 * A path that cannot be signed resolves to the path itself, so a caller renders
 * the same broken link it would have rendered before rather than nothing at
 * all, and the failure is retried after a short backoff.
 */
export function resolveAssetUrl(
	appId: string,
	rawPath: string,
	storageState: IStorageState,
): Promise<ResolvedAssetUrl> {
	const path = normalizeStorageAssetPath(rawPath);
	const cached = cache.get(cacheKey(appId, path));
	if (isUsable(cached)) return Promise.resolve(cached);
	return enqueue(appId, path, storageState);
}

/**
 * The cached entry for `path`, or `undefined` when there is none worth using.
 *
 * Read synchronously during the first render so artwork already resolved on
 * another surface paints immediately instead of flashing a placeholder.
 */
export function peekAssetUrl(
	appId: string | undefined,
	rawPath: string | undefined,
): ResolvedAssetUrl | undefined {
	if (!appId || !rawPath) return undefined;
	const entry = cache.get(cacheKey(appId, normalizeStorageAssetPath(rawPath)));
	return isUsable(entry) ? entry : undefined;
}

/**
 * Drops the cached URL for `path` so the next resolve signs it again. Returns
 * false when the path was signed too recently to be worth retrying — a link the
 * store rejects for a reason signing cannot fix (the object is gone, the caller
 * lost access) would otherwise re-sign on every failed load.
 */
export function invalidateAssetUrl(
	appId: string | undefined,
	rawPath: string | undefined,
): boolean {
	if (!appId || !rawPath) return false;

	const key = cacheKey(appId, normalizeStorageAssetPath(rawPath));
	const now = Date.now();
	if (now - (lastSignedAt.get(key) ?? 0) < MIN_REFRESH_INTERVAL_MS)
		return false;

	cache.delete(key);
	return true;
}

/** Test seam: drops every entry, in-flight request and pending batch. */
export function resetAssetUrlCache() {
	for (const queue of queues.values()) {
		if (queue.timer !== null) clearTimeout(queue.timer);
	}
	queues.clear();
	cache.clear();
	inFlight.clear();
	lastSignedAt.clear();
}
