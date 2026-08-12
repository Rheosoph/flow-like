import {
	getNativeIDBKeyRange,
	getNativeIndexedDB,
	installSqliteIndexedDB,
	isSqliteIndexedDBInstalled,
	migrateIndexedDBToSqlite,
} from "@flow-like/dexie-tauri-blob-offload/idb-sqlite";

/**
 * Module side effect, on purpose: this file must be the FIRST import of the
 * client bundle (see providers.tsx) so `window.indexedDB` is already the
 * SQLite-backed shim when Dexie and idb-keyval capture their globals.
 */
installSqliteIndexedDB();

const MASTER_FLAG = "__fl_idb_sqlite_migrated__";

/** Fallback for webviews without `indexedDB.databases()`. */
const KNOWN_DATABASES = [
	"Apps",
	"BoardSyncLineage",
	"Chat-History",
	"FlowPilotHistory",
	"Global-Chat-History",
	"LanceDBExplorerSettings",
	"Notifications",
	"OAuthTokens",
	"OfflineSync",
	"RuntimeVariables",
	"Temporary-Files-DB",
	"UI-State-DB",
	"Viewport-DB",
	"flow-like-element-values",
	"flow-like-global-state",
	"flow-like-page-state",
	"flow-like-routes",
	"keyval-store",
	"undo-redo",
];

export function needsSqliteIdbMigration(): boolean {
	if (!isSqliteIndexedDBInstalled()) return false;
	try {
		return localStorage.getItem(MASTER_FLAG) !== "1";
	} catch {
		return false;
	}
}

let migrationPromise: Promise<void> | null = null;

/**
 * Copy all native IndexedDB databases into the SQLite-backed shim, then
 * delete the native copies so the webview's IndexedDB stops growing.
 * Resolves after `timeoutMs` at the latest — a slow migration keeps running
 * in the background and only sets the completion flag when it truly finished
 * (per-database markers make re-runs add-only and idempotent).
 */
export function runSqliteIdbMigration(timeoutMs = 45_000): Promise<void> {
	migrationPromise ??= (async () => {
		const nativeIndexedDB = getNativeIndexedDB();
		const nativeIDBKeyRange = getNativeIDBKeyRange();
		if (!nativeIndexedDB || !nativeIDBKeyRange) return;

		const migration = migrateIndexedDBToSqlite({
			nativeIndexedDB,
			nativeIDBKeyRange,
			knownDatabaseNames: KNOWN_DATABASES,
		})
			.then((result) => {
				if (result.failed.length === 0) {
					try {
						localStorage.setItem(MASTER_FLAG, "1");
					} catch {
						/* re-runs are idempotent */
					}
				}
				console.info(
					`[idb-migrate] done: ${result.migrated.length} migrated, ${result.skipped.length} skipped, ${result.failed.length} failed`,
					result.failed.length > 0 ? result.failed : "",
				);
			})
			.catch((e) => {
				console.error("[idb-migrate] migration crashed:", e);
			});

		let timer: ReturnType<typeof setTimeout> | undefined;
		await Promise.race([
			migration,
			new Promise<void>((resolve) => {
				timer = setTimeout(resolve, timeoutMs);
			}),
		]);
		clearTimeout(timer);
	})();
	return migrationPromise;
}
