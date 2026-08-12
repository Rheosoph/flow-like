/// <reference path="./vendor-shims.d.ts" />
import "./self-polyfill";
import { invoke } from "@tauri-apps/api/core";
import * as indexedDBShimModule from "indexeddbshim/dist/indexeddbshim-noninvasive.js";
import customOpenDatabase from "websql-configurable/custom/index.js";

type SetGlobalVars = (
	win: unknown,
	initialConfig?: Record<string, unknown>,
) => unknown;

/**
 * The prebuilt shim is a UMD file inside a `"type": "module"` package.
 * Interpreted as ESM (bundlers, Node) it assigns `globalThis.setGlobalVars`;
 * interpreted as CJS it exports the function. Accept every shape.
 */
function resolveSetGlobalVars(): SetGlobalVars | undefined {
	const mod = indexedDBShimModule as { default?: unknown };
	if (typeof mod.default === "function") return mod.default as SetGlobalVars;
	if (typeof indexedDBShimModule === "function") {
		return indexedDBShimModule as unknown as SetGlobalVars;
	}
	const fromGlobal = (globalThis as { setGlobalVars?: unknown }).setGlobalVars;
	if (typeof fromGlobal === "function") return fromGlobal as SetGlobalVars;
	return undefined;
}

export {
	migrateIndexedDBToSqlite,
	type IdbMigrationOptions,
	type IdbMigrationResult,
} from "./idb-migrate";

const PLUGIN_PREFIX = "plugin:flow-like-dexie-blob-offload|";

interface SqlQuery {
	sql: string;
	args: unknown[];
}

interface RawSqlResult {
	error?: string;
	insert_id?: number;
	rows_affected: number;
	rows: Array<Record<string, unknown>>;
}

interface DriverResult {
	error?: Error;
	insertId?: number;
	rowsAffected: number;
	rows: Array<Record<string, unknown>>;
}

function toDriverResult(raw: RawSqlResult): DriverResult {
	if (raw.error) {
		return { error: new Error(raw.error), rowsAffected: 0, rows: [] };
	}
	return {
		insertId: raw.insert_id,
		rowsAffected: raw.rows_affected ?? 0,
		rows: raw.rows ?? [],
	};
}

function toError(e: unknown): Error {
	return e instanceof Error ? e : new Error(String(e));
}

/**
 * node-websql compatible driver executing statements against the Tauri
 * SQLite plugin. Each driver instance owns one native connection so that
 * `BEGIN`/`COMMIT` issued across separate exec batches observe the same
 * transaction state. Calls are chained to preserve statement ordering.
 */
export class TauriSQLiteDatabase {
	private readonly name: string;
	private connId: Promise<number> | undefined;
	private chain: Promise<void> = Promise.resolve();

	/** Shape expected by indexeddbshim's database cleanup (`._db._db.close`). */
	readonly _db = {
		close: (callback?: (err?: Error) => void) => {
			this.closeConnection().then(
				() => callback?.(),
				(e) => callback?.(toError(e)),
			);
		},
	};

	constructor(name: string) {
		this.name = name;
	}

	private getConnection(): Promise<number> {
		this.connId ??= invoke<number>(`${PLUGIN_PREFIX}sql_open`, {
			name: this.name,
		});
		return this.connId;
	}

	exec(
		queries: SqlQuery[],
		readOnly: boolean,
		callback: (err?: Error | null, results?: DriverResult[]) => void,
	): void {
		const task = async () => {
			let results: DriverResult[];
			try {
				const connId = await this.getConnection();
				const raw = await invoke<RawSqlResult[]>(`${PLUGIN_PREFIX}sql_exec`, {
					connId,
					queries: queries.map((q) => ({
						sql: q.sql,
						args: Array.isArray(q.args) ? q.args : [],
					})),
					readOnly,
				});
				results = raw.map(toDriverResult);
			} catch (e) {
				callback(toError(e));
				return;
			}
			try {
				callback(null, results);
			} catch (e) {
				console.error("[idb-sqlite] exec callback threw:", e);
			}
		};
		this.chain = this.chain.then(task, task);
	}

	private async closeConnection(): Promise<void> {
		const pending = this.connId;
		if (!pending) return;
		this.connId = undefined;
		const connId = await pending;
		await invoke(`${PLUGIN_PREFIX}sql_close`, { connId });
	}
}

let installed = false;
let nativeIndexedDB: IDBFactory | undefined;
let nativeIDBKeyRange: typeof IDBKeyRange | undefined;

function isTauri(): boolean {
	return (
		typeof window !== "undefined" &&
		"__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
	);
}

/**
 * Build the indexeddbshim-based IndexedDB implementation on `target`
 * (usually `window`, a plain object in tests). Returns the shimmed factory
 * and key range class, or undefined when the shim could not be created.
 */
export function createSqliteIndexedDBShim(target: object):
	| {
			indexedDB: IDBFactory;
			IDBKeyRange: typeof IDBKeyRange;
	  }
	| undefined {
	const setGlobalVars = resolveSetGlobalVars();
	if (!setGlobalVars) {
		console.error("[idb-sqlite] indexeddbshim did not expose setGlobalVars");
		return undefined;
	}

	const openDatabase = customOpenDatabase(TauriSQLiteDatabase);
	setGlobalVars(target, {
		checkOrigin: false,
		win: { openDatabase },
		deleteDatabaseFiles: false,
		useSQLiteIndexes: true,
	});

	const host = target as {
		shimIndexedDB?: { __useShim: () => void };
		indexedDB?: IDBFactory;
		IDBKeyRange?: typeof IDBKeyRange;
	};
	host.shimIndexedDB?.__useShim();
	if (!host.indexedDB || !host.IDBKeyRange) return undefined;
	return { indexedDB: host.indexedDB, IDBKeyRange: host.IDBKeyRange };
}

/**
 * Replace `window.indexedDB` (and the IDB* classes) with the indexeddbshim
 * implementation backed by native SQLite through the Tauri plugin.
 *
 * MUST run before any module that touches IndexedDB evaluates (Dexie and
 * idb-keyval both capture globals at import time). No-op outside a Tauri
 * webview so the shared UI keeps using the browser's IndexedDB.
 *
 * @returns true when the global shim is active
 */
export function installSqliteIndexedDB(): boolean {
	if (installed) return true;
	if (!isTauri()) return false;

	const previousIndexedDB = window.indexedDB;
	nativeIndexedDB = previousIndexedDB;
	nativeIDBKeyRange = window.IDBKeyRange;

	const shim = createSqliteIndexedDBShim(window);
	installed =
		shim !== undefined &&
		window.indexedDB !== undefined &&
		window.indexedDB !== previousIndexedDB;
	if (!installed) {
		console.error(
			"[idb-sqlite] Failed to replace window.indexedDB with the SQLite-backed shim",
		);
	}
	return installed;
}

export function isSqliteIndexedDBInstalled(): boolean {
	return installed;
}

/** The webview's original IndexedDB factory, captured before the shim replaced it. */
export function getNativeIndexedDB(): IDBFactory | undefined {
	return nativeIndexedDB;
}

/** The webview's original IDBKeyRange, needed to build ranges for native cursors. */
export function getNativeIDBKeyRange(): typeof IDBKeyRange | undefined {
	return nativeIDBKeyRange;
}

interface DexieLike {
	dependencies: {
		indexedDB: IDBFactory;
		IDBKeyRange: typeof IDBKeyRange;
	};
}

/**
 * Point an already-evaluated Dexie class at the shimmed globals. Dexie
 * captures `indexedDB`/`IDBKeyRange` at module-evaluation time, so this is
 * the safety net for any import-order slip. No-op when the shim is not
 * installed.
 */
export function patchDexieDependencies(dexie: DexieLike): void {
	if (!installed || typeof window === "undefined") return;
	dexie.dependencies.indexedDB = window.indexedDB;
	dexie.dependencies.IDBKeyRange = window.IDBKeyRange;
}
