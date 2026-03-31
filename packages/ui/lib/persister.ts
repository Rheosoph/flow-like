import type {
	PersistedClient,
	Persister,
} from "@tanstack/react-query-persist-client";
import { del, get, set } from "idb-keyval";

/**
 * Creates an Indexed DB persister with write throttling.
 * Writes are debounced so rapid cache mutations don't hammer IDB.
 */
export function createIDBPersister(
	idbValidKey: IDBValidKey = "reactQuery",
	throttleMs = 2000,
) {
	let pendingWrite: ReturnType<typeof setTimeout> | null = null;
	let latestClient: PersistedClient | null = null;

	const flush = async () => {
		if (latestClient) {
			const toWrite = latestClient;
			latestClient = null;
			await set(idbValidKey, toWrite);
		}
	};

	return {
		persistClient: async (client: PersistedClient) => {
			latestClient = client;
			if (pendingWrite !== null) return;
			pendingWrite = setTimeout(async () => {
				pendingWrite = null;
				await flush();
			}, throttleMs);
		},
		restoreClient: async () => {
			return await get<PersistedClient>(idbValidKey);
		},
		removeClient: async () => {
			if (pendingWrite !== null) {
				clearTimeout(pendingWrite);
				pendingWrite = null;
			}
			latestClient = null;
			await del(idbValidKey);
		},
	} as Persister;
}
