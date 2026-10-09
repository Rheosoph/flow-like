import { type IBoard, ICommentType } from "@flow-like/flow-like-ui";
import type {
	IBoardSyncRequest,
	IBoardSyncResponse,
} from "@flow-like/flow-like-ui/lib/board-sync";
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

function nativeInvoke(disk: FakeDisk, response = syncResponse()) {
	return async (command: string, args?: unknown) => {
		switch (command) {
			case "get_app":
				return { visibility: "Private" };
			case "flowpilot_list_board_edit_jobs":
				return [];
			case "sync_board": {
				if (!disk.present) throw new Error("Board not found");
				const request = (args as { request: IBoardSyncRequest }).request;
				if (request.meta === response.manifest?.meta) {
					return { manifest: structuredClone(response.manifest) };
				}
				return structuredClone(response);
			}
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

function mediaSyncResponse(): IBoardSyncResponse {
	const response = syncResponse();
	if (!response.meta || !response.manifest)
		throw new Error("Missing fixture metadata");
	response.meta.updated_at = { secs_since_epoch: 20, nanos_since_epoch: 0 };
	response.manifest.comments = "comments-with-media";
	response.manifest.layers = { "layer-1": "layer-with-media" };
	const media = (id: string, commentType: ICommentType) => ({
		id,
		comment_type: commentType,
		content: `${id}.${commentType === ICommentType.Video ? "mp4" : "png"}`,
		coordinates: [0, 0, 0],
		width: 100,
		height: 100,
		timestamp: { secs_since_epoch: 20, nanos_since_epoch: 0 },
		hash: 1,
	});
	response.comments = {
		image: media("image", ICommentType.Image),
	} as IBoard["comments"];
	response.layers = {
		"layer-1": {
			id: "layer-1",
			name: "Layer",
			type: "Collapsed",
			nodes: {},
			variables: {},
			pins: {},
			coordinates: [0, 0, 0],
			comments: { video: media("video", ICommentType.Video) },
		},
	} as unknown as IBoard["layers"];
	return response;
}

function serveRemoteMedia(signature = "valid") {
	mocks.fetcher.mockImplementation(
		async (_profile: unknown, path: string, options?: { body?: string }) => {
			if (path.endsWith("/data/download")) {
				if (!options?.body) throw new Error("Missing download request body");
				const { prefixes } = JSON.parse(options.body) as {
					prefixes: string[];
				};
				return prefixes.map((prefix) => ({
					prefix,
					url: `https://storage.example/${prefix}?signature=${signature}`,
				}));
			}
			return mediaSyncResponse();
		},
	);
}

function expectMediaUrls(board: IBoard | undefined, signature = "valid") {
	if (!board) throw new Error("Missing display board");
	for (const comment of [
		board.comments.image,
		board.layers["layer-1"].comments.video,
	]) {
		expect(comment).toHaveProperty(
			"presigned_url",
			`https://storage.example/boards/${BOARD}/${comment.content}?signature=${signature}`,
		);
	}
}

describe("media from remote boards", () => {
	test("a newly materialized board resolves media even before app visibility is known", async () => {
		mocks.invoke.mockImplementation(
			nativeInvoke({ present: false }, mediaSyncResponse()),
		);
		serveRemoteMedia();
		const state = new BoardState(fakeBackend({ offline: true }) as never);

		expectMediaUrls(await state.getBoard(APP, BOARD));
	});

	test("a remotely fetched pinned version resolves media without writing the board", async () => {
		mocks.invoke.mockImplementation(nativeInvoke({ present: false }));
		serveRemoteMedia();
		const state = new BoardState(fakeBackend() as never);

		expectMediaUrls(await state.getBoard(APP, BOARD, [0, 0, 1]));
		expect(
			mocks.invoke.mock.calls.some(([name]) => name === "upsert_board"),
		).toBe(false);
	});

	test("a forced refresh publishes the new media with resolved URLs", async () => {
		mocks.invoke.mockImplementation(nativeInvoke({ present: true }));
		serveRemoteMedia();
		const backend = fakeBackend({ offline: false });
		const state = new BoardState(backend as never);

		expectMediaUrls(await state.getBoard(APP, BOARD, undefined, true));
		expectMediaUrls(backend.queryClient.setQueryData.mock.calls.at(-1)?.[1]);
	});

	test("background sync publishes the new media with resolved URLs", async () => {
		mocks.invoke.mockImplementation(nativeInvoke({ present: true }));
		serveRemoteMedia();
		const backend = fakeBackend({ offline: false });
		const state = new BoardState(backend as never);

		const local = await state.getBoard(APP, BOARD);
		expect(local.comments).toEqual({});
		await backend.backgroundTaskHandler.mock.calls[0][0];

		expectMediaUrls(backend.queryClient.setQueryData.mock.calls.at(-1)?.[1]);
	});

	test("refreshed URLs replace display records without changing earlier results or the sync baseline", async () => {
		mocks.invoke.mockImplementation(
			nativeInvoke({ present: true }, mediaSyncResponse()),
		);
		serveRemoteMedia();
		const backend = fakeBackend({ offline: false });
		const state = new BoardState(backend as never);

		const first = await state.getBoard(APP, BOARD);
		await backend.backgroundTaskHandler.mock.calls[0][0];
		serveRemoteMedia("refreshed");
		const refreshed = await state.getBoard(APP, BOARD);
		await backend.backgroundTaskHandler.mock.calls[1][0];

		expectMediaUrls(first);
		expectMediaUrls(refreshed, "refreshed");
		expect(refreshed).not.toBe(first);
		expect(refreshed.comments.image).not.toBe(first.comments.image);
		expect(refreshed.layers["layer-1"].comments.video).not.toBe(
			first.layers["layer-1"].comments.video,
		);
		expect(refreshed.nodes).toBe(first.nodes);
		// Display URLs must not make identical local and remote workflow data differ.
		expect(
			mocks.invoke.mock.calls.some(([name]) => name === "upsert_board"),
		).toBe(false);
	});

	test("offline boards still resolve media through local storage", async () => {
		const readBoard = nativeInvoke({ present: true }, mediaSyncResponse());
		mocks.invoke.mockImplementation(async (command: string, args: unknown) => {
			if (command === "storage_get") {
				return (args as { prefixes: string[] }).prefixes.map((prefix) => ({
					prefix,
					url: `asset://localhost/${prefix}`,
				}));
			}
			return readBoard(command, args);
		});
		const state = new BoardState(fakeBackend({ offline: true }) as never);

		const board = await state.getBoard(APP, BOARD);

		expect(board.comments.image.presigned_url).toBe(
			`asset://localhost/boards/${BOARD}/image.png`,
		);
		expect(board.layers["layer-1"].comments.video.presigned_url).toBe(
			`asset://localhost/boards/${BOARD}/video.mp4`,
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("a failed media refresh preserves the last display URLs until a later retry succeeds", async () => {
		mocks.invoke.mockImplementation(
			nativeInvoke({ present: true }, mediaSyncResponse()),
		);
		serveRemoteMedia();
		const state = new BoardState(fakeBackend({ offline: false }) as never);
		const version: [number, number, number] = [0, 0, 1];
		const first = await state.getBoard(APP, BOARD, version);

		mocks.fetcher.mockRejectedValue(new Error("Temporary download failure"));
		const failedRefresh = await state.getBoard(APP, BOARD, version);
		expect(failedRefresh).toBe(first);
		expectMediaUrls(failedRefresh);

		serveRemoteMedia("retried");
		const recovered = await state.getBoard(APP, BOARD, version);
		expectMediaUrls(recovered, "retried");
		expectMediaUrls(first);
	});
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
