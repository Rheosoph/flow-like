import Dexie, { type Table } from "dexie";
import { SUGGESTION_FORMAT_VERSION } from "./types";

/**
 * On-device cache of the recommender: per-board facts, per-app sync days and trained models, all
 * scoped by user `sub`. Desktop IndexedDB is a SQLite shim whose structured-clone encoder is far
 * slower than `JSON.stringify`, so every large value is a JSON string, and the board index is kept
 * apart from the facts so listing boards (every sync, every fingerprint check) never reads them.
 */

export type ModelKind = "ngram" | "neural";

export interface StoredBoard {
	appId: string;
	boardId: string;
	/** `updatedAt` of the board summary the facts were extracted for, in ms; `0` when unknown. */
	updatedAt: number;
	transitions: number;
}

export interface StoredModel {
	fingerprint: string;
	trainedAt: number;
	/** `model.serialize()`. */
	payload: string;
	boards: number;
	transitions: number;
}

export interface SuggestionStore {
	readonly persistent: boolean;
	listBoards(sub: string, appId?: string): Promise<StoredBoard[]>;
	/** Facts JSON of `boards`, in order; boards whose facts are gone are skipped. */
	readFacts(sub: string, boards: readonly StoredBoard[]): Promise<string[]>;
	putBoard(sub: string, board: StoredBoard, facts: string): Promise<void>;
	deleteBoards(
		sub: string,
		boards: readonly Pick<StoredBoard, "appId" | "boardId">[],
	): Promise<void>;
	/** appId → local calendar day (`YYYY-MM-DD`) of its last completed sync. */
	listApps(sub: string): Promise<Map<string, string>>;
	markAppSynced(sub: string, appId: string, day: string): Promise<void>;
	/** Drops the apps' sync rows together with their boards and facts. */
	deleteApps(sub: string, appIds: readonly string[]): Promise<void>;
	getModel(sub: string, kind: ModelKind): Promise<StoredModel | undefined>;
	putModel(sub: string, kind: ModelKind, model: StoredModel): Promise<void>;
	/** Day of the last sync run that left nothing for later. */
	getSyncDay(sub: string): Promise<string | undefined>;
	setSyncDay(sub: string, day: string): Promise<void>;
}

interface Row {
	id: string;
	sub: string;
}

interface BoardRow extends Row {
	appId: string;
	boardId: string;
	updatedAt: number;
	format: number;
	transitions: number;
}

interface FactsRow extends Row {
	facts: string;
}

interface AppRow extends Row {
	appId: string;
	syncedDay: string;
	syncedAt: number;
	format: number;
}

interface ModelRow extends Row, StoredModel {
	kind: ModelKind;
	format: number;
}

interface MetaRow extends Row {
	syncedDay: string;
	syncedAt: number;
	format: number;
}

export interface RowTable<T extends Row> {
	get(id: string): Promise<T | undefined>;
	bulkGet(ids: string[]): Promise<(T | undefined)[]>;
	/** Rows of `sub`, narrowed to one app when the table is indexed by `appId`. */
	list(sub: string, appId?: string): Promise<T[]>;
	put(row: T): Promise<void>;
	bulkDelete(ids: string[]): Promise<void>;
}

export interface SuggestionTables {
	boards: RowTable<BoardRow>;
	facts: RowTable<FactsRow>;
	apps: RowTable<AppRow>;
	models: RowTable<ModelRow>;
	meta: RowTable<MetaRow>;
}

const SEPARATOR = "\u001f";

const key = (...parts: string[]) => parts.join(SEPARATOR);

const toStoredBoard = (row: BoardRow): StoredBoard => ({
	appId: row.appId,
	boardId: row.boardId,
	updatedAt: row.updatedAt,
	transitions: row.transitions,
});

export class TableSuggestionStore implements SuggestionStore {
	constructor(
		private readonly tables: SuggestionTables,
		readonly persistent: boolean,
	) {}

	async listBoards(sub: string, appId?: string): Promise<StoredBoard[]> {
		const rows = await this.tables.boards.list(sub, appId);
		const stale = rows.filter(
			(row) => row.format !== SUGGESTION_FORMAT_VERSION,
		);
		if (stale.length > 0) {
			const ids = stale.map((row) => row.id);
			await Promise.all([
				this.tables.boards.bulkDelete(ids),
				this.tables.facts.bulkDelete(ids),
			]);
		}
		return rows
			.filter((row) => row.format === SUGGESTION_FORMAT_VERSION)
			.map(toStoredBoard);
	}

	async readFacts(
		sub: string,
		boards: readonly StoredBoard[],
	): Promise<string[]> {
		const rows = await this.tables.facts.bulkGet(
			boards.map((board) => key(sub, board.appId, board.boardId)),
		);
		return rows.flatMap((row) => (row ? [row.facts] : []));
	}

	async putBoard(
		sub: string,
		board: StoredBoard,
		facts: string,
	): Promise<void> {
		const id = key(sub, board.appId, board.boardId);
		await this.tables.facts.put({ id, sub, facts });
		await this.tables.boards.put({
			id,
			sub,
			...board,
			format: SUGGESTION_FORMAT_VERSION,
		});
	}

	async deleteBoards(
		sub: string,
		boards: readonly Pick<StoredBoard, "appId" | "boardId">[],
	): Promise<void> {
		if (boards.length === 0) return;
		const ids = boards.map((board) => key(sub, board.appId, board.boardId));
		await this.tables.boards.bulkDelete(ids);
		await this.tables.facts.bulkDelete(ids);
	}

	async listApps(sub: string): Promise<Map<string, string>> {
		const rows = await this.tables.apps.list(sub);
		return new Map(
			rows
				.filter((row) => row.format === SUGGESTION_FORMAT_VERSION)
				.map((row) => [row.appId, row.syncedDay]),
		);
	}

	async markAppSynced(sub: string, appId: string, day: string): Promise<void> {
		await this.tables.apps.put({
			id: key(sub, appId),
			sub,
			appId,
			syncedDay: day,
			syncedAt: Date.now(),
			format: SUGGESTION_FORMAT_VERSION,
		});
	}

	async deleteApps(sub: string, appIds: readonly string[]): Promise<void> {
		for (const appId of appIds) {
			const boards = await this.tables.boards.list(sub, appId);
			await this.deleteBoards(sub, boards);
		}
		await this.tables.apps.bulkDelete(appIds.map((appId) => key(sub, appId)));
	}

	async getModel(
		sub: string,
		kind: ModelKind,
	): Promise<StoredModel | undefined> {
		const row = await this.tables.models.get(key(sub, kind));
		if (!row) return undefined;
		if (row.format !== SUGGESTION_FORMAT_VERSION) {
			await this.tables.models.bulkDelete([row.id]);
			return undefined;
		}
		return {
			fingerprint: row.fingerprint,
			trainedAt: row.trainedAt,
			payload: row.payload,
			boards: row.boards,
			transitions: row.transitions,
		};
	}

	async putModel(
		sub: string,
		kind: ModelKind,
		model: StoredModel,
	): Promise<void> {
		await this.tables.models.put({
			id: key(sub, kind),
			sub,
			kind,
			format: SUGGESTION_FORMAT_VERSION,
			...model,
		});
	}

	async getSyncDay(sub: string): Promise<string | undefined> {
		const row = await this.tables.meta.get(sub);
		return row?.format === SUGGESTION_FORMAT_VERSION
			? row.syncedDay
			: undefined;
	}

	async setSyncDay(sub: string, day: string): Promise<void> {
		await this.tables.meta.put({
			id: sub,
			sub,
			syncedDay: day,
			syncedAt: Date.now(),
			format: SUGGESTION_FORMAT_VERSION,
		});
	}
}

class MemoryTable<T extends Row & { appId?: string }> implements RowTable<T> {
	private readonly rows = new Map<string, T>();

	async get(id: string) {
		return this.rows.get(id);
	}

	async bulkGet(ids: string[]) {
		return ids.map((id) => this.rows.get(id));
	}

	async list(sub: string, appId?: string) {
		return [...this.rows.values()].filter(
			(row) => row.sub === sub && (appId === undefined || row.appId === appId),
		);
	}

	async put(row: T) {
		this.rows.set(row.id, row);
	}

	async bulkDelete(ids: string[]) {
		for (const id of ids) this.rows.delete(id);
	}
}

export const createMemoryTables = (): SuggestionTables => ({
	boards: new MemoryTable<BoardRow>(),
	facts: new MemoryTable<FactsRow>(),
	apps: new MemoryTable<AppRow>(),
	models: new MemoryTable<ModelRow>(),
	meta: new MemoryTable<MetaRow>(),
});

export const createMemorySuggestionStore = (): SuggestionStore =>
	new TableSuggestionStore(createMemoryTables(), false);

class NodeSuggestionsDB extends Dexie {
	boards!: Table<BoardRow, string>;
	facts!: Table<FactsRow, string>;
	apps!: Table<AppRow, string>;
	models!: Table<ModelRow, string>;
	meta!: Table<MetaRow, string>;

	constructor() {
		super("NodeSuggestions");
		this.version(1).stores({
			boards: "id, sub, [sub+appId]",
			facts: "id",
			apps: "id, sub",
			models: "id",
			meta: "id",
		});
	}
}

function dexieTable<T extends Row>(
	table: Table<T, string>,
	byApp: boolean,
): RowTable<T> {
	return {
		get: (id) => table.get(id),
		bulkGet: (ids) => table.bulkGet(ids),
		list: (sub, appId) =>
			byApp && appId !== undefined
				? table.where("[sub+appId]").equals([sub, appId]).toArray()
				: table.where("sub").equals(sub).toArray(),
		put: async (row) => {
			await table.put(row);
		},
		bulkDelete: (ids) => table.bulkDelete(ids),
	};
}

let opening: Promise<SuggestionStore> | undefined;

async function openDexieStore(): Promise<SuggestionStore> {
	if (typeof indexedDB === "undefined") return createMemorySuggestionStore();
	try {
		const db = new NodeSuggestionsDB();
		await db.open();
		return new TableSuggestionStore(
			{
				boards: dexieTable(db.boards, true),
				facts: dexieTable(db.facts, false),
				apps: dexieTable(db.apps, false),
				models: dexieTable(db.models, false),
				meta: dexieTable(db.meta, false),
			},
			true,
		);
	} catch (error) {
		console.warn(
			"[node-suggestions] IndexedDB unavailable, keeping suggestions in memory:",
			error,
		);
		return createMemorySuggestionStore();
	}
}

/** One Dexie connection per page: every open is another SQLite connection on desktop. */
export function openSuggestionStore(): Promise<SuggestionStore> {
	opening ??= openDexieStore();
	return opening;
}
