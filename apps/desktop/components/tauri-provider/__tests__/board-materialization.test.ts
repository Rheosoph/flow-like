import type { IBoardSyncResponse } from "@flow-like/flow-like-ui/lib/board-sync";
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

import { BoardMaterializationError, BoardState } from "../board-state";

const APP = "app-1";
const BOARD = "board-1";

/** A complete board, as the server sends it to a client holding nothing. */
const syncResponse = (): IBoardSyncResponse =>
	({
		manifest: {
			meta: "meta-1",
			variables: "vars-1",
			comments: "comments-1",
			layers: {},
			segments: {},
		},
		meta: {
			id: BOARD,
			name: "Board",
			description: "",
			viewport: [0, 0, 0],
			version: [0, 0, 1],
			stage: "Dev",
			log_level: "Info",
			execution_mode: "Hybrid",
			page_ids: ["page-1"],
			created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			updated_at: { secs_since_epoch: 10, nanos_since_epoch: 0 },
		},
		variables: {},
		comments: {},
		layers: {},
		refs: {},
		segments: {},
	}) as unknown as IBoardSyncResponse;

interface FakeDisk {
	/** Whether the board file exists. This is the whole point of the suite. */
	present: boolean;
	/** Fails the write, the way a rejected `upsert_board` does on a real device. */
	writeFails?: boolean;
	/** Accepts the write but leaves nothing behind — a write that lies. */
	writeIsALie?: boolean;
}

function fakeBackend(
	overrides: {
		localOnly?: boolean;
		offline?: boolean;
		authenticated?: boolean;
	} = {},
) {
	const authenticated = overrides.authenticated ?? true;
	return {
		// Unknown visibility reads as offline; only an explicit local-only app is local-only.
		isOffline: vi.fn().mockResolvedValue(overrides.offline ?? true),
		isLocalOnly: vi.fn().mockResolvedValue(overrides.localOnly ?? false),
		profile: authenticated ? { id: "profile-1", hub: "hub-1" } : undefined,
		auth: authenticated ? { user: { profile: { sub: "user-1" } } } : undefined,
		queryClient: { setQueryData: vi.fn(), invalidateQueries: vi.fn() },
		backgroundTaskHandler: vi.fn(),
		getOfflineSyncCommands: vi.fn(async () => []),
		getBoardLineage: vi.fn(async () => undefined),
		recordBoardLineage: vi.fn(async () => undefined),
		clearBoardLineage: vi.fn(async () => undefined),
	};
}

function nativeInvoke(disk: FakeDisk) {
	return async (command: string, args?: unknown) => {
		switch (command) {
			case "get_app":
				return { visibility: "Private" };
			case "flowpilot_list_board_edit_jobs":
				return [];
			case "sync_board":
				if (!disk.present) throw new Error("Board not found");
				return syncResponse();
			case "upsert_board":
				if (disk.writeFails) throw new Error("invalid args `boardData`");
				if (!disk.writeIsALie) disk.present = true;
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

/**
 * `getBoard` is the only code on the interface route that can create a board file: a hosted
 * app's manifest arrives listing board ids whose payloads were never downloaded. Every caller
 * reads from disk again the moment it resolves, so "resolved" has to mean "materialized" —
 * a download that resolves without the file landing makes every retry re-run an identical,
 * invisible failure.
 */
describe("materializing a board this device does not have", () => {
	test("resolving means the board file is on disk", async () => {
		const disk: FakeDisk = { present: false };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		mocks.fetcher.mockResolvedValue(syncResponse());
		const backend = fakeBackend();
		const state = new BoardState(backend as never);

		const board = await state.getBoard(APP, BOARD, undefined, true);

		expect(board.id).toBe(BOARD);
		expect(disk.present).toBe(true);
		// The returned board is the one read back from disk, not the one downloaded.
		expect(mocks.invoke).toHaveBeenCalledWith(
			"upsert_board",
			expect.objectContaining({ appId: APP, boardId: BOARD }),
		);
		expect(backend.recordBoardLineage).toHaveBeenCalled();
	});

	test("a persist failure rejects instead of reporting success", async () => {
		const disk: FakeDisk = { present: false, writeFails: true };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		mocks.fetcher.mockResolvedValue(syncResponse());
		const state = new BoardState(fakeBackend() as never);

		const error = await state
			.getBoard(APP, BOARD, undefined, true)
			.then(() => undefined)
			.catch((e: unknown) => e);

		expect(error).toBeInstanceOf(BoardMaterializationError);
		expect((error as BoardMaterializationError).phase).toBe("persist");
		expect(disk.present).toBe(false);
	});

	test("a write that leaves nothing behind is caught by reading it back", async () => {
		const disk: FakeDisk = { present: false, writeIsALie: true };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		mocks.fetcher.mockResolvedValue(syncResponse());
		const backend = fakeBackend();
		const state = new BoardState(backend as never);

		const error = await state
			.getBoard(APP, BOARD, undefined, true)
			.then(() => undefined)
			.catch((e: unknown) => e);

		expect(error).toBeInstanceOf(BoardMaterializationError);
		expect((error as BoardMaterializationError).phase).toBe("verify");
		// Lineage must not advance for a revision that was never persisted, or every later
		// remote apply for this board is judged against a revision the device never had.
		expect(backend.recordBoardLineage).not.toHaveBeenCalled();
	});

	test("an app whose visibility is unknown is still allowed to ask the server", async () => {
		const disk: FakeDisk = { present: false };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		mocks.fetcher.mockResolvedValue(syncResponse());
		const backend = fakeBackend({ offline: true, localOnly: false });
		const state = new BoardState(backend as never);

		await state.getBoard(APP, BOARD, undefined, true);

		expect(mocks.fetcher).toHaveBeenCalled();
		expect(disk.present).toBe(true);
	});

	test("an explicitly local-only app never reaches the network", async () => {
		const disk: FakeDisk = { present: false };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		const state = new BoardState(fakeBackend({ localOnly: true }) as never);

		const error = await state
			.getBoard(APP, BOARD, undefined, true)
			.then(() => undefined)
			.catch((e: unknown) => e);

		expect(error).toBeInstanceOf(BoardMaterializationError);
		expect((error as BoardMaterializationError).phase).toBe("gated");
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("an unreachable server names the fetch, not the missing file", async () => {
		const disk: FakeDisk = { present: false };
		mocks.invoke.mockImplementation(nativeInvoke(disk));
		mocks.fetcher.mockRejectedValue(new Error("hub unreachable"));
		const state = new BoardState(fakeBackend() as never);

		const error = await state
			.getBoard(APP, BOARD, undefined, true)
			.then(() => undefined)
			.catch((e: unknown) => e);

		expect(error).toBeInstanceOf(BoardMaterializationError);
		expect((error as BoardMaterializationError).phase).toBe("fetch");
		expect((error as BoardMaterializationError).cause).toBeInstanceOf(Error);
	});
});
