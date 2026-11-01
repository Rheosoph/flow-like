import { IVersionType } from "@flow-like/flow-like-ui/lib/schema/flow/version-type";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), fetcher: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));
vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	streamFetcher: vi.fn(),
}));

import { BoardState } from "../board-state";

function backend(localOnly = false) {
	return {
		isLocalOnly: vi.fn().mockResolvedValue(localOnly),
		isOffline: vi.fn().mockResolvedValue(localOnly),
		profile: { id: "profile", hub: "hub" },
		auth: {
			isAuthenticated: true,
			user: { access_token: "token", profile: { sub: "user" } },
		},
		queryClient: {},
		backgroundTaskHandler: vi.fn(),
	};
}

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
});

describe("board version status", () => {
	test("reads local-only publication state from the native command", async () => {
		const result = { current: [0, 0, 4], newest: [0, 0, 4] };
		mocks.invoke.mockResolvedValue(result);
		const state = new BoardState(backend(true) as never);

		expect(await state.getBoardVersionCurrent("app", "board")).toEqual(result);
		expect(mocks.invoke).toHaveBeenCalledWith("get_board_version_current", {
			appId: "app",
			boardId: "board",
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("reads hosted publication state without substituting a local draft", async () => {
		const source = backend();
		const result = { current: null, newest: [0, 0, 4] };
		mocks.fetcher.mockResolvedValue(result);
		const state = new BoardState(source as never);

		expect(await state.getBoardVersionCurrent("app#1", "board/1")).toEqual(
			result,
		);
		expect(mocks.fetcher).toHaveBeenCalledWith(
			source.profile,
			"apps/app%231/board/board%2F1/version/current",
			{ method: "GET" },
			source.auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("a failed hosted read stays unknown instead of reading local state", async () => {
		const failure = new Error("Hub unavailable");
		mocks.fetcher.mockRejectedValue(failure);
		const state = new BoardState(backend() as never);

		await expect(state.getBoardVersionCurrent("app", "board")).rejects.toBe(
			failure,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});

describe("board publication completion", () => {
	test("does not report completion before the hosted version is created", async () => {
		const source = backend();
		mocks.invoke.mockResolvedValue([0, 0, 5]);
		let complete!: (value: number[]) => void;
		mocks.fetcher.mockReturnValue(
			new Promise<number[]>((resolve) => {
				complete = resolve;
			}),
		);
		const state = new BoardState(source as never);
		let resolved = false;
		const publication = state
			.createBoardVersion("app", "board", IVersionType.Minor)
			.then((version) => {
				resolved = true;
				return version;
			});

		await vi.waitFor(() => expect(mocks.fetcher).toHaveBeenCalledTimes(1));
		expect(resolved).toBe(false);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app/board/board?version_type=Minor",
		);
		complete([0, 1, 0]);
		expect(await publication).toEqual([0, 1, 0]);
		expect(source.backgroundTaskHandler).not.toHaveBeenCalled();
	});

	test("rejects a failed hosted publication even after local publication", async () => {
		mocks.invoke.mockResolvedValue([0, 0, 5]);
		const failure = { error: "Board is locked" };
		mocks.fetcher.mockRejectedValue(failure);
		const state = new BoardState(backend() as never);

		await expect(
			state.createBoardVersion("app", "board", IVersionType.Patch),
		).rejects.toBe(failure);
	});

	test("local-only publication completes with the native draft version", async () => {
		mocks.invoke.mockResolvedValue([0, 0, 5]);
		const state = new BoardState(backend(true) as never);

		expect(
			await state.createBoardVersion("app", "board", IVersionType.Patch),
		).toEqual([0, 0, 5]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
