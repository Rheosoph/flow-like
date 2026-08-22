import { type UseStore, createStore, del, get, keys, set } from "idb-keyval";
import type { Surface } from "../components/a2ui/types";
import { hasExpiredAssetUrl } from "./stable-asset-url";

/**
 * Last-known-good surfaces.
 *
 * A page whose onLoad workflow builds its content used to show a skeleton for the entire run.
 * Keeping the surface the workflow produced last time turns that into an instant render of
 * real content that the run then replaces — so the wait is spent looking at the page instead
 * of at a placeholder.
 *
 * What makes a cached surface replayable is narrow, and every part of it is in the key:
 *  - the page revision, because an edited page renders differently;
 *  - the query parameters, because the workflow receives them and its output depends on them;
 *  - the signed-in identity, because a surface is built from that account's data.
 * Anything that does not match is a miss, never a stale render.
 */

// Created on first use rather than at import: `createStore` opens the database immediately, and
// this module is also loaded where there is no IndexedDB to open (server render, tests).
let store: UseStore | undefined;
function surfaceStore(): UseStore {
	store ??= createStore("flow-like-page-surface-cache", "page-surface-cache");
	return store;
}

/** Entries are whole rendered surfaces, so the budget is deliberately small. */
const MAX_ENTRIES = 48;
/** A surface past this size is not worth the storage it would displace. */
const MAX_ENTRY_BYTES = 2_000_000;
const MANIFEST_KEY = "__manifest__";

/** The store the surface cache used to share with a2ui page state. */
const LEGACY_PAGE_STATE_STORE = "flow-like-page-state";
const LEGACY_SUFFIX = ":surface-cache";

interface PageSurfaceCacheRecord {
	readonly surface: Surface;
	readonly cachedAt: number;
}

/** key → write time, so eviction never has to read the entries themselves. */
type CacheManifest = Record<string, number>;

export interface PageSurfaceIdentity {
	readonly appId: string;
	readonly pageId: string;
	/** Revision of the page payload the surface was built from. */
	readonly pageUpdatedAt: string;
	/** Normalized query parameters, from `pageSurfaceQueryKey`. */
	readonly queryKey: string;
	/** Identifies the signed-in account, or "anonymous". */
	readonly userKey: string;
}

/**
 * A stable signature for the query parameters a page was rendered with. Order must not matter
 * — `?a=1&b=2` and `?b=2&a=1` reach the workflow as the same input — so the pairs are sorted.
 */
export function pageSurfaceQueryKey(search: string | undefined): string {
	if (!search) return "";
	const params = new URLSearchParams(
		search.startsWith("?") ? search.slice(1) : search,
	);
	return [...params.entries()]
		.map(([key, value]) => `${key}=${value}`)
		.toSorted()
		.join("&");
}

/**
 * Only the trailing query signature is free-form, so splitting from the left stays
 * unambiguous no matter what a parameter contains.
 */
const SEP = " ";

function cacheKey(identity: PageSurfaceIdentity): string {
	return [
		identity.appId,
		identity.pageId,
		identity.userKey,
		identity.pageUpdatedAt,
		identity.queryKey,
	].join(SEP);
}

/** Entries for the same page and account, regardless of revision or parameters. */
function pagePrefix(identity: PageSurfaceIdentity): string {
	return [identity.appId, identity.pageId, identity.userKey, ""].join(SEP);
}

function revisionOf(key: string, prefix: string): string {
	return key.slice(prefix.length).split(SEP)[0] ?? "";
}

async function readManifest(): Promise<CacheManifest> {
	return (await get<CacheManifest>(MANIFEST_KEY, surfaceStore())) ?? {};
}

/**
 * Holds the cache to its budget. A page's superseded revisions go first — nothing will ever
 * ask for them again — and only then are the oldest surviving entries dropped.
 */
export function selectEvictions(
	manifest: CacheManifest,
	keepKey: string,
	keepPrefix: string,
	keepRevision: string,
): string[] {
	const superseded = new Set(
		Object.keys(manifest).filter(
			(key) =>
				key.startsWith(keepPrefix) &&
				revisionOf(key, keepPrefix) !== keepRevision,
		),
	);

	const survivors = Object.entries(manifest).filter(
		([key]) => !superseded.has(key),
	);
	const overflow = survivors.length - MAX_ENTRIES;
	if (overflow <= 0) return [...superseded];

	const oldestFirst = survivors
		.filter(([key]) => key !== keepKey)
		.toSorted((a, b) => a[1] - b[1])
		.slice(0, overflow)
		.map(([key]) => key);

	return [...superseded, ...oldestFirst];
}

export async function readPageSurfaceCache(
	identity: PageSurfaceIdentity | null,
): Promise<Surface | null> {
	if (!identity?.appId || !identity.pageId || !identity.pageUpdatedAt) {
		return null;
	}

	try {
		const key = cacheKey(identity);
		const record = await get<PageSurfaceCacheRecord>(key, surfaceStore());
		if (!record?.surface) return null;

		// A surface is stored with its media already signed, and a signature outlives
		// neither the credential that made it nor the day. Replaying one whose links
		// have died shows broken images that nothing on the page can repair — the
		// storage paths they were signed from are not in the record — so a stale entry
		// is dropped and the run builds the page from scratch instead.
		if (hasExpiredAssetUrl(record.surface)) {
			void del(key, surfaceStore()).catch(() => undefined);
			return null;
		}

		return record.surface;
	} catch {
		return null;
	}
}

export async function writePageSurfaceCache(
	identity: PageSurfaceIdentity | null,
	surface: Surface,
): Promise<void> {
	if (!identity?.appId || !identity.pageId || !identity.pageUpdatedAt) {
		return;
	}

	try {
		const record: PageSurfaceCacheRecord = { surface, cachedAt: Date.now() };
		if (JSON.stringify(record.surface).length > MAX_ENTRY_BYTES) return;

		const key = cacheKey(identity);
		await set(key, record, surfaceStore());

		const manifest = await readManifest();
		manifest[key] = record.cachedAt;

		const evicted = selectEvictions(
			manifest,
			key,
			pagePrefix(identity),
			identity.pageUpdatedAt,
		);
		for (const staleKey of evicted) {
			delete manifest[staleKey];
			await del(staleKey, surfaceStore()).catch(() => undefined);
		}

		await set(MANIFEST_KEY, manifest, surfaceStore());
	} catch {
		// IndexedDB unavailable or over quota: the page still renders, just without a head start.
	}
}

/**
 * Removes surfaces written under the old scheme, which shared a store with a2ui page state and
 * had no eviction of any kind — they would otherwise sit in IndexedDB forever.
 */
export async function purgeLegacyPageSurfaceCache(): Promise<void> {
	try {
		const legacyStore = createStore(LEGACY_PAGE_STATE_STORE, "page-state");
		const legacyKeys = await keys(legacyStore);
		for (const key of legacyKeys) {
			if (String(key).endsWith(LEGACY_SUFFIX)) {
				await del(key, legacyStore).catch(() => undefined);
			}
		}
	} catch {
		// A store that cannot be opened has nothing to purge.
	}
}
