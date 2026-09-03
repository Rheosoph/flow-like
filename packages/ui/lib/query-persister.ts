"use client";

import { experimental_createQueryPersister } from "@tanstack/query-persist-client-core";
import { createStore, del, entries, get, set } from "idb-keyval";

/**
 * Per-query persistence with an opinionated retention policy.
 *
 * The legacy whole-client persister snapshotted the entire dehydrated cache
 * (12MB observed — 96% of it five oversized admin/detail queries) on every
 * change. This persister writes each query individually as it resolves and
 * restores lazily on first mount, with three layers keeping trash out:
 *
 * 1. `meta.persist` on a query: `false` never persists, `true` always does.
 * 2. A denylist of key prefixes measured to be huge and worthless offline.
 * 3. A hard per-entry size cap in the storage adapter — the backstop that
 *    keeps any future query from turning the cache into a dump.
 */

/** Bump to invalidate every persisted entry on policy/schema changes. */
const POLICY_BUSTER = "fl-qp-v1";

const DEFAULT_MAX_ENTRY_BYTES = 128 * 1024;

const DEFAULT_MAX_AGE_MS = 24 * 60 * 60 * 1000;

/**
 * Key prefixes that never persist. Evidence from the legacy cache blob
 * (sizes per prefix): ai-act 5.7MB, getRoles 1.7MB,
 * getRoutes 1.7MB (routes are already persisted via routeStorage), plus
 * admin dashboards which must always be live.
 *
 * The size cap is a backstop, not a substitute for this list: it is applied to
 * the *finished* string, so an oversized query still pays a full JSON.stringify
 * on the main thread before being thrown away — on every fetch, not just once.
 * Anything known to be large belongs here so that cost is never paid at all.
 * `project-runs` refetches every 60s carrying each run's whole input payload,
 * and `getBoardSummaries` with metrics/node_types has always exceeded the cap.
 */
const DENY_KEY_PREFIXES: ReadonlySet<string> = new Set([
	"admin",
	"ai-act",
	"getRoles",
	"getRoutes",
	"project-runs",
	"getBoardSummaries",
]);

interface PersistableQuery {
	queryKey: readonly unknown[];
	meta?: Record<string, unknown> | undefined;
}

/**
 * Decide whether a query's data is worth keeping across sessions.
 * `meta.persist` wins in both directions; otherwise queries persist by
 * default unless their key prefix is denylisted or unidentifiable.
 */
export function shouldPersistQuery(query: PersistableQuery): boolean {
	const persistMeta = query.meta?.persist;
	if (persistMeta === false) return false;
	if (persistMeta === true) return true;

	const head = query.queryKey[0];
	if (typeof head !== "string" || head.length === 0) return false;
	return !DENY_KEY_PREFIXES.has(head);
}

export interface QueryStorageBackend {
	get: (key: string) => Promise<string | undefined>;
	set: (key: string, value: string) => Promise<void>;
	del: (key: string) => Promise<void>;
	entries: () => Promise<Array<[string, string]>>;
}

export interface BoundedQueryStorage {
	getItem: (key: string) => Promise<string | undefined>;
	setItem: (key: string, value: string) => Promise<void>;
	removeItem: (key: string) => Promise<void>;
	entries: () => Promise<Array<[string, string]>>;
}

/**
 * Wrap a storage backend with the per-entry size cap. Oversized writes are
 * refused AND the previous entry under that key is dropped, so a query that
 * grew past the cap can never be restored stale from an older, smaller
 * version of itself.
 *
 * All backend failures are swallowed: persistence is strictly best-effort,
 * and a broken storage layer (private-mode browsers, blocked IndexedDB)
 * must never fail the query it decorates.
 */
export function createBoundedStorage(
	backend: QueryStorageBackend,
	maxEntryBytes: number = DEFAULT_MAX_ENTRY_BYTES,
): BoundedQueryStorage {
	return {
		getItem: async (key) => {
			try {
				return await backend.get(key);
			} catch {
				return undefined;
			}
		},
		setItem: async (key, value) => {
			try {
				if (value.length > maxEntryBytes) {
					await backend.del(key);
					return;
				}
				await backend.set(key, value);
			} catch {
				/* best effort */
			}
		},
		removeItem: async (key) => {
			try {
				await backend.del(key);
			} catch {
				/* best effort */
			}
		},
		entries: async () => {
			try {
				return await backend.entries();
			} catch {
				return [];
			}
		},
	};
}

/** idb-keyval backend on a dedicated database, opened lazily (SSR-safe). */
function createIdbBackend(): QueryStorageBackend {
	let store: ReturnType<typeof createStore> | undefined;
	const getStore = () => {
		store ??= createStore("flow-like-query-cache", "queries");
		return store;
	};
	return {
		get: (key) => get<string>(key, getStore()),
		set: (key, value) => set(key, value, getStore()),
		del: (key) => del(key, getStore()),
		entries: () => entries<string, string>(getStore()),
	};
}

export interface SmartQueryPersisterOptions {
	maxAge?: number;
	maxEntryBytes?: number;
	backend?: QueryStorageBackend;
}

/**
 * Create the per-query persister. Attach `persisterFn` to the QueryClient's
 * `defaultOptions.queries.persister` and schedule `persisterGc()` once per
 * session on idle to sweep expired/busted entries.
 */
export function createSmartQueryPersister(
	options: SmartQueryPersisterOptions = {},
) {
	const storage = createBoundedStorage(
		options.backend ?? createIdbBackend(),
		options.maxEntryBytes,
	);
	return experimental_createQueryPersister({
		storage,
		maxAge: options.maxAge ?? DEFAULT_MAX_AGE_MS,
		buster: POLICY_BUSTER,
		prefix: "fl-q",
		filters: {
			predicate: (query) => shouldPersistQuery(query),
		},
	});
}

/**
 * Remove the legacy whole-client cache blob (key "reactQuery" in the
 * default idb-keyval store) left behind by the previous persister.
 */
export async function cleanupLegacyQueryCacheBlob(): Promise<void> {
	try {
		await del("reactQuery");
	} catch {
		/* best effort — the blob simply expires otherwise */
	}
}
