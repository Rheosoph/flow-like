import { describe, expect, test } from "bun:test";
import { clearServiceHistory } from "./history";

type Stores = Record<string, unknown[]>;

function fakeIndexedDb(
	databases: Record<string, Stores>,
	options: { list?: boolean; abort?: string } = {},
) {
	const opened: string[] = [];
	const closed: string[] = [];
	const open = (name: string) => {
		opened.push(name);
		const stores = databases[name] ?? {};
		const request: {
			result?: unknown;
			error: null;
			onsuccess?: () => void;
		} = { error: null };
		queueMicrotask(() => {
			request.result = {
				objectStoreNames: Object.keys(stores),
				transaction: (scope: string[], mode: string) => {
					expect(scope).toEqual(Object.keys(stores));
					expect(mode).toBe("readwrite");
					const transaction: {
						error: Error | null;
						oncomplete?: () => void;
						onabort?: () => void;
						objectStore: (store: string) => { clear: () => void };
					} = {
						error: null,
						objectStore: (store) => ({
							clear: () => {
								if (options.abort !== name) stores[store].length = 0;
							},
						}),
					};
					setTimeout(() => {
						if (options.abort !== name) return transaction.oncomplete?.();
						transaction.error = new Error("blocked");
						transaction.onabort?.();
					});
					return transaction;
				},
				close: () => closed.push(name),
			};
			request.onsuccess?.();
		});
		return request;
	};
	const databasesList = async () =>
		Object.keys(databases).map((name) => ({ name, version: 1 }));
	const idb =
		options.list === false ? { open } : { open, databases: databasesList };
	return { idb: idb as unknown as IDBFactory, opened, closed };
}

const viewerData = (): Record<string, Stores> => ({
	"Chat-History": { sessions: ["s1"], messages: ["m1"], drafts: ["d1"] },
	"UI-State-DB": { elementValues: ["v1"], pageState: ["p1"] },
	"flow-like-page-state": { "page-state": ["p2"] },
	"flow-like-global-state": { "global-state": ["g1"] },
	"flow-like-element-values": { "element-values": ["v2"] },
});

describe("standalone service history", () => {
	test("locking empties every store on the service origin", async () => {
		const databases = viewerData();
		const { idb, opened, closed } = fakeIndexedDb(databases);
		await clearServiceHistory(idb);
		for (const stores of Object.values(databases))
			for (const rows of Object.values(stores)) expect(rows).toEqual([]);
		expect(opened.sort()).toEqual(Object.keys(databases).sort());
		expect(closed.sort()).toEqual(Object.keys(databases).sort());
	});

	test("a browser that cannot list databases clears the known ones and reports the rest", async () => {
		const databases = viewerData();
		const { idb, opened } = fakeIndexedDb(databases, { list: false });
		await expect(clearServiceHistory(idb)).rejects.toThrow("cannot list");
		expect(opened.sort()).toEqual(["Chat-History", "UI-State-DB"]);
		expect(databases["Chat-History"].messages).toEqual([]);
		expect(databases["UI-State-DB"].pageState).toEqual([]);
		expect(databases["flow-like-page-state"]["page-state"]).toEqual(["p2"]);
	});

	test("a failed clear reaches the caller and still closes the database", async () => {
		const { idb, closed } = fakeIndexedDb(viewerData(), {
			abort: "flow-like-page-state",
		});
		await expect(clearServiceHistory(idb)).rejects.toThrow("blocked");
		expect(closed).toContain("flow-like-page-state");
	});
});
