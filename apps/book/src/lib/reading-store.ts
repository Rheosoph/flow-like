import {
	type ReadingBookmark,
	type ReadingComment,
	type ReadingProgressRecord,
	mergePersistedReadingProgress,
} from "./reading-progress";

const DATABASE_NAME = "flowbook-reader";
// Version 2 repairs browsers that created the progress-only v1 schema before
// bookmark and comment stores were introduced.
const DATABASE_VERSION = 2;

const STORE_PROGRESS = "progress";
const STORE_BOOKMARKS = "bookmarks";
const STORE_COMMENTS = "comments";

type ReadingStoreName =
	| typeof STORE_PROGRESS
	| typeof STORE_BOOKMARKS
	| typeof STORE_COMMENTS;

export interface ReadingData {
	progress: ReadingProgressRecord[];
	bookmarks: ReadingBookmark[];
	comments: ReadingComment[];
}

let databasePromise: Promise<IDBDatabase> | undefined;

function openReadingDatabase(): Promise<IDBDatabase> {
	if (typeof indexedDB === "undefined") {
		return Promise.reject(new Error("IndexedDB is unavailable"));
	}

	databasePromise ??= new Promise<IDBDatabase>((resolve, reject) => {
		const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
		let rejectedAsBlocked = false;

		request.onupgradeneeded = () => {
			const database = request.result;

			if (!database.objectStoreNames.contains(STORE_PROGRESS)) {
				const store = database.createObjectStore(STORE_PROGRESS, {
					keyPath: "id",
				});
				store.createIndex("editionId", "editionId", { unique: false });
				store.createIndex("updatedAt", "updatedAt", { unique: false });
			}

			if (!database.objectStoreNames.contains(STORE_BOOKMARKS)) {
				const store = database.createObjectStore(STORE_BOOKMARKS, {
					keyPath: "id",
				});
				store.createIndex("editionId", "editionId", { unique: false });
				store.createIndex("path", "path", { unique: false });
				store.createIndex("createdAt", "createdAt", { unique: false });
			}

			if (!database.objectStoreNames.contains(STORE_COMMENTS)) {
				const store = database.createObjectStore(STORE_COMMENTS, {
					keyPath: "id",
				});
				store.createIndex("editionId", "editionId", { unique: false });
				store.createIndex("path", "path", { unique: false });
				store.createIndex("createdAt", "createdAt", { unique: false });
			}
		};

		request.onsuccess = () => {
			const database = request.result;
			if (rejectedAsBlocked) {
				database.close();
				return;
			}
			database.onversionchange = () => {
				database.close();
				databasePromise = undefined;
			};
			resolve(database);
		};
		request.onerror = () =>
			reject(request.error ?? new Error("Could not open IndexedDB"));
		request.onblocked = () => {
			rejectedAsBlocked = true;
			reject(new Error("IndexedDB upgrade was blocked"));
		};
	}).catch((error) => {
		databasePromise = undefined;
		throw error;
	});

	return databasePromise;
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(request.error ?? new Error("IndexedDB request failed"));
	});
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
	return new Promise((resolve, reject) => {
		transaction.oncomplete = () => resolve();
		transaction.onerror = () =>
			reject(transaction.error ?? new Error("IndexedDB transaction failed"));
		transaction.onabort = () =>
			reject(
				transaction.error ?? new Error("IndexedDB transaction was aborted"),
			);
	});
}

async function getEditionRecords<T>(
	storeName: ReadingStoreName,
	editionId: string,
): Promise<T[]> {
	const database = await openReadingDatabase();
	const transaction = database.transaction(storeName, "readonly");
	const completed = transactionDone(transaction);
	const [records] = await Promise.all([
		requestResult<T[]>(
			transaction.objectStore(storeName).index("editionId").getAll(editionId),
		),
		completed,
	]);
	return records;
}

async function putRecord<T>(
	storeName: ReadingStoreName,
	record: T,
): Promise<void> {
	const database = await openReadingDatabase();
	const transaction = database.transaction(storeName, "readwrite");
	const completed = transactionDone(transaction);
	await Promise.all([
		requestResult(transaction.objectStore(storeName).put(record)),
		completed,
	]);
}

async function deleteRecord(
	storeName: ReadingStoreName,
	id: string,
): Promise<void> {
	const database = await openReadingDatabase();
	const transaction = database.transaction(storeName, "readwrite");
	const completed = transactionDone(transaction);
	await Promise.all([
		requestResult(transaction.objectStore(storeName).delete(id)),
		completed,
	]);
}

function mergeAndPutProgress(
	store: IDBObjectStore,
	incoming: ReadingProgressRecord,
): Promise<ReadingProgressRecord> {
	return new Promise((resolve, reject) => {
		const getRequest = store.get(incoming.id);
		getRequest.onerror = () =>
			reject(getRequest.error ?? new Error("Could not read saved progress"));
		getRequest.onsuccess = () => {
			const merged = mergePersistedReadingProgress(
				getRequest.result as ReadingProgressRecord | undefined,
				incoming,
			);
			const putRequest = store.put(merged);
			putRequest.onerror = () =>
				reject(
					putRequest.error ?? new Error("Could not save reading progress"),
				);
			putRequest.onsuccess = () => resolve(merged);
		};
	});
}

export async function getReadingData(editionId: string): Promise<ReadingData> {
	const [progress, bookmarks, comments] = await Promise.all([
		getEditionRecords<ReadingProgressRecord>(STORE_PROGRESS, editionId),
		getEditionRecords<ReadingBookmark>(STORE_BOOKMARKS, editionId),
		getEditionRecords<ReadingComment>(STORE_COMMENTS, editionId),
	]);

	return {
		progress: progress.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
		bookmarks: bookmarks.sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
		comments: comments.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
	};
}

export async function saveReadingProgress(
	progress: ReadingProgressRecord,
): Promise<ReadingProgressRecord> {
	const database = await openReadingDatabase();
	const transaction = database.transaction(STORE_PROGRESS, "readwrite");
	const completed = transactionDone(transaction);
	const [merged] = await Promise.all([
		mergeAndPutProgress(transaction.objectStore(STORE_PROGRESS), progress),
		completed,
	]);
	return merged;
}

export async function saveReadingBookmark(
	bookmark: ReadingBookmark,
): Promise<void> {
	await putRecord(STORE_BOOKMARKS, bookmark);
}

export async function deleteReadingBookmark(id: string): Promise<void> {
	await deleteRecord(STORE_BOOKMARKS, id);
}

export async function saveReadingComment(
	comment: ReadingComment,
): Promise<void> {
	await putRecord(STORE_COMMENTS, comment);
}

export async function deleteReadingComment(id: string): Promise<void> {
	await deleteRecord(STORE_COMMENTS, id);
}
