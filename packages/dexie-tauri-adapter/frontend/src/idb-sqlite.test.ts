import { beforeEach, describe, expect, it } from "bun:test";
import Dexie, { type Table } from "dexie";
import { invokeCalls, invokeResults } from "./test-utils/invoke-mock";

const invokeHandlers = invokeResults;

const { TauriSQLiteDatabase, createSqliteIndexedDBShim } = await import(
	"./idb-sqlite"
);

const PREFIX = "plugin:flow-like-dexie-blob-offload|";

function execAsync(
	db: InstanceType<typeof TauriSQLiteDatabase>,
	queries: Array<{ sql: string; args: unknown[] }>,
	readOnly = false,
): Promise<{ err?: Error | null; results?: unknown[] }> {
	return new Promise((resolve) => {
		db.exec(queries, readOnly, (err, results) => resolve({ err, results }));
	});
}

describe("TauriSQLiteDatabase driver", () => {
	beforeEach(() => {
		invokeCalls.length = 0;
		invokeHandlers.clear();
		invokeHandlers.set(`${PREFIX}sql_open`, () => 7);
		invokeHandlers.set(`${PREFIX}sql_close`, () => undefined);
	});

	it("opens lazily once and reuses the connection", async () => {
		invokeHandlers.set(`${PREFIX}sql_exec`, () => []);
		const db = new TauriSQLiteDatabase("Test.sqlite");
		expect(invokeCalls.filter((c) => c.cmd.endsWith("sql_open"))).toHaveLength(
			0,
		);

		await execAsync(db, [{ sql: "SELECT 1", args: [] }]);
		await execAsync(db, [{ sql: "SELECT 2", args: [] }]);

		const opens = invokeCalls.filter((c) => c.cmd.endsWith("sql_open"));
		expect(opens).toHaveLength(1);
		expect(opens[0].args).toEqual({ name: "Test.sqlite" });
		const execs = invokeCalls.filter((c) => c.cmd.endsWith("sql_exec"));
		expect(execs).toHaveLength(2);
		expect(execs[0].args?.connId).toBe(7);
	});

	it("maps raw results to websql driver results", async () => {
		invokeHandlers.set(`${PREFIX}sql_exec`, () => [
			{ rows_affected: 1, insert_id: 42, rows: [] },
			{ rows_affected: 0, rows: [{ v: "hello" }] },
			{
				error: "could not prepare statement (boom)",
				rows_affected: 0,
				rows: [],
			},
		]);
		const db = new TauriSQLiteDatabase("Test.sqlite");
		const { err, results } = await execAsync(db, [
			{ sql: "INSERT ...", args: [] },
			{ sql: "SELECT v", args: [] },
			{ sql: "BOGUS", args: [] },
		]);

		expect(err).toBeNull();
		const [insert, select, failed] = results as Array<{
			error?: Error;
			insertId?: number;
			rowsAffected: number;
			rows: unknown[];
		}>;
		expect(insert.insertId).toBe(42);
		expect(insert.rowsAffected).toBe(1);
		expect(select.rows).toEqual([{ v: "hello" }]);
		expect(failed.error).toBeInstanceOf(Error);
		expect(failed.error?.message).toContain("boom");
	});

	it("reports fatal invoke failures via the error callback and keeps working", async () => {
		let fail = true;
		invokeHandlers.set(`${PREFIX}sql_exec`, () => {
			if (fail) throw new Error("ipc down");
			return [];
		});
		const db = new TauriSQLiteDatabase("Test.sqlite");

		const first = await execAsync(db, [{ sql: "SELECT 1", args: [] }]);
		expect(first.err?.message).toContain("ipc down");

		fail = false;
		const second = await execAsync(db, [{ sql: "SELECT 1", args: [] }]);
		expect(second.err).toBeNull();
	});

	it("preserves exec ordering", async () => {
		const seen: string[] = [];
		invokeHandlers.set(`${PREFIX}sql_exec`, (args) => {
			const queries = args?.queries as Array<{ sql: string }>;
			seen.push(queries[0].sql);
			return [];
		});
		const db = new TauriSQLiteDatabase("Test.sqlite");
		await Promise.all([
			execAsync(db, [{ sql: "one", args: [] }]),
			execAsync(db, [{ sql: "two", args: [] }]),
			execAsync(db, [{ sql: "three", args: [] }]),
		]);
		expect(seen).toEqual(["one", "two", "three"]);
	});

	it("closes the native connection through the inner _db handle", async () => {
		invokeHandlers.set(`${PREFIX}sql_exec`, () => []);
		const db = new TauriSQLiteDatabase("Test.sqlite");
		await execAsync(db, [{ sql: "SELECT 1", args: [] }]);

		await new Promise<void>((resolve, reject) => {
			db._db.close((err) => (err ? reject(err) : resolve()));
		});
		const closes = invokeCalls.filter((c) => c.cmd.endsWith("sql_close"));
		expect(closes).toHaveLength(1);
		expect(closes[0].args).toEqual({ connId: 7 });
	});
});

/**
 * Everything below runs REAL IndexedDB operations through indexeddbshim +
 * websql + TauriSQLiteDatabase, with bun:sqlite standing in for the Rust
 * plugin (mirroring its exec semantics: per-statement errors, insert_id,
 * rows_affected, shared files across connections). The Rust side asserts
 * the same exec contract in its own unit tests.
 */
const { Database } = require("bun:sqlite") as {
	Database: new (
		path: string,
	) => {
		prepare: (sql: string) => {
			columnNames: string[];
			all: (...args: unknown[]) => Array<Record<string, unknown>>;
			run: (...args: unknown[]) => {
				changes: number;
				lastInsertRowid: number | bigint;
			};
		};
		close: () => void;
	};
};
const { mkdtempSync } = require("node:fs") as typeof import("node:fs");
const { tmpdir } = require("node:os") as typeof import("node:os");
const { join } = require("node:path") as typeof import("node:path");

const baseDir = mkdtempSync(join(tmpdir(), "idb-sqlite-test-"));
const connections = new Map<number, InstanceType<typeof Database>>();
let nextConnId = 1;

function installSqlBackend() {
	invokeHandlers.set(`${PREFIX}sql_open`, (args) => {
		const name = String(args?.name);
		const db = new Database(join(baseDir, name));
		const id = nextConnId++;
		connections.set(id, db);
		return id;
	});
	invokeHandlers.set(`${PREFIX}sql_exec`, (args) => {
		const db = connections.get(args?.connId as number);
		if (!db) throw new Error(`Unknown connection ${args?.connId}`);
		const queries = args?.queries as Array<{ sql: string; args: unknown[] }>;
		return queries.map((query) => {
			try {
				const stmt = db.prepare(query.sql);
				const params = (query.args ?? []).map((value) => value ?? null);
				if (stmt.columnNames.length > 0) {
					return { rows_affected: 0, rows: stmt.all(...params) };
				}
				const info = stmt.run(...params);
				return {
					insert_id: Number(info.lastInsertRowid),
					rows_affected: info.changes,
					rows: [],
				};
			} catch (e) {
				return {
					error: e instanceof Error ? e.message : String(e),
					rows_affected: 0,
					rows: [],
				};
			}
		});
	});
	invokeHandlers.set(`${PREFIX}sql_close`, (args) => {
		const db = connections.get(args?.connId as number);
		db?.close();
		connections.delete(args?.connId as number);
		return undefined;
	});
}

let sharedShim:
	| { indexedDB: IDBFactory; IDBKeyRange: typeof IDBKeyRange }
	| undefined;

function getShim() {
	sharedShim ??= createSqliteIndexedDBShim({});
	if (!sharedShim) throw new Error("Failed to create SQLite IndexedDB shim");
	return sharedShim;
}

function makeDexie(name: string): Dexie {
	const shim = getShim();
	return new Dexie(name, {
		indexedDB: shim.indexedDB,
		IDBKeyRange: shim.IDBKeyRange,
	});
}

function req<T>(request: IDBRequest<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		request.onsuccess = () => resolve(request.result);
		request.onerror = () => reject(request.error);
	});
}

function done(tx: IDBTransaction): Promise<void> {
	return new Promise((resolve, reject) => {
		tx.oncomplete = () => resolve();
		tx.onabort = () => reject(tx.error ?? new Error("aborted"));
		tx.onerror = () => {};
	});
}

function openDb(
	factory: IDBFactory,
	name: string,
	version?: number,
	upgrade?: (db: IDBDatabase, tx: IDBTransaction | null) => void,
): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const request =
			version === undefined ? factory.open(name) : factory.open(name, version);
		request.onupgradeneeded = () =>
			upgrade?.(request.result, request.transaction);
		request.onsuccess = () => resolve(request.result);
		request.onerror = () => reject(request.error);
	});
}

describe("SQLite-backed IndexedDB integration", () => {
	it("runs full IndexedDB CRUD, indexes, cursors and persistence over SQLite", async () => {
		installSqlBackend();
		const target: Record<string, unknown> = {};
		const shim = createSqliteIndexedDBShim(target);
		expect(shim).toBeDefined();
		if (!shim) return;
		const factory = shim.indexedDB;

		const db = await openDb(factory, "Integration-Test", 1, (fresh) => {
			const items = fresh.createObjectStore("items", { keyPath: "id" });
			items.createIndex("by_type", "type", { unique: false });
			fresh.createObjectStore("kv");
		});
		expect(Array.from(db.objectStoreNames).sort()).toEqual(["items", "kv"]);

		// Writes: inline keys, out-of-line keys, nested values, dates
		{
			const tx = db.transaction(["items", "kv"], "readwrite");
			const items = tx.objectStore("items");
			items.add({
				id: "a",
				type: "note",
				body: { text: "hello", tags: [1, 2] },
			});
			items.add({ id: "b", type: "note", body: { text: "world" } });
			items.add({ id: "c", type: "task", createdAt: new Date(1700000000000) });
			tx.objectStore("kv").put({ value: 42 }, "answer");
			await done(tx);
		}

		// Reads: get, getAll, index queries, count — requests must be placed
		// synchronously; the shim (per spec) deactivates transactions in
		// microtasks after an await.
		{
			const tx = db.transaction(["items", "kv"], "readonly");
			const items = tx.objectStore("items");
			const [a, all, notes, count, kv] = await Promise.all([
				req(items.get("a") as IDBRequest<{ body: { text: string } }>),
				req(items.getAll() as IDBRequest<unknown[]>),
				req(items.index("by_type").getAll("note") as IDBRequest<unknown[]>),
				req(items.count()),
				req(
					tx.objectStore("kv").get("answer") as IDBRequest<{ value: number }>,
				),
			]);
			expect(a.body.text).toBe("hello");
			expect(all).toHaveLength(3);
			expect(notes).toHaveLength(2);
			expect(count).toBe(3);
			expect(kv.value).toBe(42);
		}

		// Cursor iteration with a key range
		{
			const tx = db.transaction("items", "readonly");
			const seen: string[] = [];
			await new Promise<void>((resolve, reject) => {
				const cursorReq = tx
					.objectStore("items")
					.openCursor(shim.IDBKeyRange.lowerBound("b"));
				cursorReq.onsuccess = () => {
					const cursor = cursorReq.result;
					if (!cursor) {
						resolve();
						return;
					}
					seen.push(String(cursor.primaryKey));
					cursor.continue();
				};
				cursorReq.onerror = () => reject(cursorReq.error);
			});
			expect(seen).toEqual(["b", "c"]);
		}

		// Update + delete
		{
			const tx = db.transaction("items", "readwrite");
			tx.objectStore("items").put({
				id: "a",
				type: "note",
				body: { text: "updated" },
			});
			tx.objectStore("items").delete("b");
			await done(tx);
			const check = db.transaction("items", "readonly").objectStore("items");
			const [a, count] = await Promise.all([
				req(check.get("a") as IDBRequest<{ body: { text: string } }>),
				req(check.count()),
			]);
			expect(a.body.text).toBe("updated");
			expect(count).toBe(2);
		}

		// Date survives the typeson round-trip
		{
			const tx = db.transaction("items", "readonly");
			const c = await req(
				tx.objectStore("items").get("c") as IDBRequest<{ createdAt: Date }>,
			);
			expect(c.createdAt).toBeInstanceOf(Date);
			expect(c.createdAt.getTime()).toBe(1700000000000);
		}

		// databases() lists it
		const listed = await factory.databases();
		expect(listed.map((d) => d.name)).toContain("Integration-Test");

		// Persistence: reopen through a fresh connection
		db.close();
		const reopened = await openDb(factory, "Integration-Test");
		const persisted = await req(
			reopened.transaction("items", "readonly").objectStore("items").count(),
		);
		expect(persisted).toBe(2);
		reopened.close();
	}, 20000);
});

describe("raw IndexedDB query surface", () => {
	beforeEach(() => {
		installSqlBackend();
	});

	interface RawRow {
		id?: number;
		group: string;
		meta: { pos: number };
	}

	async function seedRawDb(name: string): Promise<IDBDatabase> {
		const factory = getShim().indexedDB;
		const db = await openDb(factory, name, 1, (fresh) => {
			const store = fresh.createObjectStore("rows", {
				keyPath: "id",
				autoIncrement: true,
			});
			store.createIndex("by_group", "group");
			store.createIndex("by_pos", "meta.pos");
		});
		const tx = db.transaction("rows", "readwrite");
		const store = tx.objectStore("rows");
		const groups = ["a", "a", "a", "b", "b", "b", "c", "c", "c"];
		groups.forEach((group, i) => {
			store.add({ group, meta: { pos: 9 - i } } satisfies RawRow);
		});
		await done(tx);
		return db;
	}

	interface CursorSnapshot {
		key: IDBValidKey;
		primaryKey: IDBValidKey;
	}

	// Cursor objects are reused as they advance, so their position must be
	// snapshotted at visit time.
	function collectCursor(
		request: IDBRequest<IDBCursor | null>,
	): Promise<CursorSnapshot[]> {
		return new Promise((resolve, reject) => {
			const seen: CursorSnapshot[] = [];
			request.onsuccess = () => {
				const cursor = request.result;
				if (!cursor) {
					resolve(seen);
					return;
				}
				seen.push({ key: cursor.key, primaryKey: cursor.primaryKey });
				cursor.continue();
			};
			request.onerror = () => reject(request.error);
		});
	}

	it("supports getAll limits, getAllKeys and exclusive key ranges", async () => {
		const db = await seedRawDb("RawRanges");
		const KeyRange = getShim().IDBKeyRange;
		try {
			const store = db.transaction("rows", "readonly").objectStore("rows");
			const [limited, keys, exclusiveKeys, onlyB] = await Promise.all([
				req(store.getAll(undefined, 4) as IDBRequest<RawRow[]>),
				req(store.getAllKeys(KeyRange.bound(2, 7))),
				req(store.getAllKeys(KeyRange.bound(2, 7, true, true))),
				req(
					store.index("by_group").getAll(KeyRange.only("b")) as IDBRequest<
						RawRow[]
					>,
				),
			]);
			expect(limited).toHaveLength(4);
			expect(keys).toEqual([2, 3, 4, 5, 6, 7]);
			expect(exclusiveKeys).toEqual([3, 4, 5, 6]);
			expect(onlyB).toHaveLength(3);
			expect(onlyB.every((r) => r.group === "b")).toBe(true);
		} finally {
			db.close();
		}
	});

	it("supports key cursors, unique and reverse directions, and advance", async () => {
		const db = await seedRawDb("RawCursors");
		try {
			{
				const store = db.transaction("rows", "readonly").objectStore("rows");
				const keyCursors = await collectCursor(store.openKeyCursor());
				expect(keyCursors.map((c) => c.primaryKey)).toEqual([
					1, 2, 3, 4, 5, 6, 7, 8, 9,
				]);
			}
			{
				const index = db
					.transaction("rows", "readonly")
					.objectStore("rows")
					.index("by_group");
				const unique = await collectCursor(
					index.openKeyCursor(null, "nextunique"),
				);
				expect(unique.map((c) => String(c.key))).toEqual(["a", "b", "c"]);
			}
			{
				const store = db.transaction("rows", "readonly").objectStore("rows");
				const reversed = await collectCursor(
					store.openCursor(
						null,
						"prev",
					) as IDBRequest<IDBCursorWithValue | null>,
				);
				expect(reversed.map((c) => c.primaryKey)).toEqual([
					9, 8, 7, 6, 5, 4, 3, 2, 1,
				]);
			}
			{
				const index = db
					.transaction("rows", "readonly")
					.objectStore("rows")
					.index("by_group");
				const uniqueReversed = await collectCursor(
					index.openKeyCursor(null, "prevunique"),
				);
				expect(uniqueReversed.map((c) => String(c.key))).toEqual([
					"c",
					"b",
					"a",
				]);
			}
			{
				const store = db.transaction("rows", "readonly").objectStore("rows");
				const afterAdvance = await new Promise<number[]>((resolve, reject) => {
					const seen: number[] = [];
					const request = store.openCursor();
					let advanced = false;
					request.onsuccess = () => {
						const cursor = request.result;
						if (!cursor) {
							resolve(seen);
							return;
						}
						if (!advanced) {
							advanced = true;
							cursor.advance(3);
							return;
						}
						seen.push(Number(cursor.primaryKey));
						cursor.continue();
					};
					request.onerror = () => reject(request.error);
				});
				expect(afterAdvance).toEqual([4, 5, 6, 7, 8, 9]);
			}
			{
				const index = db
					.transaction("rows", "readonly")
					.objectStore("rows")
					.index("by_pos");
				const ordered = await req(index.getAll() as IDBRequest<RawRow[]>);
				expect(ordered[0].meta.pos).toBe(1);
				expect(ordered[0].id).toBe(9);
			}
		} finally {
			db.close();
		}
	});

	it("supports cursor.update and cursor.delete", async () => {
		const db = await seedRawDb("RawCursorMutation");
		try {
			const tx = db.transaction("rows", "readwrite");
			await new Promise<void>((resolve, reject) => {
				const request = tx.objectStore("rows").openCursor();
				request.onsuccess = () => {
					const cursor = request.result;
					if (!cursor) {
						resolve();
						return;
					}
					const row = cursor.value as RawRow;
					if (row.id === 5) {
						cursor.update({ ...row, group: "updated" });
					} else if (row.id === 6) {
						cursor.delete();
					}
					cursor.continue();
				};
				request.onerror = () => reject(request.error);
			});
			await done(tx);

			const check = db.transaction("rows", "readonly").objectStore("rows");
			const [five, count] = await Promise.all([
				req(check.get(5) as IDBRequest<RawRow>),
				req(check.count()),
			]);
			expect(five.group).toBe("updated");
			expect(count).toBe(8);
		} finally {
			db.close();
		}
	});

	it("supports version upgrades adding stores and indexes, and deleteDatabase", async () => {
		const factory = getShim().indexedDB;
		let db = await seedRawDb("RawUpgrade");
		db.close();

		db = await openDb(factory, "RawUpgrade", 2, (upgradeDb, tx) => {
			upgradeDb.createObjectStore("extra", { keyPath: "id" });
			tx?.objectStore("rows").createIndex("by_group_pos", [
				"group",
				"meta.pos",
			]);
		});
		try {
			expect(Array.from(db.objectStoreNames).sort()).toEqual(["extra", "rows"]);
			const store = db.transaction("rows", "readonly").objectStore("rows");
			expect(Array.from(store.indexNames)).toContain("by_group_pos");
			const compound = await req(
				store.index("by_group_pos").getAll() as IDBRequest<RawRow[]>,
			);
			expect(compound).toHaveLength(9);
			expect(compound[0].group).toBe("a");
		} finally {
			db.close();
		}

		await new Promise<void>((resolve, reject) => {
			const request = factory.deleteDatabase("RawUpgrade");
			request.onsuccess = () => resolve();
			request.onerror = () => reject(request.error);
		});
		const listed = await factory.databases();
		expect(listed.map((d) => d.name)).not.toContain("RawUpgrade");
	}, 20000);
});

describe("Dexie e2e on the SQLite shim", () => {
	beforeEach(() => {
		installSqlBackend();
	});

	interface TaskRow {
		id?: number;
		title: string;
		priority: number;
		project: string;
		dueAt: Date;
		tags: string[];
		done?: boolean;
	}

	interface ProfileRow {
		email: string;
		name: { first: string; last: string };
	}

	type TasksDB = Dexie & {
		tasks: Table<TaskRow, number>;
		profiles: Table<ProfileRow, string>;
	};

	const DAY = 24 * 60 * 60 * 1000;
	const BASE = 1750000000000;

	function makeTasksDb(name: string): TasksDB {
		const db = makeDexie(name) as TasksDB;
		db.version(1).stores({
			tasks: "++id, title, priority, dueAt, *tags, [project+priority]",
			profiles: "&email, name.last",
		});
		return db;
	}

	function seedTasks(): TaskRow[] {
		const rows: TaskRow[] = [];
		for (let i = 1; i <= 20; i++) {
			rows.push({
				title: `Task ${i}`,
				priority: i % 4,
				project: i < 10 ? "alpha" : "beta",
				dueAt: new Date(BASE + i * DAY),
				tags: i % 2 === 1 ? ["urgent", "home"] : ["work"],
			});
		}
		return rows;
	}

	it("answers Dexie's full where-clause and collection API", async () => {
		const db = makeTasksDb("DexieQueries");
		await db.tasks.bulkAdd(seedTasks());

		expect(await db.tasks.count()).toBe(20);
		expect(await db.tasks.where("priority").equals(2).count()).toBe(5);
		expect(await db.tasks.where("priority").anyOf([1, 3]).count()).toBe(10);
		expect(await db.tasks.where("priority").above(1).count()).toBe(10);
		expect(await db.tasks.where("priority").notEqual(0).count()).toBe(15);
		expect(
			await db.tasks.where("priority").between(1, 3, true, false).count(),
		).toBe(10);
		expect(await db.tasks.where("title").startsWith("Task 1").count()).toBe(11);
		expect(
			await db.tasks.where("title").equalsIgnoreCase("task 3").count(),
		).toBe(1);
		expect(await db.tasks.where("tags").equals("urgent").count()).toBe(10);
		expect(
			await db.tasks.where("[project+priority]").equals(["alpha", 2]).count(),
		).toBe(2);
		expect(
			await db.tasks
				.where("dueAt")
				.below(new Date(BASE + 5 * DAY))
				.count(),
		).toBe(4);

		const window = await db.tasks
			.orderBy("dueAt")
			.reverse()
			.offset(2)
			.limit(3)
			.toArray();
		expect(window.map((t) => t.id)).toEqual([18, 17, 16]);

		expect(await db.tasks.where("priority").equals(3).primaryKeys()).toEqual([
			3, 7, 11, 15, 19,
		]);
		expect(await db.tasks.orderBy("project").uniqueKeys()).toEqual([
			"alpha",
			"beta",
		]);

		const fetched = await db.tasks.bulkGet([1, 3, 999]);
		expect(fetched[0]?.title).toBe("Task 1");
		expect(fetched[1]?.title).toBe("Task 3");
		expect(fetched[2]).toBeUndefined();

		await db.profiles.bulkAdd([
			{ email: "jo@example.com", name: { first: "Jo", last: "Doe" } },
			{ email: "max@example.com", name: { first: "Max", last: "Mustermann" } },
		]);
		const doe = await db.profiles.where("name.last").equals("Doe").first();
		expect(doe?.email).toBe("jo@example.com");

		db.close();
	}, 20000);

	it("handles modify, delete, updates and key generator continuity", async () => {
		const db = makeTasksDb("DexieMutations");
		await db.tasks.bulkAdd(seedTasks());

		const modified = await db.tasks
			.where("project")
			.equals("beta")
			.modify({ done: true });
		expect(modified).toBe(11);
		expect(await db.tasks.filter((t) => t.done === true).count()).toBe(11);

		const removed = await db.tasks.where("priority").equals(0).delete();
		expect(removed).toBe(5);
		expect(await db.tasks.count()).toBe(15);

		await db.tasks.update(1, { title: "renamed" });
		expect((await db.tasks.get(1))?.title).toBe("renamed");

		// Explicit key must bump the auto-increment generator
		await db.tasks.put({
			id: 1000,
			title: "explicit",
			priority: 1,
			project: "alpha",
			dueAt: new Date(BASE),
			tags: [],
		});
		const generated = await db.tasks.add({
			title: "generated",
			priority: 1,
			project: "alpha",
			dueAt: new Date(BASE),
			tags: [],
		});
		expect(generated).toBe(1001);

		await db.tasks.bulkDelete([1000, 1001]);
		expect(await db.tasks.count()).toBe(15);

		db.close();
	}, 20000);

	it("keeps explicit transactions alive across awaited operations", async () => {
		// Regression guard for the transaction-liveness patch on indexeddbshim:
		// sequential awaits inside one Dexie transaction (the offline sync
		// queue's write pattern) must not hit TransactionInactiveError.
		const db = makeTasksDb("DexieAwaitTx");
		const observed = await db.transaction("rw", db.tasks, async () => {
			await db.tasks.add({
				title: "first",
				priority: 1,
				project: "alpha",
				dueAt: new Date(BASE),
				tags: [],
			});
			await db.tasks.add({
				title: "second",
				priority: 2,
				project: "alpha",
				dueAt: new Date(BASE),
				tags: [],
			});
			const first = await db.tasks.where("title").equals("first").first();
			await db.tasks.update(first?.id as number, { priority: 9 });
			return db.tasks.count();
		});
		expect(observed).toBe(2);
		expect(
			(await db.tasks.where("title").equals("first").first())?.priority,
		).toBe(9);
		db.close();
	}, 20000);

	it("enforces unique indexes and rolls back failed transactions", async () => {
		const db = makeTasksDb("DexieConstraints");
		await db.profiles.add({
			email: "jo@example.com",
			name: { first: "Jo", last: "Doe" },
		});

		let constraintError: unknown;
		try {
			await db.profiles.add({
				email: "jo@example.com",
				name: { first: "Other", last: "Person" },
			});
		} catch (e) {
			constraintError = e;
		}
		expect((constraintError as Error)?.name).toBe("ConstraintError");
		expect(await db.profiles.count()).toBe(1);

		let rollbackError: unknown;
		try {
			await db.transaction("rw", db.tasks, async () => {
				await db.tasks.add({
					title: "doomed",
					priority: 1,
					project: "alpha",
					dueAt: new Date(BASE),
					tags: [],
				});
				throw new Error("boom");
			});
		} catch (e) {
			rollbackError = e;
		}
		expect((rollbackError as Error)?.message).toContain("boom");
		expect(await db.tasks.count()).toBe(0);

		db.close();
	}, 20000);

	it("preserves data across Dexie schema upgrades", async () => {
		const name = "DexieUpgrade";
		let db = makeDexie(name) as Dexie & {
			things: Table<{ id?: number; kind: string; size: number }, number>;
		};
		db.version(1).stores({ things: "++id, kind" });
		await db.things.bulkAdd([
			{ kind: "x", size: 1 },
			{ kind: "y", size: 2 },
			{ kind: "y", size: 3 },
		]);
		db.close();

		db = makeDexie(name) as Dexie & {
			things: Table<
				{ id?: number; kind: string; size: number; migrated?: boolean },
				number
			>;
		};
		db.version(1).stores({ things: "++id, kind" });
		db.version(2)
			.stores({ things: "++id, kind, size" })
			.upgrade(async (tx) => {
				await tx
					.table("things")
					.toCollection()
					.modify((thing: { migrated?: boolean }) => {
						thing.migrated = true;
					});
			});

		expect(await db.things.where("size").above(1).count()).toBe(2);
		const all = await db.things.toArray();
		expect(all).toHaveLength(3);
		expect(all.every((t) => t.migrated === true)).toBe(true);

		db.close();
	}, 20000);
});

describe("blob-offload middleware over the SQLite shim", () => {
	const storedBlobs = new Map<string, number[]>();

	function installBlobBackend() {
		invokeHandlers.set(`${PREFIX}blob_store`, (args) => {
			const data = args?.data as number[];
			const hash = `h${storedBlobs.size}`;
			storedBlobs.set(hash, data);
			return { hash, mac: `mac-${hash}` };
		});
		invokeHandlers.set(`${PREFIX}blob_get`, (args) => {
			const data = storedBlobs.get(String(args?.hash));
			if (!data) throw new Error(`missing blob ${args?.hash}`);
			return data;
		});
		invokeHandlers.set(`${PREFIX}blob_store_batch`, (args) => {
			const entries = args?.entries as Array<{ key: string; data: number[] }>;
			return entries.map((entry) => {
				const hash = `h${storedBlobs.size}`;
				storedBlobs.set(hash, entry.data);
				return { key: entry.key, blob_ref: { hash, mac: `mac-${hash}` } };
			});
		});
		invokeHandlers.set(`${PREFIX}blob_get_batch`, (args) => {
			const refs = args?.refs as Array<{
				key: string;
				blob_ref: { hash: string };
			}>;
			return refs.map((ref) => ({
				key: ref.key,
				data: storedBlobs.get(ref.blob_ref.hash) ?? [],
			}));
		});
		invokeHandlers.set(`${PREFIX}blob_inc_refs`, () => undefined);
		invokeHandlers.set(`${PREFIX}blob_dec_refs`, () => []);
	}

	it("offloads and rehydrates large values through Dexie on the shim", async () => {
		installSqlBackend();
		installBlobBackend();
		const { dexieTauriBlobOffload } = await import("./index");

		type DocsDB = Dexie & {
			docs: Table<{ id: string; content: string; small: string }, string>;
		};
		const db = makeDexie("BlobOnShim") as DocsDB;
		db.version(1).stores({ docs: "id" });
		db.use(dexieTauriBlobOffload(50));

		const big = "x".repeat(500);
		await db.docs.put({ id: "doc1", content: big, small: "ok" });

		// Raw view (no middleware): the stored record must hold a blob marker,
		// not the payload.
		const raw = makeDexie("BlobOnShim") as DocsDB;
		raw.version(1).stores({ docs: "id" });
		const rawDoc = await raw.docs.get("doc1");
		const rawJson = JSON.stringify(rawDoc);
		expect(rawJson).toContain("__fl_blob__");
		expect(rawJson).not.toContain("xxxxxxxxxx");

		// Through the middleware the value rehydrates.
		const doc = await db.docs.get("doc1");
		expect(doc?.content).toBe(big);
		expect(doc?.small).toBe("ok");

		// Overwrite exercises old-hash collection + Dexie.waitFor keep-alive
		// inside a live shim transaction.
		const updated = "y".repeat(500);
		await db.docs.put({ id: "doc1", content: updated, small: "ok" });
		expect((await db.docs.get("doc1"))?.content).toBe(updated);

		raw.close();
		db.close();
	}, 20000);
});
