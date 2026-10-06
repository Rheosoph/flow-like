import { appGlobalState, pageLocalState } from "../../lib/idb-storage";
import {
	isRuntimeNamespace,
	onRuntimeNamespaceClose,
	runtimeNamespaceActive,
} from "../../lib/service-runtime/session-scope";

type StateRecord = Record<string, unknown>;

export interface FrontendStateSnapshot {
	globalState: StateRecord;
	pageStates: Record<string, StateRecord>;
}

export interface FrontendStateStore {
	getSnapshot(): FrontendStateSnapshot;
	subscribe(listener: () => void): () => void;
	ensureLoaded(pageId: string): Promise<void>;
	setGlobalState(key: string, value: unknown): void;
	setPageState(pageId: string, key: string, value: unknown): void;
	clearPageState(pageId: string): void;
	handleMessage(message: unknown): boolean;
	dispose(): void;
}

export interface FrontendStatePersistence {
	global: Pick<typeof appGlobalState, "getAll" | "set">;
	page: Pick<typeof pageLocalState, "getAll" | "set" | "clearPage">;
}

const UNSAFE_RECORD_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function isSafeKey(key: unknown): key is string {
	return (
		typeof key === "string" && key.length > 0 && !UNSAFE_RECORD_KEYS.has(key)
	);
}

function immutableRecord<T>(
	...sources: Record<string, T>[]
): Record<string, T> {
	const result: Record<string, T> = Object.create(null);
	for (const source of sources) {
		for (const [key, value] of Object.entries(source)) {
			if (isSafeKey(key)) result[key] = value;
		}
	}
	return Object.freeze(result);
}

export const EMPTY_FRONTEND_STATE_RECORD: StateRecord = immutableRecord();

const persistence: FrontendStatePersistence = {
	global: appGlobalState,
	page: pageLocalState,
};

/** Share state across page providers and lifecycle runs belonging to one app. */
export function createFrontendStateStore(
	appId: string | undefined,
	storage: FrontendStatePersistence = persistence,
): FrontendStateStore {
	let snapshot: FrontendStateSnapshot = Object.freeze({
		globalState: EMPTY_FRONTEND_STATE_RECORD,
		pageStates: immutableRecord<StateRecord>(),
	});
	const listeners = new Set<() => void>();
	const pageLoads = new Map<string, Promise<void>>();
	const clearedPages = new Set<string>();
	let globalLoad: Promise<void> | undefined;
	let pendingWrites = Promise.resolve();
	const runtime = isRuntimeNamespace(appId);
	let disposed = false;
	let releaseRuntime: (() => void) | undefined;
	const persistentAppId = isSafeKey(appId) && !runtime ? appId : undefined;
	const available = () =>
		!disposed && (!runtime || runtimeNamespaceActive(appId));

	function publish(next: FrontendStateSnapshot) {
		if (!available()) return;
		snapshot = Object.freeze(next);
		for (const listener of listeners) listener();
	}

	function replacePage(pageId: string, state: StateRecord) {
		publish({
			...snapshot,
			pageStates: immutableRecord(snapshot.pageStates, { [pageId]: state }),
		});
	}

	function persist(write: (id: string) => Promise<void>) {
		if (!persistentAppId) return;
		// A clear must finish before a later set, or it could delete the new value.
		pendingWrites = pendingWrites
			.then(() => write(persistentAppId))
			.catch(() => console.error("Failed to persist frontend state"));
	}

	async function loadGlobal() {
		if (!persistentAppId) return;
		const persisted = await storage.global.getAll(persistentAppId);
		publish({
			...snapshot,
			globalState: immutableRecord(persisted, snapshot.globalState),
		});
	}

	async function loadPage(pageId: string) {
		if (!persistentAppId) return;
		const persisted = await storage.page.getAll(persistentAppId, pageId);
		// Writes win over the initial read; a clear also discards untouched keys.
		replacePage(
			pageId,
			immutableRecord(
				clearedPages.has(pageId) ? EMPTY_FRONTEND_STATE_RECORD : persisted,
				snapshot.pageStates[pageId] ?? EMPTY_FRONTEND_STATE_RECORD,
			),
		);
	}

	const store: FrontendStateStore = {
		getSnapshot: () => snapshot,
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		async ensureLoaded(pageId) {
			if (!available()) return;
			globalLoad ??= loadGlobal().catch(() => {
				globalLoad = undefined;
				console.error("Failed to load global state");
			});
			if (!isSafeKey(pageId)) {
				await globalLoad;
				return;
			}
			let pageLoad = pageLoads.get(pageId);
			if (!pageLoad) {
				pageLoad = loadPage(pageId).catch(() => {
					pageLoads.delete(pageId);
					console.error("Failed to load page state");
				});
				pageLoads.set(pageId, pageLoad);
			}
			await Promise.all([globalLoad, pageLoad]);
		},
		setGlobalState(key, value) {
			if (!available()) return;
			if (!isSafeKey(key)) return;
			persist((id) => storage.global.set(id, key, value));
			publish({
				...snapshot,
				globalState: immutableRecord(snapshot.globalState, { [key]: value }),
			});
		},
		setPageState(pageId, key, value) {
			if (!available()) return;
			if (!isSafeKey(pageId) || !isSafeKey(key)) return;
			persist((id) => storage.page.set(id, pageId, key, value));
			replacePage(
				pageId,
				immutableRecord(snapshot.pageStates[pageId] ?? {}, { [key]: value }),
			);
		},
		clearPageState(pageId) {
			if (!available()) return;
			if (!isSafeKey(pageId)) return;
			clearedPages.add(pageId);
			persist((id) => storage.page.clearPage(id, pageId));
			replacePage(pageId, EMPTY_FRONTEND_STATE_RECORD);
		},
		handleMessage(message) {
			if (!available()) return false;
			if (!message || typeof message !== "object" || Array.isArray(message)) {
				return false;
			}
			const payload = message as Record<string, unknown>;
			switch (payload.type) {
				case "setGlobalState":
					if (isSafeKey(payload.key) && Object.hasOwn(payload, "value")) {
						store.setGlobalState(payload.key, payload.value);
					}
					return true;
				case "setPageState": {
					const pageId = payload.page_id ?? payload.pageId;
					if (
						isSafeKey(pageId) &&
						isSafeKey(payload.key) &&
						Object.hasOwn(payload, "value")
					) {
						store.setPageState(pageId, payload.key, payload.value);
					}
					return true;
				}
				case "clearPageState": {
					const pageId = payload.page_id ?? payload.pageId;
					if (isSafeKey(pageId)) store.clearPageState(pageId);
					return true;
				}
				default:
					return false;
			}
		},
		dispose() {
			disposed = true;
			releaseRuntime?.();
			snapshot = Object.freeze({
				globalState: EMPTY_FRONTEND_STATE_RECORD,
				pageStates: immutableRecord<StateRecord>(),
			});
			listeners.clear();
			pageLoads.clear();
			clearedPages.clear();
		},
	};
	if (runtime)
		releaseRuntime = onRuntimeNamespaceClose(appId, () => store.dispose());
	return store;
}

const stores = new Map<string | undefined, FrontendStateStore>();

export function releaseFrontendStateStore(appId: string): void {
	stores.get(appId)?.dispose();
	stores.delete(appId);
}

export function getFrontendStateStore(
	appId: string | undefined,
): FrontendStateStore {
	let store = stores.get(appId);
	if (!store) {
		store = createFrontendStateStore(appId);
		stores.set(appId, store);
	}
	return store;
}
