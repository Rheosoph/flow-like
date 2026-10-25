/**
 * Approximate on-disk bytes of a stored value. Strings count one byte per code unit: stored
 * JSON, base64 and ids are ASCII, which browsers (and the desktop SQLite shim) keep one byte
 * per character.
 */
export function estimateValueBytes(
	value: unknown,
	seen = new WeakSet<object>(),
): number {
	if (value === null || value === undefined) return 0;
	if (typeof value === "string") return value.length;
	if (typeof value === "number" || typeof value === "bigint") return 8;
	if (typeof value === "boolean") return 4;
	if (typeof Blob !== "undefined" && value instanceof Blob) return value.size;
	if (value instanceof ArrayBuffer) return value.byteLength;
	if (ArrayBuffer.isView(value)) return value.byteLength;
	if (value instanceof Date) return 8;
	if (typeof value !== "object" || seen.has(value)) return 0;
	seen.add(value);
	if (Array.isArray(value)) {
		return value.reduce(
			(total, entry) => total + estimateValueBytes(entry, seen),
			0,
		);
	}
	return Object.entries(value as Record<string, unknown>).reduce(
		(total, [key, entry]) =>
			total + key.length + estimateValueBytes(entry, seen),
		0,
	);
}

export interface ObjectStoreMeasurement {
	bytes: number;
	records: number;
}

export interface DatabaseMeasurement {
	bytes: number;
	records: number;
	stores: Record<string, ObjectStoreMeasurement>;
}

export interface IndexedDatabaseInfo {
	name: string;
	version?: number;
}

function scanObjectStore(store: IDBObjectStore) {
	return new Promise<ObjectStoreMeasurement>((resolve, reject) => {
		let bytes = 0;
		let records = 0;
		const request = store.openCursor();
		request.onsuccess = () => {
			const cursor = request.result;
			if (!cursor) {
				resolve({ bytes, records });
				return;
			}
			records += 1;
			bytes +=
				estimateValueBytes(cursor.key) + estimateValueBytes(cursor.value);
			cursor.continue();
		};
		request.onerror = () => reject(request.error);
	});
}

export class MissingDatabaseError extends Error {}

/**
 * Open an existing database at its current version. Never creates one: an upgrade request means
 * the database is missing, so it is aborted. The connection steps aside for any other tab that
 * wants to upgrade.
 */
function openExistingDatabase(name: string) {
	return new Promise<IDBDatabase>((resolve, reject) => {
		const request = indexedDB.open(name);
		request.onsuccess = () => {
			const database = request.result;
			database.onversionchange = () => database.close();
			resolve(database);
		};
		request.onerror = () => reject(request.error);
		request.onblocked = () =>
			reject(new Error(`IndexedDB ${name} is being upgraded in another tab`));
		request.onupgradeneeded = () => {
			request.transaction?.abort();
			reject(new MissingDatabaseError(`IndexedDB ${name} does not exist`));
		};
	});
}

export async function scanIndexedDatabase(
	name: string,
): Promise<DatabaseMeasurement> {
	const database = await openExistingDatabase(name);
	try {
		const storeNames = Array.from(database.objectStoreNames);
		if (storeNames.length === 0) return { bytes: 0, records: 0, stores: {} };
		const transaction = database.transaction(storeNames, "readonly");
		const results = await Promise.all(
			storeNames.map((store) =>
				scanObjectStore(transaction.objectStore(store)),
			),
		);
		const stores: Record<string, ObjectStoreMeasurement> = {};
		let bytes = 0;
		let records = 0;
		storeNames.forEach((store, index) => {
			stores[store] = results[index];
			bytes += results[index].bytes;
			records += results[index].records;
		});
		return { bytes, records, stores };
	} finally {
		database.close();
	}
}

/** Empty the given object stores (all of them when omitted) without dropping the schema. */
export async function clearIndexedDatabaseStores(
	name: string,
	stores?: readonly string[],
): Promise<void> {
	const database = await openExistingDatabase(name);
	try {
		const existing = Array.from(database.objectStoreNames);
		const targets = stores
			? stores.filter((store) => existing.includes(store))
			: existing;
		if (targets.length === 0) return;
		await new Promise<void>((resolve, reject) => {
			const transaction = database.transaction(targets, "readwrite");
			for (const store of targets) transaction.objectStore(store).clear();
			transaction.oncomplete = () => resolve();
			transaction.onerror = () => reject(transaction.error);
			transaction.onabort = () =>
				reject(transaction.error ?? new Error(`Clearing ${name} was aborted`));
		});
	} finally {
		database.close();
	}
}

export async function listIndexedDatabases(): Promise<
	IndexedDatabaseInfo[] | undefined
> {
	if (typeof indexedDB === "undefined") return [];
	const factory = indexedDB as IDBFactory & {
		databases?: () => Promise<IDBDatabaseInfo[]>;
	};
	if (!factory.databases) return undefined;
	const databases = await factory.databases().catch(() => []);
	return databases.flatMap((info) =>
		info.name ? [{ name: info.name, version: info.version }] : [],
	);
}

export interface LocalStorageMeasurement {
	bytes: number;
	keys: number;
}

/** localStorage quotas count UTF-16 code units, so each character is two bytes here. */
export function measureLocalStorage(): LocalStorageMeasurement {
	let bytes = 0;
	let keys = 0;
	try {
		const storage = globalThis.localStorage;
		if (!storage) return { bytes, keys };
		for (let index = 0; index < storage.length; index += 1) {
			const key = storage.key(index);
			if (key === null) continue;
			keys += 1;
			bytes += (key.length + (storage.getItem(key)?.length ?? 0)) * 2;
		}
	} catch {
		// localStorage can be unavailable in hardened browsers.
	}
	return { bytes, keys };
}

export interface OriginStorageEstimate {
	usage?: number;
	quota?: number;
	usageDetails?: Record<string, number>;
	persisted?: boolean;
}

export async function estimateOriginStorage(): Promise<OriginStorageEstimate> {
	const storage = globalThis.navigator?.storage;
	if (!storage) return {};
	const [estimate, persisted] = await Promise.all([
		storage.estimate?.().catch(() => undefined),
		storage.persisted?.().catch(() => undefined),
	]);
	return {
		usage: estimate?.usage,
		quota: estimate?.quota,
		usageDetails: (estimate as { usageDetails?: Record<string, number> })
			?.usageDetails,
		persisted,
	};
}
