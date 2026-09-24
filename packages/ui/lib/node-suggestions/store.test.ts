import { describe, expect, test } from "bun:test";
import {
	TableSuggestionStore,
	createMemorySuggestionStore,
	createMemoryTables,
} from "./store";
import { SUGGESTION_FORMAT_VERSION } from "./types";

const SUB = "user";
const OLD = SUGGESTION_FORMAT_VERSION - 1;
const key = (...parts: string[]) => parts.join("\u001f");

describe("suggestion store", () => {
	test("round-trips boards, facts, sync days and models per sub", async () => {
		const store = createMemorySuggestionStore();
		const board = {
			appId: "app",
			boardId: "b1",
			updatedAt: 42,
			transitions: 3,
		};
		await store.putBoard(SUB, board, '{"types":[]}');
		await store.markAppSynced(SUB, "app", "2026-09-23");
		await store.putModel(SUB, "ngram", {
			fingerprint: "fp",
			trainedAt: 1,
			payload: "{}",
			boards: 1,
			transitions: 3,
		});
		await store.setSyncDay(SUB, "2026-09-23");

		expect(await store.listBoards(SUB)).toEqual([board]);
		expect(await store.listBoards(SUB, "app")).toEqual([board]);
		expect(await store.listBoards(SUB, "other")).toEqual([]);
		expect(await store.listBoards("someone-else")).toEqual([]);
		expect(await store.readFacts(SUB, [board])).toEqual(['{"types":[]}']);
		expect(await store.listApps(SUB)).toEqual(new Map([["app", "2026-09-23"]]));
		expect((await store.getModel(SUB, "ngram"))?.fingerprint).toBe("fp");
		expect(await store.getModel(SUB, "neural")).toBeUndefined();
		expect(await store.getSyncDay(SUB)).toBe("2026-09-23");

		await store.deleteApps(SUB, ["app"]);
		expect(await store.listBoards(SUB)).toEqual([]);
		expect(await store.readFacts(SUB, [board])).toEqual([]);
		expect(await store.listApps(SUB)).toEqual(new Map());
	});

	test("drops rows written by another fact format", async () => {
		const tables = createMemoryTables();
		const id = key(SUB, "app", "old");
		await tables.boards.put({
			id,
			sub: SUB,
			appId: "app",
			boardId: "old",
			updatedAt: 1,
			format: OLD,
			transitions: 1,
		});
		await tables.facts.put({ id, sub: SUB, facts: "{}" });
		await tables.apps.put({
			id: key(SUB, "app"),
			sub: SUB,
			appId: "app",
			syncedDay: "2026-09-23",
			syncedAt: 1,
			format: OLD,
		});
		await tables.models.put({
			id: key(SUB, "ngram"),
			sub: SUB,
			kind: "ngram",
			format: OLD,
			fingerprint: "fp",
			trainedAt: 1,
			payload: "{}",
			boards: 1,
			transitions: 1,
		});
		await tables.meta.put({
			id: SUB,
			sub: SUB,
			syncedDay: "2026-09-23",
			syncedAt: 1,
			format: OLD,
		});
		const store = new TableSuggestionStore(tables, false);

		expect(await store.listBoards(SUB)).toEqual([]);
		expect(await tables.boards.get(id)).toBeUndefined();
		expect(await tables.facts.get(id)).toBeUndefined();
		expect(await store.listApps(SUB)).toEqual(new Map());
		expect(await store.getModel(SUB, "ngram")).toBeUndefined();
		expect(await tables.models.get(key(SUB, "ngram"))).toBeUndefined();
		expect(await store.getSyncDay(SUB)).toBeUndefined();
	});
});
