import {
	deleteBoardSnapshots,
	summarizeBoardSnapshots,
} from "../../db/board-last-seen-db";
import { summarizeBoardHistories } from "../flow-history-db";
import { clearConversationHistory } from "../idb-cleanup";
import {
	type DatabaseMeasurement,
	MissingDatabaseError,
	type OriginStorageEstimate,
	clearIndexedDatabaseStores,
	estimateOriginStorage,
	listIndexedDatabases,
	measureLocalStorage,
	scanIndexedDatabase,
} from "./measure";

export type BrowserStorageCategory =
	| "boards"
	| "activity"
	| "caches"
	| "local"
	| "protected"
	| "other";

export const BROWSER_STORAGE_CATEGORIES: readonly BrowserStorageCategory[] = [
	"boards",
	"activity",
	"caches",
	"local",
	"protected",
	"other",
];

export type BrowserStoreId =
	| "chats"
	| "globalChat"
	| "flowpilot"
	| "formRuns"
	| "discardedSync"
	| "notifications"
	| "queryCache"
	| "pageSurfaces"
	| "viewports"
	| "temporaryFiles"
	| "nodeSuggestions"
	| "keyval"
	| "cacheStorage"
	| "runtimeVariables"
	| "pageState"
	| "formPresets"
	| "appShortcuts"
	| "lanceSettings"
	| "preferences"
	| "oauth"
	| "deviceVault"
	| "pendingSync";

interface DatabasePart {
	readonly name: string;
	/** Object stores that belong to this entry; all of them when omitted. */
	readonly stores?: readonly string[];
}

interface StoreDefinition {
	readonly id: BrowserStoreId;
	readonly category: BrowserStorageCategory;
	readonly databases: readonly DatabasePart[];
	/** Absent: shown, never deleted from here. `"stores"`: empty the listed object stores. */
	readonly clear?: "stores" | (() => Promise<unknown>);
}

const FORM_MEMORY_DB = "flow-like-form-memory";
const BOARD_HISTORY_DB = "undo-redo";
const BOARD_SNAPSHOT_DB = "Board-Last-Seen";

const STORE_DEFINITIONS: readonly StoreDefinition[] = [
	{
		id: "chats",
		category: "activity",
		databases: [{ name: "Chat-History" }],
		clear: () => clearConversationHistory("chats"),
	},
	{
		id: "globalChat",
		category: "activity",
		databases: [{ name: "Global-Chat-History" }],
		clear: () => clearConversationHistory("globalChat"),
	},
	{
		id: "flowpilot",
		category: "activity",
		databases: [{ name: "FlowPilotHistory" }],
		clear: () => clearConversationHistory("flowpilot"),
	},
	{
		id: "formRuns",
		category: "activity",
		databases: [{ name: FORM_MEMORY_DB, stores: ["runs"] }],
		clear: "stores",
	},
	{
		id: "discardedSync",
		category: "activity",
		databases: [{ name: "OfflineSync", stores: ["discarded"] }],
		clear: "stores",
	},
	{
		id: "notifications",
		category: "activity",
		databases: [{ name: "Notifications" }],
		clear: "stores",
	},
	{
		id: "queryCache",
		category: "caches",
		databases: [{ name: "flow-like-query-cache" }],
		clear: "stores",
	},
	{
		id: "pageSurfaces",
		category: "caches",
		databases: [{ name: "flow-like-page-surface-cache" }],
		clear: "stores",
	},
	{
		id: "viewports",
		category: "caches",
		databases: [{ name: "Viewport-DB" }],
		clear: "stores",
	},
	{
		id: "temporaryFiles",
		category: "caches",
		databases: [{ name: "Temporary-Files-DB" }],
		clear: "stores",
	},
	{
		id: "nodeSuggestions",
		category: "caches",
		databases: [{ name: "NodeSuggestions" }],
		clear: "stores",
	},
	{
		id: "keyval",
		category: "caches",
		databases: [{ name: "keyval-store" }],
		clear: "stores",
	},
	{
		id: "runtimeVariables",
		category: "local",
		databases: [{ name: "RuntimeVariables" }],
		clear: "stores",
	},
	{
		id: "pageState",
		category: "local",
		databases: [
			{ name: "UI-State-DB" },
			{ name: "flow-like-page-state" },
			{ name: "flow-like-global-state" },
			{ name: "flow-like-element-values" },
			{ name: "flow-like-routes" },
		],
		clear: "stores",
	},
	{
		id: "formPresets",
		category: "local",
		databases: [{ name: FORM_MEMORY_DB, stores: ["presets", "prefs"] }],
		clear: "stores",
	},
	{
		id: "lanceSettings",
		category: "local",
		databases: [{ name: "LanceDBExplorerSettings" }],
		clear: "stores",
	},
	{
		id: "appShortcuts",
		category: "local",
		databases: [{ name: "Apps" }],
	},
	{
		id: "oauth",
		category: "protected",
		databases: [{ name: "OAuthTokens" }],
	},
	{
		id: "deviceVault",
		category: "protected",
		databases: [{ name: "flow-like-device-management" }],
	},
	{
		id: "pendingSync",
		category: "protected",
		databases: [{ name: "OfflineSync", stores: ["commands"] }],
	},
];

const DEFINITIONS_BY_ID = new Map(
	STORE_DEFINITIONS.map((definition) => [definition.id, definition]),
);

const KNOWN_DATABASES = new Set([
	BOARD_HISTORY_DB,
	BOARD_SNAPSHOT_DB,
	...STORE_DEFINITIONS.flatMap((definition) =>
		definition.databases.map((part) => part.name),
	),
]);

export interface BoardTraces {
	/** Undefined when an undo-history key could not be split into app and board. */
	readonly appId?: string;
	readonly boardId: string;
	readonly historyKeys: readonly string[];
	readonly snapshotIds: readonly string[];
	readonly undoSteps: number;
	readonly undoBytes: number;
	readonly snapshotBytes: number;
}

export interface BrowserStorageEntry {
	readonly id: string;
	readonly category: BrowserStorageCategory;
	readonly sizeBytes: number;
	readonly records: number;
	readonly updatedAtMs?: number;
	readonly deletable: boolean;
	readonly store?: BrowserStoreId;
	readonly board?: BoardTraces;
	/** Set for databases this inventory does not know, and for the unattributed remainder. */
	readonly database?: string;
	readonly unattributed?: boolean;
}

export interface BrowserStorageScan {
	readonly entries: readonly BrowserStorageEntry[];
	readonly estimate: OriginStorageEstimate;
	readonly measuredBytes: number;
	readonly scannedAt: number;
}

export interface ScanProgress {
	readonly done: number;
	readonly total: number;
}

function sumParts(
	part: DatabasePart,
	measurement: DatabaseMeasurement | undefined,
): { bytes: number; records: number } {
	if (!measurement) return { bytes: 0, records: 0 };
	if (!part.stores) return measurement;
	return part.stores.reduce(
		(total, store) => ({
			bytes: total.bytes + (measurement.stores[store]?.bytes ?? 0),
			records: total.records + (measurement.stores[store]?.records ?? 0),
		}),
		{ bytes: 0, records: 0 },
	);
}

/** `boardHistoryKey` joins with `_`; ids never contain one, so the first `_` splits them. */
function splitHistoryKey(key: string): { appId?: string; boardId: string } {
	const separator = key.indexOf("_");
	if (separator <= 0) return { boardId: key };
	return {
		appId: key.slice(0, separator),
		boardId: key.slice(separator + 1),
	};
}

interface BoardAccumulator {
	appId?: string;
	boardId: string;
	historyKeys: string[];
	snapshotIds: string[];
	undoSteps: number;
	undoBytes: number;
	snapshotBytes: number;
	updatedAtMs?: number;
}

async function scanBoardTraces(
	existing: ReadonlySet<string> | undefined,
): Promise<BrowserStorageEntry[]> {
	const [histories, snapshots] = await Promise.all([
		existing && !existing.has(BOARD_HISTORY_DB)
			? []
			: summarizeBoardHistories().catch(() => []),
		existing && !existing.has(BOARD_SNAPSHOT_DB)
			? []
			: summarizeBoardSnapshots().catch(() => []),
	]);
	const boards = new Map<string, BoardAccumulator>();
	const boardFor = (appId: string | undefined, boardId: string) => {
		const key = `${appId ?? ""}\u001f${boardId}`;
		let board = boards.get(key);
		if (!board) {
			board = {
				appId,
				boardId,
				historyKeys: [],
				snapshotIds: [],
				undoSteps: 0,
				undoBytes: 0,
				snapshotBytes: 0,
			};
			boards.set(key, board);
		}
		return board;
	};
	const touch = (board: BoardAccumulator, at: number | undefined) => {
		if (at && at > (board.updatedAtMs ?? 0)) board.updatedAtMs = at;
	};
	for (const history of histories) {
		const { appId, boardId } = splitHistoryKey(history.key);
		const board = boardFor(appId, boardId);
		board.historyKeys.push(history.key);
		board.undoSteps += history.entries;
		board.undoBytes += history.bytes;
		touch(board, history.updatedAt);
	}
	for (const snapshot of snapshots) {
		const board = boardFor(snapshot.appId, snapshot.boardId);
		board.snapshotIds.push(snapshot.id);
		board.snapshotBytes += snapshot.bytes;
		touch(board, snapshot.updatedAt);
	}
	return [...boards.values()].map(({ updatedAtMs, ...traces }) => ({
		id: `board:${traces.appId ?? ""}:${traces.boardId}`,
		category: "boards",
		sizeBytes: traces.undoBytes + traces.snapshotBytes,
		records: traces.undoSteps + traces.snapshotIds.length,
		updatedAtMs,
		deletable: true,
		board: traces,
	}));
}

async function measureCacheStorage(): Promise<BrowserStorageEntry | undefined> {
	if (typeof caches === "undefined") return undefined;
	const names = await caches.keys().catch(() => []);
	if (names.length === 0) return undefined;
	return {
		id: "cacheStorage",
		category: "caches",
		store: "cacheStorage",
		sizeBytes: 0,
		records: names.length,
		deletable: true,
	};
}

/**
 * Measure everything this origin keeps in the browser. Databases are read one at a time with
 * cursors, so memory stays at one record; sizes are estimates of the stored bytes.
 */
export async function scanBrowserStorage(options?: {
	signal?: AbortSignal;
	onProgress?: (progress: ScanProgress) => void;
}): Promise<BrowserStorageScan> {
	const [estimate, databases] = await Promise.all([
		estimateOriginStorage(),
		listIndexedDatabases(),
	]);
	const existing = databases
		? new Set(databases.map((info) => info.name))
		: undefined;
	const toMeasure = existing
		? [...existing].filter(
				(name) => name !== BOARD_HISTORY_DB && name !== BOARD_SNAPSHOT_DB,
			)
		: [...KNOWN_DATABASES].filter(
				(name) => name !== BOARD_HISTORY_DB && name !== BOARD_SNAPSHOT_DB,
			);
	const total = toMeasure.length + 1;
	const measurements = new Map<string, DatabaseMeasurement>();
	let done = 0;
	options?.onProgress?.({ done, total });
	for (const name of toMeasure) {
		options?.signal?.throwIfAborted();
		const measurement = await scanIndexedDatabase(name).catch(() => undefined);
		if (measurement) measurements.set(name, measurement);
		done += 1;
		options?.onProgress?.({ done, total });
	}
	options?.signal?.throwIfAborted();
	const boardEntries = await scanBoardTraces(existing);
	options?.onProgress?.({ done: total, total });

	const entries: BrowserStorageEntry[] = [...boardEntries];
	for (const definition of STORE_DEFINITIONS) {
		const present = definition.databases.filter((part) =>
			measurements.has(part.name),
		);
		if (present.length === 0) continue;
		const size = present.reduce(
			(sum, part) => {
				const measured = sumParts(part, measurements.get(part.name));
				return {
					bytes: sum.bytes + measured.bytes,
					records: sum.records + measured.records,
				};
			},
			{ bytes: 0, records: 0 },
		);
		if (size.records === 0) continue;
		entries.push({
			id: definition.id,
			category: definition.category,
			store: definition.id,
			sizeBytes: size.bytes,
			records: size.records,
			deletable: definition.clear !== undefined,
		});
	}
	for (const [name, measurement] of measurements) {
		if (KNOWN_DATABASES.has(name)) continue;
		entries.push({
			id: `db:${name}`,
			category: "other",
			database: name,
			sizeBytes: measurement.bytes,
			records: measurement.records,
			deletable: false,
		});
	}
	const localStorageSize = measureLocalStorage();
	if (localStorageSize.keys > 0) {
		entries.push({
			id: "preferences",
			category: "local",
			store: "preferences",
			sizeBytes: localStorageSize.bytes,
			records: localStorageSize.keys,
			deletable: false,
		});
	}
	const cacheStorage = await measureCacheStorage();
	if (cacheStorage) {
		entries.push({
			...cacheStorage,
			sizeBytes: estimate.usageDetails?.caches ?? 0,
		});
	}

	const measuredBytes = entries.reduce(
		(total, entry) => total + entry.sizeBytes,
		0,
	);
	const remainder = (estimate.usage ?? 0) - measuredBytes;
	if (remainder > 0) {
		entries.push({
			id: "unattributed",
			category: "other",
			sizeBytes: remainder,
			records: 0,
			deletable: false,
			unattributed: true,
		});
	}
	entries.sort((a, b) => b.sizeBytes - a.sizeBytes);
	return { entries, estimate, measuredBytes, scannedAt: Date.now() };
}

export interface DeleteEntriesOptions {
	/** Clears one board's undo history, including a copy resident in this tab. */
	readonly clearBoardHistory: (key: string) => Promise<void>;
}

export interface DeleteEntriesResult {
	readonly deleted: number;
	readonly failed: readonly { id: string; error: unknown }[];
}

async function deleteEntry(
	entry: BrowserStorageEntry,
	options: DeleteEntriesOptions,
): Promise<void> {
	if (!entry.deletable) throw new Error(`${entry.id} is not deletable`);
	if (entry.board) {
		for (const key of entry.board.historyKeys) {
			await options.clearBoardHistory(key);
		}
		await deleteBoardSnapshots(entry.board.snapshotIds);
		return;
	}
	if (entry.store === "cacheStorage") {
		const names = await caches.keys();
		await Promise.all(names.map((name) => caches.delete(name)));
		return;
	}
	const definition = entry.store
		? DEFINITIONS_BY_ID.get(entry.store)
		: undefined;
	if (!definition?.clear) throw new Error(`${entry.id} has no cleanup routine`);
	if (definition.clear === "stores") {
		for (const part of definition.databases) {
			await clearIndexedDatabaseStores(part.name, part.stores).catch(
				(error: unknown) => {
					if (!(error instanceof MissingDatabaseError)) throw error;
				},
			);
		}
		return;
	}
	await definition.clear();
}

export async function deleteBrowserStorageEntries(
	entries: readonly BrowserStorageEntry[],
	options: DeleteEntriesOptions,
): Promise<DeleteEntriesResult> {
	let deleted = 0;
	const failed: { id: string; error: unknown }[] = [];
	for (const entry of entries) {
		try {
			await deleteEntry(entry, options);
			deleted += 1;
		} catch (error) {
			failed.push({ id: entry.id, error });
		}
	}
	return { deleted, failed };
}
