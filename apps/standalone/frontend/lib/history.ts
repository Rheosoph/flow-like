import { chatDb } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import { uiStateDb } from "@flow-like/flow-like-ui/db/ui-state-db";

/** Where the shared UI keeps viewer data, for browsers that cannot list their databases. */
const KNOWN_DATABASES = [chatDb.name, uiStateDb.name];

function settle<T>(request: IDBRequest<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		request.onsuccess = () => resolve(request.result);
		request.onerror = () => reject(request.error);
	});
}

async function clearDatabase(idb: IDBFactory, name: string): Promise<void> {
	const db = await settle(idb.open(name));
	try {
		const stores = Array.from(db.objectStoreNames);
		if (stores.length === 0) return;
		const transaction = db.transaction(stores, "readwrite");
		for (const store of stores) transaction.objectStore(store).clear();
		await new Promise<void>((resolve, reject) => {
			transaction.oncomplete = () => resolve();
			transaction.onabort = () =>
				reject(
					transaction.error ??
						new Error(`Clearing the ${name} database was aborted`),
				);
		});
	} finally {
		db.close();
	}
}

/**
 * The service origin belongs to one placement and its token is shared, so locking empties every
 * IndexedDB store on it: transcripts and attachments, Page and global state, input values and
 * cached surfaces.
 */
export async function clearServiceHistory(
	idb: IDBFactory = indexedDB,
): Promise<void> {
	const listed =
		typeof idb.databases === "function" ? await idb.databases() : undefined;
	const names = listed
		? listed.flatMap(({ name }) => (name ? [name] : []))
		: KNOWN_DATABASES;
	await Promise.all(names.map((name) => clearDatabase(idb, name)));
	if (!listed)
		throw new Error("This browser cannot list the Page data it stores.");
}
