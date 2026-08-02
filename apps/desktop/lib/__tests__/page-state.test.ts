import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
}));

vi.mock("../api", () => ({
	fetcher: mocks.fetcher,
}));

import {
	PageState,
	isNativePageBoardUnavailableError,
	isNativePageNotFoundError,
} from "../../components/tauri-provider/page-state";

function offlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(true),
		profile: undefined,
		auth: undefined,
		backgroundTaskHandler: vi.fn(),
	};
}

describe("desktop page lookup errors", () => {
	beforeEach(() => {
		mocks.invoke.mockReset();
		mocks.fetcher.mockReset();
	});

	test("recognizes only authoritative native page misses", () => {
		expect(isNativePageNotFoundError(new Error("Page not found"))).toBe(true);
		expect(
			isNativePageNotFoundError({
				error: "Page not found in specified board",
			}),
		).toBe(true);
		expect(isNativePageNotFoundError("Page not found")).toBe(true);

		expect(
			isNativePageNotFoundError({
				error:
					"Failed to load page 'page-1' from board 'board-1': page not found",
			}),
		).toBe(false);
		expect(
			isNativePageNotFoundError({
				error: "Failed to open board 'board-1': Project store not found",
			}),
		).toBe(false);
	});

	test("recognizes the scoped native missing-board lookup error", () => {
		expect(
			isNativePageBoardUnavailableError({
				error:
					"Failed to open board 'board-1' while looking up page 'page-1': Project store not found",
			}),
		).toBe(true);
		expect(
			isNativePageBoardUnavailableError({
				error: "Failed to open board 'board-1': Project store not found",
			}),
		).toBe(false);
		expect(
			isNativePageBoardUnavailableError({
				error:
					"Failed to load page 'page-1' from board 'board-1': corrupt page payload",
			}),
		).toBe(false);
	});

	test("syncs a missing native board before retrying the page lookup", async () => {
		const nativeFailure = {
			error:
				"Failed to open board 'board-1' while looking up page 'page-1': Project store not found",
		};
		const page = { id: "page-1", boardId: "board-1" };
		mocks.invoke
			.mockRejectedValueOnce(nativeFailure)
			.mockResolvedValueOnce(page);
		const getBoard = vi.fn().mockResolvedValue({ id: "board-1" });
		const backend = {
			...offlineBackend(),
			boardState: { getBoard },
		};
		const state = new PageState(backend as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).resolves.toBe(
			page,
		);
		expect(getBoard).toHaveBeenCalledWith("app-1", "board-1", undefined, true);
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		expect(getBoard.mock.invocationCallOrder[0]).toBeGreaterThan(
			mocks.invoke.mock.invocationCallOrder[0],
		);
		expect(getBoard.mock.invocationCallOrder[0]).toBeLessThan(
			mocks.invoke.mock.invocationCallOrder[1],
		);
	});

	test("preserves the native board error when board repair fails", async () => {
		const nativeFailure = {
			error:
				"Failed to open board 'board-1' while looking up page 'page-1': Project store not found",
		};
		mocks.invoke.mockRejectedValueOnce(nativeFailure);
		const getBoard = vi
			.fn()
			.mockRejectedValue(new Error("network unavailable"));
		const backend = {
			...offlineBackend(),
			boardState: { getBoard },
		};
		const state = new PageState(backend as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).rejects.toBe(
			nativeFailure,
		);
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("preserves a non-not-found native failure without trying remote fallback", async () => {
		const nativeFailure = {
			error:
				"Failed to load page 'page-1' from board 'board-1': corrupt page payload",
		};
		mocks.invoke.mockRejectedValueOnce(nativeFailure);
		const backend = offlineBackend();
		const state = new PageState(backend as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).rejects.toBe(
			nativeFailure,
		);
		expect(backend.isOffline).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("uses the remote/offline fallback only for an authoritative native miss", async () => {
		mocks.invoke.mockRejectedValueOnce({ error: "Page not found" });
		const backend = offlineBackend();
		const state = new PageState(backend as never);

		await expect(state.getPage("app-1", "page-1")).rejects.toThrow(
			"Page not found: page-1",
		);
		expect(backend.isOffline).toHaveBeenCalledWith("app-1");
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
