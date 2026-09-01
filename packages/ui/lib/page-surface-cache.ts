import { type UseStore, createStore, del, get, keys, set } from "idb-keyval";
import type { Surface } from "../components/a2ui/types";
import { hasExpiredAssetUrl } from "./stable-asset-url";

/**
 * Last-known-good surfaces.
 *
 * A page whose onLoad workflow builds its content used to show a skeleton for the entire run.
 * Keeping the surface the workflow produced last time turns that into an instant render of
 * real content that the run then replaces, so the wait is spent looking at the page instead
 * of at a placeholder.
 *
 * What makes a cached surface replayable is narrow, and every part of it is in the key:
 *  - the page and execution-authority revisions, because either can change
 *    the rendered output or invalidate its opaque action selectors;
 *  - the route, because the workflow receives it as input;
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
	/** Combined Page payload and execution-authority revision. */
	readonly pageUpdatedAt: string;
	/** Normalized route passed to the page workflow. */
	readonly routeKey: string;
	/** Normalized query parameters, from `pageSurfaceQueryKey`. */
	readonly queryKey: string;
	/** Identifies the signed-in account, or "anonymous". */
	readonly userKey: string;
}

/**
 * Cache revision for the content and the exact governed execution contract.
 * The execution revision is omitted for preview and legacy surfaces that do
 * not have a Page Event authority map.
 */
export function pageSurfaceRevision(
	pageRevision: string | null | undefined,
	executionRevision?: string | null,
): string | undefined {
	if (!pageRevision) return undefined;
	return executionRevision
		? JSON.stringify([pageRevision, executionRevision])
		: pageRevision;
}

/** Stable route input for cache and execution identity. */
export function pageSurfaceRouteKey(route: string | undefined): string {
	const withoutFragment = (route ?? "/").trim().split("#", 1)[0] ?? "";
	const withoutQuery = withoutFragment.split("?", 1)[0] ?? "";
	const withLeadingSlash = withoutQuery.startsWith("/")
		? withoutQuery
		: `/${withoutQuery}`;
	const withoutTrailingSlash = withLeadingSlash.replace(/\/+$/, "");
	return withoutTrailingSlash || "/";
}

/**
 * A stable signature for the query parameters a page was rendered with. Order must not matter
 * because `?a=1&b=2` and `?b=2&a=1` reach the workflow as the same input. The pairs are sorted.
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
 * Only the trailing route/query signature is free-form, so splitting from the left stays
 * unambiguous no matter what a parameter contains.
 */
const SEP = " ";

/** Stable identity used both by IndexedDB and by render-time cache result matching. */
export function pageSurfaceCacheKey(identity: PageSurfaceIdentity): string {
	return [
		identity.appId,
		identity.pageId,
		identity.userKey,
		identity.pageUpdatedAt,
		JSON.stringify([identity.routeKey, identity.queryKey]),
	].join(SEP);
}

/** Entries for the same page and account, regardless of revision or parameters. */
function pagePrefix(identity: PageSurfaceIdentity): string {
	return [identity.appId, identity.pageId, identity.userKey, ""].join(SEP);
}

function revisionOf(key: string, prefix: string): string {
	return key.slice(prefix.length).split(SEP)[0] ?? "";
}

/** Dynamic Page authorization is tied to its originating run. This includes
 * signed capabilities and native `lda1_` action handles, neither of which may
 * survive in IndexedDB or be replayed from a cached surface. */
export function hasPageActionCapability(value: unknown): boolean {
	if (Array.isArray(value)) return value.some(hasPageActionCapability);
	if (!value || typeof value !== "object") return false;
	for (const [key, child] of Object.entries(value)) {
		if (
			(key === "pageAction" || key === "page_action") &&
			child &&
			typeof child === "object" &&
			!Array.isArray(child)
		) {
			const action = child as Record<string, unknown>;
			const actionId = action.actionId ?? action.action_id;
			if (typeof actionId === "string" && actionId.startsWith("lda1_")) {
				return true;
			}
		}
		if (
			(key === "capabilityJwt" || key === "capability_jwt") &&
			typeof child === "string" &&
			child.length > 0
		)
			return true;
		if (
			(key === "literalJson" || key === "literal_json") &&
			typeof child === "string"
		) {
			try {
				if (hasPageActionCapability(JSON.parse(child))) return true;
			} catch {
				// Invalid literal JSON is handled by its renderer and carries no parsed action.
			}
			continue;
		}
		if (hasPageActionCapability(child)) return true;
	}
	return false;
}

async function readManifest(): Promise<CacheManifest> {
	return (await get<CacheManifest>(MANIFEST_KEY, surfaceStore())) ?? {};
}

/**
 * Holds the cache to its budget. A page's superseded revisions go first because nothing will ever
 * ask for them again. Only then are the oldest surviving entries dropped.
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
		const key = pageSurfaceCacheKey(identity);
		const record = await get<PageSurfaceCacheRecord>(key, surfaceStore());
		if (!record?.surface) return null;
		if (hasPageActionCapability(record.surface)) {
			void del(key, surfaceStore()).catch(() => undefined);
			return null;
		}

		// A surface is stored with its media already signed, and a signature outlives
		// neither the credential that made it nor the day. Replaying one whose links
		// have died shows broken images that nothing on the page can repair. The
		// storage paths they were signed from are not in the record, so a stale entry
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
		const key = pageSurfaceCacheKey(identity);
		if (hasPageActionCapability(surface)) {
			await del(key, surfaceStore()).catch(() => undefined);
			return;
		}
		const record: PageSurfaceCacheRecord = { surface, cachedAt: Date.now() };
		if (JSON.stringify(record.surface).length > MAX_ENTRY_BYTES) return;

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
 * had no eviction of any kind. They would otherwise sit in IndexedDB forever.
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
