export const SCREEN_TRUST_DB = "flow-like-screen-trust";
const STORE = "hosts";

export interface TrustedHost {
	key_id: string;
	public_key: string;
	label: string;
	host_peer_id: string;
	/** Unix seconds. */
	paired_at: number;
}

/** The host keys this device pinned by QR pairing, per signed-in account. */
export interface ScreenTrustStore {
	list(sub: string): Promise<TrustedHost[]>;
	put(sub: string, host: TrustedHost): Promise<void>;
	remove(sub: string, keyId: string): Promise<void>;
}

const recordKey = (sub: string, keyId: string) => `${sub}\0${keyId}`;

export function memoryTrustStore(): ScreenTrustStore {
	const hosts = new Map<string, TrustedHost>();
	return {
		async list(sub) {
			return [...hosts]
				.filter(([key]) => key.startsWith(`${sub}\0`))
				.map(([, host]) => host);
		},
		async put(sub, host) {
			hosts.set(recordKey(sub, host.key_id), host);
		},
		async remove(sub, keyId) {
			hosts.delete(recordKey(sub, keyId));
		},
	};
}

function database(): Promise<IDBDatabase> {
	if (!globalThis.indexedDB)
		return Promise.reject(new Error("IndexedDB is unavailable"));
	return new Promise((resolve, reject) => {
		const request = indexedDB.open(SCREEN_TRUST_DB, 1);
		request.onupgradeneeded = () => {
			if (!request.result.objectStoreNames.contains(STORE))
				request.result.createObjectStore(STORE);
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(new Error("The pairing store could not be opened"));
	});
}

async function run<T>(
	mode: IDBTransactionMode,
	act: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
	const db = await database();
	try {
		return await new Promise<T>((resolve, reject) => {
			const request = act(db.transaction(STORE, mode).objectStore(STORE));
			request.onsuccess = () => resolve(request.result);
			request.onerror = () =>
				reject(new Error("The pairing store could not be used"));
		});
	} finally {
		db.close();
	}
}

const wellFormed = (value: unknown): value is TrustedHost => {
	const host = value as Partial<TrustedHost> | null;
	return (
		typeof host?.key_id === "string" &&
		typeof host.public_key === "string" &&
		typeof host.label === "string" &&
		typeof host.host_peer_id === "string" &&
		typeof host.paired_at === "number"
	);
};

/** IndexedDB, with a memory copy that keeps pairings working for this page when the database is blocked. */
export function indexedDbTrustStore(): ScreenTrustStore {
	const shadow = memoryTrustStore();
	return {
		async list(sub) {
			const merged = new Map<string, TrustedHost>();
			try {
				const range = IDBKeyRange.bound(
					`${sub}\0`,
					`${sub}\u0001`,
					false,
					true,
				);
				const found = await run("readonly", (store) => store.getAll(range));
				for (const host of found.filter(wellFormed))
					merged.set(host.key_id, host);
			} catch (error) {
				console.warn("Could not read the paired computers", error);
			}
			for (const host of await shadow.list(sub)) merged.set(host.key_id, host);
			return [...merged.values()];
		},
		async put(sub, host) {
			await shadow.put(sub, host);
			try {
				await run("readwrite", (store) =>
					store.put(host, recordKey(sub, host.key_id)),
				);
			} catch (error) {
				console.warn("Could not save the paired computer", error);
			}
		},
		async remove(sub, keyId) {
			await shadow.remove(sub, keyId);
			try {
				await run("readwrite", (store) => store.delete(recordKey(sub, keyId)));
			} catch (error) {
				console.warn("Could not forget the paired computer", error);
			}
		},
	};
}

/** IndexedDB where the browser has it, otherwise memory (pairings then last until reload). */
export function createScreenTrustStore(): ScreenTrustStore {
	return globalThis.indexedDB ? indexedDbTrustStore() : memoryTrustStore();
}
