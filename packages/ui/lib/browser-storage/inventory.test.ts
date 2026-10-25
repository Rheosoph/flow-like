import "fake-indexeddb/auto";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { chatDb } from "../../components/interfaces/chat-default/chat-db";
import {
	boardLastSeenDb,
	writeBoardLastSeen,
} from "../../db/board-last-seen-db";
import { globalChatDb } from "../../state/global-chat/global-chat-db";
import { DexieHistoryPersistence } from "../flow-history-db";
import { BoardHistoryRegistry, boardHistoryKey } from "../flow-history-store";
import { runIDBCleanup } from "../idb-cleanup";
import { offlineSyncDB } from "../sync-db";
import {
	type BrowserStorageEntry,
	deleteBrowserStorageEntries,
	scanBrowserStorage,
} from "./inventory";
import { clearIndexedDatabaseStores, scanIndexedDatabase } from "./measure";
import {
	DAY_MS,
	DEFAULT_RETENTION_POLICY,
	parseRetentionPolicy,
} from "./retention";

const persistence = new DexieHistoryPersistence();
const registry = new BoardHistoryRegistry(persistence);
const clearBoardHistory = (key: string) => registry.clearKey(key);

async function recordHistory(appId: string, boardId: string, at: number) {
	const clock = vi.spyOn(Date, "now").mockReturnValue(at);
	try {
		await persistence.write(boardHistoryKey(appId, boardId), {
			put: [{ seq: 1, at, payload: "x".repeat(1_000), bytes: 1_000 }],
			appliedSeq: 1,
			nextSeq: 2,
		});
	} finally {
		clock.mockRestore();
	}
}

const byId = (entries: readonly BrowserStorageEntry[], id: string) =>
	entries.find((entry) => entry.id === id);

function entryOf(entries: readonly BrowserStorageEntry[], id: string) {
	const entry = byId(entries, id);
	if (!entry) throw new Error(`No storage entry ${id}`);
	return entry;
}

beforeEach(async () => {
	await Promise.all([
		boardLastSeenDb.seen.clear(),
		chatDb.sessions.clear(),
		chatDb.messages.clear(),
		globalChatDb.sessions.clear(),
		globalChatDb.messages.clear(),
		offlineSyncDB.commands.clear(),
		offlineSyncDB.discarded.clear(),
	]);
	await clearIndexedDatabaseStores("undo-redo").catch(() => {});
});

describe("retention policy", () => {
	it("keeps valid stored days and falls back per kind", () => {
		const policy = parseRetentionPolicy(
			JSON.stringify({
				undoHistory: 7,
				chats: null,
				flowpilot: -3,
				viewports: "x",
			}),
		);
		expect(policy.undoHistory).toBe(7);
		expect(policy.chats).toBeNull();
		expect(policy.flowpilot).toBe(DEFAULT_RETENTION_POLICY.flowpilot);
		expect(policy.viewports).toBe(DEFAULT_RETENTION_POLICY.viewports);
		expect(parseRetentionPolicy("{not json")).toBe(DEFAULT_RETENTION_POLICY);
	});
});

describe("scanBrowserStorage", () => {
	it("joins a board's undo history and change snapshot into one entry", async () => {
		await recordHistory("app1", "board1", Date.now());
		await writeBoardLastSeen("user", "app1", "board1", {
			payload: "y".repeat(500),
			encoding: "gzip-base64",
		});
		await writeBoardLastSeen("user", "app2", "board2", {
			payload: "z".repeat(200),
			encoding: "gzip-base64",
		});

		const { entries } = await scanBrowserStorage();
		const first = byId(entries, "board:app1:board1");
		expect(first?.board?.historyKeys).toEqual(["app1_board1"]);
		expect(first?.board?.snapshotIds).toEqual(["user:app1:board1"]);
		expect(first?.board?.undoSteps).toBe(1);
		expect(first?.sizeBytes).toBeGreaterThan(1_500);
		expect(byId(entries, "board:app2:board2")?.board?.historyKeys).toEqual([]);
	});

	it("attributes object stores of a shared database to separate entries", async () => {
		await offlineSyncDB.discarded.put({
			archiveId: "a",
			appId: "app",
			boardId: "board",
			archivedAt: new Date(),
			commands: ["c".repeat(400)],
		} as never);

		const { entries } = await scanBrowserStorage();
		expect(byId(entries, "discardedSync")?.records).toBe(1);
		expect(byId(entries, "discardedSync")?.deletable).toBe(true);
		expect(byId(entries, "pendingSync")).toBeUndefined();
	});
});

describe("deleteBrowserStorageEntries", () => {
	it("removes a board's history and snapshot but leaves other boards", async () => {
		await recordHistory("app1", "board1", Date.now());
		await recordHistory("app1", "board2", Date.now());
		await writeBoardLastSeen("user", "app1", "board1", {
			payload: "y",
			encoding: "gzip-base64",
		});

		const { entries } = await scanBrowserStorage();
		const result = await deleteBrowserStorageEntries(
			[entryOf(entries, "board:app1:board1")],
			{ clearBoardHistory },
		);
		expect(result).toEqual({ deleted: 1, failed: [] });

		const after = await scanBrowserStorage();
		expect(byId(after.entries, "board:app1:board1")).toBeUndefined();
		expect(byId(after.entries, "board:app1:board2")).toBeDefined();
		expect(await boardLastSeenDb.seen.count()).toBe(0);
	});

	it("empties only the listed object stores and keeps pinned chats", async () => {
		await offlineSyncDB.commands.put({
			commandId: "pending",
			appId: "app",
			boardId: "board",
			commands: [],
			createdAt: new Date(),
		} as never);
		await offlineSyncDB.discarded.put({
			archiveId: "a",
			appId: "app",
			boardId: "board",
			archivedAt: new Date(),
		} as never);
		await globalChatDb.sessions.bulkPut([
			{ id: "keep", updatedAt: 1, pinnedAt: 5 } as never,
			{ id: "drop", updatedAt: 1 } as never,
		]);
		await globalChatDb.messages.bulkPut([
			{ id: "m1", sessionId: "keep", timestamp: 1 } as never,
			{ id: "m2", sessionId: "drop", timestamp: 1 } as never,
		]);

		const { entries } = await scanBrowserStorage();
		expect(entryOf(entries, "pendingSync").deletable).toBe(false);
		const result = await deleteBrowserStorageEntries(
			[entryOf(entries, "discardedSync"), entryOf(entries, "globalChat")],
			{ clearBoardHistory },
		);
		expect(result.failed).toEqual([]);
		expect(await offlineSyncDB.discarded.count()).toBe(0);
		expect(await offlineSyncDB.commands.count()).toBe(1);
		expect(await globalChatDb.sessions.toCollection().primaryKeys()).toEqual([
			"keep",
		]);
		expect(await globalChatDb.messages.toCollection().primaryKeys()).toEqual([
			"m1",
		]);
	});

	it("refuses protected entries", async () => {
		const result = await deleteBrowserStorageEntries(
			[
				{
					id: "pendingSync",
					category: "protected",
					store: "pendingSync",
					sizeBytes: 0,
					records: 0,
					deletable: false,
				},
			],
			{ clearBoardHistory },
		);
		expect(result.deleted).toBe(0);
		expect(result.failed).toHaveLength(1);
	});
});

describe("runIDBCleanup", () => {
	it("prunes by the policy and skips kinds kept forever", async () => {
		const old = Date.now() - 40 * DAY_MS;
		await recordHistory("app", "stale", old);
		await globalChatDb.sessions.put({ id: "old", updatedAt: old } as never);

		const keepUndo = await runIDBCleanup({
			...DEFAULT_RETENTION_POLICY,
			undoHistory: null,
			globalChat: null,
		});
		expect(keepUndo.undoHistory).toBeUndefined();
		expect(await globalChatDb.sessions.count()).toBe(1);

		const pruned = await runIDBCleanup({
			...DEFAULT_RETENTION_POLICY,
			undoHistory: 30,
			globalChat: 30,
		});
		expect(pruned.undoHistory).toBe(1);
		expect(pruned.globalChat).toBe(1);
		expect(await globalChatDb.sessions.count()).toBe(0);
		const remaining = await scanIndexedDatabase("undo-redo");
		expect(remaining.stores.entries.records).toBe(0);
	});
});
