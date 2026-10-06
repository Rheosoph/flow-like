import Dexie, { type IndexableType, type Table } from "dexie";
import {
	type FormMemoryKey,
	MEMORY_DB_NAME,
} from "../components/interfaces/form-workbench/contracts";

/**
 * Device memory of the form workbench (PLAN §3.6, spec §5): one database, rows keyed by
 * `[scope, appId, eventId, …]` so profiles and accounts never see each other's rows. Every row carries its
 * payload as one JSON string: on desktop each stored value crosses the SQLite shim's structured-clone
 * encoder, which is far slower than JSON for nested objects. The database is born in the shim on desktop,
 * so it needs no `KNOWN_DATABASES` entry.
 */

export type FormMemoryTableName = "runs" | "presets" | "prefs";

/** One stored row of a form: its id within its table (run id, preset id, "" for prefs), when it was made, and its payload. */
export interface FormMemoryRow {
	readonly id: string;
	readonly createdAt: number;
	readonly json: string;
}

/**
 * Row storage for the memory store (`store/form-memory.ts`), which keeps every rule (JSON payloads,
 * pruning, merging, error swallowing) above this layer. Rows are always those of one form.
 */
export interface FormMemoryTables {
	/** Every row of the form in `table`, in no particular order. */
	read(
		table: FormMemoryTableName,
		key: FormMemoryKey,
	): Promise<readonly FormMemoryRow[]>;
	/** Run ids of the form, oldest first by `createdAt`, without reading payloads. */
	runIdsOldestFirst(key: FormMemoryKey): Promise<readonly string[]>;
	/** Puts rows by id; `prefs` keeps the last row given. */
	write(
		table: FormMemoryTableName,
		key: FormMemoryKey,
		rows: readonly FormMemoryRow[],
	): Promise<void>;
	/** Deletes rows by id; `prefs` deletes the form's row whatever the ids. */
	remove(
		table: FormMemoryTableName,
		key: FormMemoryKey,
		ids: readonly string[],
	): Promise<void>;
	/** Runs `work` in one read-write transaction over all tables. */
	transaction<T>(work: () => Promise<T>): Promise<T>;
}

export interface FormMemoryRunRow {
	scope: string;
	appId: string;
	eventId: string;
	id: string;
	createdAt: number;
	json: string;
}

export interface FormMemoryPresetRow {
	scope: string;
	appId: string;
	eventId: string;
	presetId: string;
	createdAt: number;
	json: string;
}

export interface FormMemoryPrefsRow {
	scope: string;
	appId: string;
	eventId: string;
	json: string;
}

type FormKey = [string, string, string];
type RowKey = [string, string, string, string];

export type FormMemoryDb = Dexie & {
	runs: Table<FormMemoryRunRow, RowKey>;
	presets: Table<FormMemoryPresetRow, RowKey>;
	prefs: Table<FormMemoryPrefsRow, FormKey>;
};

const RUNS_BY_TIME = "[scope+appId+eventId+createdAt]";

let database: FormMemoryDb | null = null;

/** The database, made on first use so pages that keep memory in the session never open it. */
export function formMemoryDb() {
	if (database) return database;
	const db = new Dexie(MEMORY_DB_NAME) as FormMemoryDb;
	db.version(1).stores({
		runs: `[scope+appId+eventId+id], ${RUNS_BY_TIME}`,
		presets: "[scope+appId+eventId+presetId]",
		prefs: "[scope+appId+eventId]",
	});
	database = db;
	return db;
}

const formKey = (key: FormMemoryKey): FormKey => [
	key.scope,
	key.appId,
	key.eventId,
];

/** All keys of one form: `[scope, appId, eventId, anything]`. */
const formRange = (key: FormMemoryKey): [IndexableType, IndexableType] => [
	[...formKey(key), Dexie.minKey],
	[...formKey(key), Dexie.maxKey],
];

async function readRows(
	db: FormMemoryDb,
	table: FormMemoryTableName,
	key: FormMemoryKey,
) {
	if (table === "prefs") {
		const row = await db.prefs.get(formKey(key));
		return row ? [{ id: "", createdAt: 0, json: row.json }] : [];
	}
	const [lower, upper] = formRange(key);
	if (table === "runs") {
		const rows = await db.runs.where(":id").between(lower, upper).toArray();
		return rows.map(({ id, createdAt, json }) => ({ id, createdAt, json }));
	}
	const rows = await db.presets.where(":id").between(lower, upper).toArray();
	return rows.map(({ presetId, createdAt, json }) => ({
		id: presetId,
		createdAt,
		json,
	}));
}

async function writeRows(
	db: FormMemoryDb,
	table: FormMemoryTableName,
	key: FormMemoryKey,
	rows: readonly FormMemoryRow[],
) {
	const { scope, appId, eventId } = key;
	if (table === "prefs") {
		const last = rows.at(-1);
		if (last) await db.prefs.put({ scope, appId, eventId, json: last.json });
		return;
	}
	if (table === "runs") {
		await db.runs.bulkPut(
			rows.map(({ id, createdAt, json }) => ({
				scope,
				appId,
				eventId,
				id,
				createdAt,
				json,
			})),
		);
		return;
	}
	await db.presets.bulkPut(
		rows.map(({ id, createdAt, json }) => ({
			scope,
			appId,
			eventId,
			presetId: id,
			createdAt,
			json,
		})),
	);
}

async function removeRows(
	db: FormMemoryDb,
	table: FormMemoryTableName,
	key: FormMemoryKey,
	ids: readonly string[],
) {
	if (table === "prefs") {
		await db.prefs.delete(formKey(key));
		return;
	}
	const keys = ids.map((id): RowKey => [...formKey(key), id]);
	await (table === "runs" ? db.runs : db.presets).bulkDelete(keys);
}

/** The Dexie tables behind the device memory store. `open` is called per operation, so a failing open rejects that call only. */
export function dexieFormMemoryTables(open: () => FormMemoryDb = formMemoryDb) {
	const tables: FormMemoryTables = {
		read: (table, key) => readRows(open(), table, key),
		async runIdsOldestFirst(key) {
			const [lower, upper] = formRange(key);
			const keys = await open()
				.runs.where(RUNS_BY_TIME)
				.between(lower, upper)
				.primaryKeys();
			return keys.map((primary) => primary[3]);
		},
		write: (table, key, rows) => writeRows(open(), table, key, rows),
		remove: (table, key, ids) => removeRows(open(), table, key, ids),
		transaction(work) {
			const db = open();
			return db.transaction("rw", [db.runs, db.presets, db.prefs], work);
		},
	};
	return tables;
}
