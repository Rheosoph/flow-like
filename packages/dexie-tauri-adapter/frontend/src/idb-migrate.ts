/**
 * One-time migration of the webview's native IndexedDB databases into the
 * SQLite-backed shim. Copies records in batches (never holding a native
 * transaction across await points), uses add-only semantics so a retry can
 * never overwrite data written to SQLite after a partial failure, and
 * deletes each native database once its copy is complete so the webview's
 * IndexedDB shrinks back to nothing.
 */

const MARKER_PREFIX = "__fl_idb_sqlite_migrated__:";

export interface IdbMigrationOptions {
	/** The webview's original factory (captured before the shim was installed). */
	nativeIndexedDB: IDBFactory;
	/** The original IDBKeyRange, required to page cursors on the native side. */
	nativeIDBKeyRange: typeof IDBKeyRange;
	/** Target factory; defaults to the (shimmed) global `indexedDB`. */
	targetIndexedDB?: IDBFactory;
	/** Used when the native factory has no `databases()` (older webviews). */
	knownDatabaseNames?: string[];
	batchSize?: number;
	/** Delete each native database after a successful copy. Default true. */
	deleteNativeAfterCopy?: boolean;
	onProgress?: (info: {
		database: string;
		store?: string;
		copied?: number;
	}) => void;
}

export interface IdbMigrationResult {
	migrated: string[];
	skipped: string[];
	failed: Array<{ name: string; error: string }>;
}

interface StoreSchema {
	name: string;
	keyPath: string | string[] | null;
	autoIncrement: boolean;
	indexes: Array<{
		name: string;
		keyPath: string | string[];
		unique: boolean;
		multiEntry: boolean;
	}>;
}

interface MigrationRecord {
	key: IDBValidKey;
	value: unknown;
}

function markerKey(name: string): string {
	return `${MARKER_PREFIX}${name}`;
}

function isMarked(name: string): boolean {
	try {
		return localStorage.getItem(markerKey(name)) === "1";
	} catch {
		return false;
	}
}

function mark(name: string): void {
	try {
		localStorage.setItem(markerKey(name), "1");
	} catch {
		/* localStorage unavailable — worst case the migration re-runs (add-only, safe) */
	}
}

function requestToPromise<T>(request: IDBRequest<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(request.error ?? new Error("IDB request failed"));
	});
}

function openDatabase(
	factory: IDBFactory,
	name: string,
	version?: number,
	onUpgrade?: (db: IDBDatabase, tx: IDBTransaction | null) => void,
): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const request =
			version === undefined ? factory.open(name) : factory.open(name, version);
		request.onupgradeneeded = () => {
			try {
				onUpgrade?.(request.result, request.transaction);
			} catch (e) {
				reject(e);
			}
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(request.error ?? new Error(`Failed to open IndexedDB ${name}`));
		request.onblocked = () =>
			reject(new Error(`Opening IndexedDB ${name} was blocked`));
	});
}

function readSchema(db: IDBDatabase): StoreSchema[] {
	const storeNames = Array.from(db.objectStoreNames);
	if (storeNames.length === 0) return [];
	const tx = db.transaction(storeNames, "readonly");
	const schemas = storeNames.map((storeName) => {
		const store = tx.objectStore(storeName);
		return {
			name: storeName,
			keyPath: store.keyPath,
			autoIncrement: store.autoIncrement,
			indexes: Array.from(store.indexNames).map((indexName) => {
				const index = store.index(indexName);
				return {
					name: indexName,
					keyPath: index.keyPath,
					unique: index.unique,
					multiEntry: index.multiEntry,
				};
			}),
		};
	});
	tx.abort();
	return schemas;
}

function createStoreFromSchema(db: IDBDatabase, schema: StoreSchema): void {
	const store = db.createObjectStore(schema.name, {
		keyPath: schema.keyPath ?? undefined,
		autoIncrement: schema.autoIncrement,
	});
	for (const index of schema.indexes) {
		try {
			store.createIndex(index.name, index.keyPath, {
				unique: index.unique,
				multiEntry: index.multiEntry,
			});
		} catch (e) {
			console.warn(
				`[idb-migrate] Could not recreate index ${schema.name}.${index.name}:`,
				e,
			);
		}
	}
}

/** Open the target db, adding any object stores present natively but missing there. */
async function openTargetWithStores(
	factory: IDBFactory,
	name: string,
	schemas: StoreSchema[],
): Promise<IDBDatabase> {
	let db = await openDatabase(factory, name, undefined, (freshDb) => {
		for (const schema of schemas) createStoreFromSchema(freshDb, schema);
	});
	const missing = schemas.filter((s) => !db.objectStoreNames.contains(s.name));
	if (missing.length === 0) return db;
	const nextVersion = db.version + 1;
	db.close();
	db = await openDatabase(factory, name, nextVersion, (upgradeDb) => {
		for (const schema of missing) {
			if (!upgradeDb.objectStoreNames.contains(schema.name)) {
				createStoreFromSchema(upgradeDb, schema);
			}
		}
	});
	return db;
}

function readBatch(
	db: IDBDatabase,
	storeName: string,
	keyRangeCtor: typeof IDBKeyRange,
	afterKey: IDBValidKey | undefined,
	limit: number,
): Promise<MigrationRecord[]> {
	return new Promise((resolve, reject) => {
		const records: MigrationRecord[] = [];
		const tx = db.transaction(storeName, "readonly");
		const range =
			afterKey === undefined
				? undefined
				: keyRangeCtor.lowerBound(afterKey, true);
		const request = tx.objectStore(storeName).openCursor(range);
		request.onsuccess = () => {
			const cursor = request.result;
			if (!cursor || records.length >= limit) {
				resolve(records);
				return;
			}
			records.push({ key: cursor.primaryKey, value: cursor.value });
			cursor.continue();
		};
		request.onerror = () =>
			reject(request.error ?? new Error(`Cursor failed on ${storeName}`));
	});
}

function writeBatchAdd(
	db: IDBDatabase,
	storeName: string,
	inlineKeys: boolean,
	records: MigrationRecord[],
): Promise<void> {
	return new Promise((resolve, reject) => {
		const tx = db.transaction(storeName, "readwrite");
		tx.oncomplete = () => resolve();
		tx.onabort = () =>
			reject(tx.error ?? new Error(`Write aborted on ${storeName}`));
		const store = tx.objectStore(storeName);
		for (const record of records) {
			const request = inlineKeys
				? store.add(record.value)
				: store.add(record.value, record.key);
			request.onerror = (event) => {
				// Key already exists in SQLite (retry after a partial run):
				// keep the newer record, swallow the conflict.
				if (request.error?.name === "ConstraintError") {
					event.preventDefault();
					event.stopPropagation();
				}
			};
		}
	});
}

/** Slow path: per-record existence check, used when a batched add aborts. */
async function writeBatchChecked(
	db: IDBDatabase,
	storeName: string,
	inlineKeys: boolean,
	records: MigrationRecord[],
): Promise<void> {
	for (const record of records) {
		const readTx = db.transaction(storeName, "readonly");
		const existing = await requestToPromise(
			readTx.objectStore(storeName).getKey(record.key),
		);
		if (existing !== undefined) continue;
		await writeBatchAdd(db, storeName, inlineKeys, [record]);
	}
}

async function copyStore(
	nativeDb: IDBDatabase,
	targetDb: IDBDatabase,
	schema: StoreSchema,
	keyRangeCtor: typeof IDBKeyRange,
	batchSize: number,
	onProgress?: IdbMigrationOptions["onProgress"],
): Promise<number> {
	if (!targetDb.objectStoreNames.contains(schema.name)) {
		throw new Error(
			`Target database ${targetDb.name} is missing object store ${schema.name}`,
		);
	}
	const inlineKeys = schema.keyPath !== null;
	let copied = 0;
	let afterKey: IDBValidKey | undefined;
	while (true) {
		const batch = await readBatch(
			nativeDb,
			schema.name,
			keyRangeCtor,
			afterKey,
			batchSize,
		);
		if (batch.length === 0) break;
		try {
			await writeBatchAdd(targetDb, schema.name, inlineKeys, batch);
		} catch (e) {
			console.warn(
				`[idb-migrate] Batched write failed on ${nativeDb.name}.${schema.name}, retrying record-by-record:`,
				e,
			);
			await writeBatchChecked(targetDb, schema.name, inlineKeys, batch);
		}
		copied += batch.length;
		afterKey = batch[batch.length - 1].key;
		onProgress?.({ database: nativeDb.name, store: schema.name, copied });
	}
	return copied;
}

function deleteNativeDatabase(
	factory: IDBFactory,
	name: string,
): Promise<void> {
	return new Promise((resolve) => {
		let settled = false;
		const finish = () => {
			if (!settled) {
				settled = true;
				resolve();
			}
		};
		const request = factory.deleteDatabase(name);
		request.onsuccess = finish;
		request.onerror = () => {
			console.warn(
				`[idb-migrate] Failed to delete native db ${name}:`,
				request.error,
			);
			finish();
		};
		request.onblocked = () => {
			console.warn(
				`[idb-migrate] Deleting native db ${name} is blocked; it will be removed once connections close`,
			);
			finish();
		};
	});
}

async function listNativeDatabases(
	factory: IDBFactory,
	knownNames: string[],
): Promise<string[]> {
	if (typeof factory.databases === "function") {
		const databases = await factory.databases.call(factory);
		return databases
			.map((db) => db.name)
			.filter((name): name is string => typeof name === "string");
	}
	return knownNames;
}

async function migrateDatabase(
	name: string,
	options: IdbMigrationOptions,
	target: IDBFactory,
): Promise<void> {
	const nativeDb = await openDatabase(options.nativeIndexedDB, name);
	try {
		const schemas = readSchema(nativeDb);
		if (schemas.length === 0) return;

		const targetDb = await openTargetWithStores(target, name, schemas);
		try {
			for (const schema of schemas) {
				await copyStore(
					nativeDb,
					targetDb,
					schema,
					options.nativeIDBKeyRange,
					options.batchSize ?? 200,
					options.onProgress,
				);
			}
		} finally {
			targetDb.close();
		}
	} finally {
		nativeDb.close();
	}
}

export async function migrateIndexedDBToSqlite(
	options: IdbMigrationOptions,
): Promise<IdbMigrationResult> {
	const target = options.targetIndexedDB ?? indexedDB;
	const deleteNative = options.deleteNativeAfterCopy !== false;
	const result: IdbMigrationResult = { migrated: [], skipped: [], failed: [] };

	const names = await listNativeDatabases(
		options.nativeIndexedDB,
		options.knownDatabaseNames ?? [],
	);

	for (const name of names) {
		if (isMarked(name)) {
			result.skipped.push(name);
			if (deleteNative) {
				await deleteNativeDatabase(options.nativeIndexedDB, name);
			}
			continue;
		}
		try {
			options.onProgress?.({ database: name });
			await migrateDatabase(name, options, target);
			mark(name);
			result.migrated.push(name);
			if (deleteNative) {
				await deleteNativeDatabase(options.nativeIndexedDB, name);
			}
		} catch (e) {
			const message = e instanceof Error ? e.message : String(e);
			console.error(`[idb-migrate] Migration failed for ${name}:`, e);
			result.failed.push({ name, error: message });
		}
	}

	return result;
}
