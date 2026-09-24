import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { IApp, IBoard } from "../schema";
import type { IBoardSummary } from "../schema/flow/board-summary";
import { type SuggestionStore, createMemorySuggestionStore } from "./store";
import {
	MAX_FETCHES_PER_RUN,
	type SyncBackend,
	localDay,
	syncCorpus,
	timeToMillis,
} from "./sync";
import type { BoardFacts } from "./types";

const SUB = "user";
const DAY_1 = "2026-09-23";
const DAY_2 = "2026-09-24";

const time = (ms: number) => ({
	secs_since_epoch: Math.floor(ms / 1000),
	nanos_since_epoch: (ms % 1000) * 1_000_000,
});

/** appId → boardId → `updatedAt` in ms (`undefined` = summary without it). */
type Library = Map<string, Record<string, number | undefined>>;

const libraryOf = (apps: Record<string, Record<string, number | undefined>>) =>
	new Map(Object.entries(apps));

function fakeBackend(library: Library) {
	const fetched: string[] = [];
	const listed: string[] = [];
	const failingBoards = new Set<string>();
	const failingApps = new Set<string>();
	let onFetch: (() => void) | undefined;
	const backend: SyncBackend = {
		appState: {
			getApps: async () =>
				[...library.keys()].map(
					(id, index) =>
						[{ id, updated_at: time(index * 1000) } as IApp, undefined] as [
							IApp,
							undefined,
						],
				),
		},
		boardState: {
			getBoardSummaries: async (appId) => {
				listed.push(appId);
				if (failingApps.has(appId)) throw new Error(`listing ${appId} failed`);
				return Object.entries(library.get(appId) ?? {}).map(
					([id, updatedAt]) =>
						({
							id,
							updatedAt: updatedAt === undefined ? undefined : time(updatedAt),
						}) as IBoardSummary,
				);
			},
			getBoard: async (appId, boardId) => {
				fetched.push(`${appId}/${boardId}`);
				onFetch?.();
				if (failingBoards.has(boardId)) throw new Error(`${boardId} failed`);
				return { id: boardId } as IBoard;
			},
		},
	};
	return {
		backend,
		fetched,
		listed,
		failingBoards,
		failingApps,
		setOnFetch: (callback: () => void) => {
			onFetch = callback;
		},
	};
}

const extract = async (board: IBoard): Promise<BoardFacts> => ({
	types: [board.id],
	transitions: [],
	outcomes: [],
});

function run(
	backend: SyncBackend,
	store: SuggestionStore,
	today: string,
	signal?: AbortSignal,
) {
	return syncCorpus({ backend, sub: SUB, today, store, extract, signal });
}

const boardIds = async (store: SuggestionStore) =>
	(await store.listBoards(SUB))
		.map((board) => `${board.appId}/${board.boardId}`)
		.sort();

describe("syncCorpus", () => {
	let warn: ReturnType<typeof spyOn>;
	beforeEach(() => {
		warn = spyOn(console, "warn").mockImplementation(() => {});
	});
	afterEach(() => warn.mockRestore());

	test("first run fetches every board once and stores its facts", async () => {
		const library = libraryOf({ a: { a1: 10, a2: 20 }, b: { b1: 30 } });
		const fake = fakeBackend(library);
		const store = createMemorySuggestionStore();

		const summary = await run(fake.backend, store, DAY_1);

		expect(summary).toEqual({
			apps: 2,
			fetched: 3,
			reused: 0,
			removed: 0,
			failed: 0,
			skipped: 0,
			complete: true,
		});
		expect(fake.fetched.sort()).toEqual(["a/a1", "a/a2", "b/b1"]);
		expect(await boardIds(store)).toEqual(["a/a1", "a/a2", "b/b1"]);
		const [a1] = await store.listBoards(SUB, "a");
		expect(await store.readFacts(SUB, [a1])).toEqual([
			JSON.stringify({ types: [a1.boardId], transitions: [], outcomes: [] }),
		]);
		expect((await store.listBoards(SUB, "a")).map((b) => b.updatedAt)).toEqual(
			expect.arrayContaining([10, 20]),
		);
		expect(await store.listApps(SUB)).toEqual(
			new Map([
				["a", DAY_1],
				["b", DAY_1],
			]),
		);
	});

	test("a second run on the same day touches no board", async () => {
		const fake = fakeBackend(libraryOf({ a: { a1: 10 }, b: { b1: 30 } }));
		const store = createMemorySuggestionStore();
		await run(fake.backend, store, DAY_1);
		fake.fetched.length = 0;
		fake.listed.length = 0;

		const summary = await run(fake.backend, store, DAY_1);

		expect(summary.fetched).toBe(0);
		expect(summary.apps).toBe(0);
		expect(summary.complete).toBe(true);
		expect(fake.fetched).toEqual([]);
		expect(fake.listed).toEqual([]);
	});

	test("the next day fetches only new and changed boards and forgets removed ones", async () => {
		const library = libraryOf({ a: { a1: 10, a2: 20, a3: 30 }, b: { b1: 40 } });
		const fake = fakeBackend(library);
		const store = createMemorySuggestionStore();
		await run(fake.backend, store, DAY_1);
		fake.fetched.length = 0;

		library.set("a", { a1: 10, a2: 25, a4: 50 });
		const summary = await run(fake.backend, store, DAY_2);

		expect(fake.fetched.sort()).toEqual(["a/a2", "a/a4"]);
		expect(summary).toMatchObject({
			apps: 2,
			fetched: 2,
			reused: 2,
			removed: 1,
			failed: 0,
		});
		expect(await boardIds(store)).toEqual(["a/a1", "a/a2", "a/a4", "b/b1"]);
		const a2 = (await store.listBoards(SUB, "a")).find(
			(board) => board.boardId === "a2",
		);
		expect(a2?.updatedAt).toBe(25);
	});

	test("a summary without updatedAt is fetched only while it has no facts", async () => {
		const fake = fakeBackend(libraryOf({ a: { a1: undefined } }));
		const store = createMemorySuggestionStore();
		await run(fake.backend, store, DAY_1);
		expect(fake.fetched).toEqual(["a/a1"]);

		await run(fake.backend, store, DAY_2);
		expect(fake.fetched).toEqual(["a/a1"]);
	});

	test("apps that disappeared lose their boards", async () => {
		const library = libraryOf({ a: { a1: 10 }, b: { b1: 20, b2: 30 } });
		const fake = fakeBackend(library);
		const store = createMemorySuggestionStore();
		await run(fake.backend, store, DAY_1);

		library.delete("b");
		const summary = await run(fake.backend, store, DAY_2);

		expect(summary.removed).toBe(2);
		expect(await boardIds(store)).toEqual(["a/a1"]);
		expect([...(await store.listApps(SUB)).keys()]).toEqual(["a"]);
	});

	test("an empty app listing forgets nothing", async () => {
		const library = libraryOf({ a: { a1: 10 } });
		const fake = fakeBackend(library);
		const store = createMemorySuggestionStore();
		await run(fake.backend, store, DAY_1);

		library.delete("a");
		await run(fake.backend, store, DAY_2);

		expect(await boardIds(store)).toEqual(["a/a1"]);
	});

	test("a failing board is isolated and retried the next day", async () => {
		const fake = fakeBackend(libraryOf({ a: { a1: 10, a2: 20, a3: 30 } }));
		fake.failingBoards.add("a2");
		const store = createMemorySuggestionStore();

		const first = await run(fake.backend, store, DAY_1);

		expect(first).toMatchObject({ apps: 1, fetched: 2, failed: 1 });
		expect(await boardIds(store)).toEqual(["a/a1", "a/a3"]);

		fake.failingBoards.clear();
		fake.fetched.length = 0;
		await run(fake.backend, store, DAY_1);
		expect(fake.fetched).toEqual([]);

		await run(fake.backend, store, DAY_2);
		expect(fake.fetched).toEqual(["a/a2"]);
	});

	test("a failing app listing leaves the run incomplete and is retried", async () => {
		const fake = fakeBackend(libraryOf({ a: { a1: 10 }, b: { b1: 20 } }));
		fake.failingApps.add("b");
		const store = createMemorySuggestionStore();

		const first = await run(fake.backend, store, DAY_1);

		expect(first).toMatchObject({ apps: 1, failed: 1, complete: false });
		fake.failingApps.clear();
		fake.fetched.length = 0;
		const second = await run(fake.backend, store, DAY_1);
		expect(second).toMatchObject({ apps: 1, fetched: 1, complete: true });
		expect(fake.fetched).toEqual(["b/b1"]);
	});

	test("abort stops the run without marking the app synced", async () => {
		const boards = Object.fromEntries(
			Array.from({ length: 10 }, (_, index) => [`x${index}`, index + 1]),
		);
		const fake = fakeBackend(libraryOf({ a: boards }));
		const store = createMemorySuggestionStore();
		const controller = new AbortController();
		fake.setOnFetch(() => {
			if (fake.fetched.length === 3) controller.abort();
		});

		const summary = await run(fake.backend, store, DAY_1, controller.signal);

		expect(summary.complete).toBe(false);
		expect(summary.apps).toBe(0);
		expect(fake.fetched.length).toBeLessThan(10);
		expect(await store.listApps(SUB)).toEqual(new Map());

		const stored = (await store.listBoards(SUB)).length;
		fake.setOnFetch(() => {});
		fake.fetched.length = 0;
		const resumed = await run(fake.backend, store, DAY_1);
		expect(resumed.complete).toBe(true);
		expect(fake.fetched.length).toBe(10 - stored);
	});

	test("the per-run fetch budget leaves the rest for the next run", async () => {
		const boards = Object.fromEntries(
			Array.from({ length: MAX_FETCHES_PER_RUN + 5 }, (_, index) => [
				`x${index}`,
				index + 1,
			]),
		);
		const fake = fakeBackend(libraryOf({ a: boards, b: { b1: 1 } }));
		const store = createMemorySuggestionStore();

		const first = await run(fake.backend, store, DAY_1);

		expect(first.fetched).toBe(MAX_FETCHES_PER_RUN);
		expect(first.complete).toBe(false);
		expect(first.skipped).toBe(1);
		fake.fetched.length = 0;
		const second = await run(fake.backend, store, DAY_1);
		expect(second.fetched).toBe(6);
		expect(second.complete).toBe(true);
	});

	test("lists and reads through the side-effect-free snapshots when the host has them", async () => {
		const fake = fakeBackend(libraryOf({ a: { a1: 1, a2: 2 } }));
		const snapshots: string[] = [];
		const { boardState } = fake.backend;
		boardState.getBoardSummariesSnapshot = async (appId) => [
			{ id: "a1", updatedAt: time(1) } as IBoardSummary,
			...(await boardState.getBoardSummaries(appId)).filter(
				(summary) => summary.id !== "a1",
			),
		];
		boardState.getBoardSnapshot = async (appId, boardId) => {
			snapshots.push(`${appId}/${boardId}`);
			return { id: boardId } as IBoard;
		};
		const listed = fake.listed.length;

		const summary = await run(
			fake.backend,
			createMemorySuggestionStore(),
			DAY_1,
		);

		expect(summary.fetched).toBe(2);
		expect(snapshots.sort()).toEqual(["a/a1", "a/a2"]);
		expect(fake.fetched).toEqual([]);
		expect(fake.listed.length - listed).toBe(1);
	});
});

describe("sync helpers", () => {
	test("timeToMillis reads system times, strings and gaps", () => {
		expect(timeToMillis(time(1_234_567))).toBe(1_234_567);
		expect(timeToMillis("1970-01-01T00:00:01.000Z")).toBe(1000);
		expect(timeToMillis(null)).toBe(0);
		expect(timeToMillis(undefined)).toBe(0);
		expect(timeToMillis("not a date")).toBe(0);
	});

	test("localDay is the local calendar day", () => {
		expect(localDay(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
	});
});
