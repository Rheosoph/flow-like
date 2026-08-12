import type { IBoard, IGenericCommand } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	streamFetcher: vi.fn(),
}));

vi.mock("sonner", () => ({
	toast: Object.assign(vi.fn(), {
		success: vi.fn(),
		error: vi.fn(),
		info: vi.fn(),
		warning: vi.fn(),
		dismiss: vi.fn(),
	}),
}));

import { BoardState, BoardSyncDiscardRequiredError } from "../board-state";
import type { CommandSyncQueueRow } from "../command-sync";

const APP = "app-1";
const BOARD = "board-1";

const board = (seconds: number, nodeId: string): IBoard =>
	({
		id: BOARD,
		name: "Board",
		description: "",
		nodes: { [nodeId]: { id: nodeId, pins: {} } },
		layers: {},
		variables: {},
		comments: {},
		refs: {},
		page_ids: [],
		updated_at: { secs_since_epoch: seconds, nanos_since_epoch: 0 },
	}) as unknown as IBoard;

const queuedRow = (
	overrides: Partial<CommandSyncQueueRow> = {},
): CommandSyncQueueRow => ({
	commandId: "cmd-1",
	createdAt: new Date("2026-01-01T00:00:00.000Z"),
	chunks: [[{ command_type: "AddNode" } as unknown as IGenericCommand]],
	remoteIdentityVersion: 1,
	remoteProfileId: "profile-old",
	remotePrincipalId: "user-old",
	remoteHub: "hub-old",
	...overrides,
});

interface FakeBackend {
	queue: (CommandSyncQueueRow & { appId: string; boardId: string })[];
	lineage: Map<string, number>;
	archived: { commandId: string; reason: string }[];
	profile?: { id: string; hub: string };
	auth?: { user: { profile: { sub: string } } };
	queryClient: {
		setQueryData: ReturnType<typeof vi.fn>;
		invalidateQueries: ReturnType<typeof vi.fn>;
	};
	[key: string]: unknown;
}

function fakeBackend(rows: CommandSyncQueueRow[] = []): FakeBackend {
	const backend: FakeBackend = {
		queue: rows.map((row) => ({ ...row, appId: APP, boardId: BOARD })),
		lineage: new Map<string, number>(),
		archived: [],
		profile: { id: "profile-new", hub: "hub-new" },
		auth: { user: { profile: { sub: "user-new" } } },
		queryClient: { setQueryData: vi.fn(), invalidateQueries: vi.fn() },
		isOffline: vi.fn().mockResolvedValue(false),
		backgroundTaskHandler: vi.fn(),
		getOfflineSyncCommands: vi.fn(async () => backend.queue),
		archiveOfflineSyncCommands: vi.fn(
			async (
				_appId: string,
				_boardId: string,
				commandIds: readonly string[],
				reason: string,
			) => {
				const removed = backend.queue.filter((row) =>
					commandIds.includes(row.commandId),
				);
				backend.queue = backend.queue.filter(
					(row) => !commandIds.includes(row.commandId),
				);
				backend.archived.push(
					...removed.map((row) => ({ commandId: row.commandId, reason })),
				);
				return removed.length;
			},
		),
		clearOfflineSyncCommands: vi.fn(async (commandId: string) => {
			backend.queue = backend.queue.filter(
				(row) => row.commandId !== commandId,
			);
		}),
		recordOfflineSyncFailure: vi.fn(),
		blockOfflineSyncCommand: vi.fn(),
		getBoardLineage: vi.fn(async () => backend.lineage.get(BOARD)),
		recordBoardLineage: vi.fn(async (_a: string, _b: string, ns: number) => {
			const existing = backend.lineage.get(BOARD) ?? 0;
			if (ns > existing) backend.lineage.set(BOARD, ns);
		}),
		clearBoardLineage: vi.fn(async () => {
			backend.lineage.delete(BOARD);
		}),
		listOfflineSyncArchive: vi.fn(async () => backend.archived),
	};
	return backend;
}

function nativeInvoke(localBoard?: IBoard) {
	return async (command: string, args?: unknown) => {
		switch (command) {
			case "get_app":
				return { visibility: "Public" };
			case "flowpilot_list_board_edit_jobs":
				return [];
			case "get_board":
				if (!localBoard) throw new Error("Board not found");
				return localBoard;
			case "upsert_board":
				return undefined;
			default:
				throw new Error(
					`unexpected invoke: ${command} ${JSON.stringify(args)}`,
				);
		}
	};
}

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
});

describe("resetBoardFromServer", () => {
	test("a wedged queue is refused until the discard is authorized", async () => {
		const backend = fakeBackend([queuedRow()]);
		mocks.invoke.mockImplementation(nativeInvoke(board(10, "local-node")));
		const state = new BoardState(backend as never);

		await expect(
			state.resetBoardFromServer(APP, BOARD, { discardQueuedEdits: false }),
		).rejects.toBeInstanceOf(BoardSyncDiscardRequiredError);

		expect(backend.queue).toHaveLength(1);
		expect(backend.archived).toHaveLength(0);
		expect(backend.clearBoardLineage).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("the refusal carries the itemized queue so it can be reviewed", async () => {
		const backend = fakeBackend([queuedRow()]);
		mocks.invoke.mockImplementation(nativeInvoke(board(10, "local-node")));
		const state = new BoardState(backend as never);

		const error: unknown = await state
			.resetBoardFromServer(APP, BOARD, { discardQueuedEdits: false })
			.then(() => undefined)
			.catch((e) => e);

		expect(error).toBeInstanceOf(BoardSyncDiscardRequiredError);
		const { status } = error as BoardSyncDiscardRequiredError;
		expect(status.pendingBatches).toBe(1);
		expect(status.ownershipMismatch).toContain("different remote profile");
	});

	test("authorized discard clears the queue, clears lineage and applies the server board", async () => {
		const backend = fakeBackend([
			queuedRow(),
			queuedRow({ commandId: "cmd-2" }),
		]);
		backend.lineage.set(BOARD, 9_999_999_999_999);
		const local = board(10, "local-node");
		const remote = board(5, "remote-node");
		mocks.invoke.mockImplementation(nativeInvoke(local));
		mocks.fetcher.mockResolvedValue(remote);
		const state = new BoardState(backend as never);

		const result = await state.resetBoardFromServer(APP, BOARD, {
			discardQueuedEdits: true,
		});

		expect(result.discardedBatches).toBe(2);
		expect(backend.queue).toHaveLength(0);
		expect(backend.archived.map((entry) => entry.commandId)).toEqual([
			"cmd-1",
			"cmd-2",
		]);
		expect(backend.clearBoardLineage).toHaveBeenCalled();
		// Cleared, then re-stamped from the snapshot that was actually applied.
		expect(backend.lineage.get(BOARD)).toBe(5_000_000_000);
		expect(result.board.nodes["remote-node"]).toBeDefined();
		expect(result.board.nodes["local-node"]).toBeUndefined();

		const upsert = mocks.invoke.mock.calls.find(
			([command]) => command === "upsert_board",
		);
		expect(upsert?.[1].authoritativeUpdatedAt).toEqual(remote.updated_at);
		expect(backend.queryClient.setQueryData).toHaveBeenCalled();
		expect(backend.queryClient.invalidateQueries).toHaveBeenCalled();
	});

	test("an older server revision still wins — the reset bypasses the freshness guards", async () => {
		const backend = fakeBackend([queuedRow()]);
		mocks.invoke.mockImplementation(nativeInvoke(board(9_999, "local-node")));
		mocks.fetcher.mockResolvedValue(board(1, "remote-node"));
		const state = new BoardState(backend as never);

		const result = await state.resetBoardFromServer(APP, BOARD, {
			discardQueuedEdits: true,
		});
		expect(result.board.nodes["remote-node"]).toBeDefined();
	});

	test("a failed fetch leaves the queue and lineage untouched", async () => {
		const backend = fakeBackend([queuedRow()]);
		backend.lineage.set(BOARD, 42);
		mocks.invoke.mockImplementation(nativeInvoke(board(10, "local-node")));
		mocks.fetcher.mockRejectedValue(new Error("network down"));
		const state = new BoardState(backend as never);

		await expect(
			state.resetBoardFromServer(APP, BOARD, { discardQueuedEdits: true }),
		).rejects.toThrow("network down");

		expect(backend.queue).toHaveLength(1);
		expect(backend.archived).toHaveLength(0);
		expect(backend.lineage.get(BOARD)).toBe(42);
	});

	test("local-only apps are refused — the queue is the only copy of those edits", async () => {
		const backend = fakeBackend([queuedRow()]);
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_app") return { visibility: "Offline" };
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new BoardState(backend as never);

		await expect(
			state.resetBoardFromServer(APP, BOARD, { discardQueuedEdits: true }),
		).rejects.toThrow(/local-only/);
		expect(backend.queue).toHaveLength(1);
	});

	test("a FlowPilot delivery still owning native replay blocks the reset", async () => {
		const backend = fakeBackend([queuedRow()]);
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_app") return { visibility: "Public" };
			if (command === "flowpilot_list_board_edit_jobs")
				return [{ jobId: "job-1", phase: "applied_pending_delivery" }];
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new BoardState(backend as never);

		await expect(
			state.resetBoardFromServer(APP, BOARD, { discardQueuedEdits: true }),
		).rejects.toThrow(/durable delivery/);
		expect(backend.queue).toHaveLength(1);
	});

	test("a tombstone-only queue needs no authorization and is preserved", async () => {
		const backend = fakeBackend([
			queuedRow({
				commandId: "tombstone",
				chunks: [],
				deferReceiptAckUntilNativeTerminal: true,
			} as Partial<CommandSyncQueueRow>),
		]);
		mocks.invoke.mockImplementation(nativeInvoke(board(10, "local-node")));
		mocks.fetcher.mockResolvedValue(board(20, "remote-node"));
		const state = new BoardState(backend as never);

		const result = await state.resetBoardFromServer(APP, BOARD, {
			discardQueuedEdits: false,
		});

		expect(result.discardedBatches).toBe(0);
		expect(backend.queue).toHaveLength(1);
		expect(result.board.nodes["remote-node"]).toBeDefined();
	});

	test("a board missing locally is fetched and written without a merge partner", async () => {
		const backend = fakeBackend();
		mocks.invoke.mockImplementation(nativeInvoke(undefined));
		mocks.fetcher.mockResolvedValue(board(7, "remote-node"));
		const state = new BoardState(backend as never);

		const result = await state.resetBoardFromServer(APP, BOARD, {
			discardQueuedEdits: false,
		});
		expect(result.board.nodes["remote-node"]).toBeDefined();
	});
});

describe("getBoardSyncStatus", () => {
	test("reports the mismatch that wedges the board", async () => {
		const backend = fakeBackend([queuedRow()]);
		const state = new BoardState(backend as never);

		const status = await state.getBoardSyncStatus(APP, BOARD);
		expect(status.supported).toBe(true);
		expect(status.pendingBatches).toBe(1);
		expect(status.ownershipMismatch).toContain("different remote profile");
	});

	test("a queue read failure never breaks the surface that reports it", async () => {
		const backend = fakeBackend();
		backend.getOfflineSyncCommands = vi
			.fn()
			.mockRejectedValue(new Error("idb closed"));
		const state = new BoardState(backend as never);

		const status = await state.getBoardSyncStatus(APP, BOARD);
		expect(status).toMatchObject({ supported: true, pendingBatches: 0 });
	});
});
