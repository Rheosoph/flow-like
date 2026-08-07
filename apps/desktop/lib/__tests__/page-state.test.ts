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
	isCachedPageOutdated,
	isNativePageBoardUnavailableError,
	isNativePageContentUnavailableError,
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

function onlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		profile: { hub: "hub.example" },
		auth: { user: { access_token: "token" } },
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

	test("recognizes an unreadable local page payload", () => {
		expect(
			isNativePageContentUnavailableError({
				error:
					"Failed to load page 'page-1' from board 'board-1': page page-1 not found at canonical path",
			}),
		).toBe(true);
		expect(
			isNativePageContentUnavailableError({ error: "Page not found" }),
		).toBe(false);
		expect(
			isNativePageContentUnavailableError({
				error:
					"Failed to open board 'board-1' while looking up page 'page-1': Project store not found",
			}),
		).toBe(false);
	});

	test("serves a page the local board lists but never downloaded", async () => {
		const nativeFailure = {
			error:
				"Failed to load page 'page-1' from board 'board-1': page page-1 not found at canonical path",
		};
		const remotePage = { id: "page-1", boardId: "board-1" };
		mocks.invoke
			.mockRejectedValueOnce(nativeFailure)
			.mockResolvedValueOnce(undefined);
		mocks.fetcher.mockResolvedValueOnce(remotePage);
		const state = new PageState(onlineBackend() as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).resolves.toEqual(
			remotePage,
		);
		expect(mocks.invoke).toHaveBeenLastCalledWith("update_page", {
			appId: "app-1",
			page: remotePage,
		});
	});

	test("caches a remote page whose payload omits its board id", async () => {
		mocks.invoke
			.mockRejectedValueOnce({ error: "Page not found in specified board" })
			.mockResolvedValueOnce(undefined);
		mocks.fetcher.mockResolvedValueOnce({ id: "page-1" });
		const state = new PageState(onlineBackend() as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).resolves.toEqual({
			id: "page-1",
			boardId: "board-1",
		});
		expect(mocks.invoke).toHaveBeenLastCalledWith("update_page", {
			appId: "app-1",
			page: { id: "page-1", boardId: "board-1" },
		});
	});

	test("preserves the native failure when the payload cannot be fetched", async () => {
		const nativeFailure = {
			error:
				"Failed to load page 'page-1' from board 'board-1': corrupt page payload",
		};
		mocks.invoke.mockRejectedValueOnce(nativeFailure);
		const state = new PageState(offlineBackend() as never);

		await expect(state.getPage("app-1", "page-1", "board-1")).rejects.toBe(
			nativeFailure,
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("preserves an unrelated native failure without trying remote fallback", async () => {
		const nativeFailure = { error: "Project store not found" };
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

describe("versioned page reads", () => {
	beforeEach(() => {
		mocks.invoke.mockReset();
		mocks.fetcher.mockReset();
	});

	test("asks the native board for the pinned snapshot", async () => {
		const page = { id: "page-1", boardId: "board-1" };
		mocks.invoke.mockResolvedValueOnce(page);
		const state = new PageState(onlineBackend() as never);

		await expect(
			state.getPage("app-1", "page-1", "board-1", [2, 1, 0]),
		).resolves.toBe(page);
		expect(mocks.invoke).toHaveBeenCalledWith("get_page", {
			appId: "app-1",
			pageId: "page-1",
			boardId: "board-1",
			version: [2, 1, 0],
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("falls back to the server snapshot without overwriting the draft page", async () => {
		const remotePage = { id: "page-1", boardId: "board-1" };
		mocks.invoke.mockRejectedValueOnce({
			error:
				"Failed to load page 'page-1' from board 'board-1': version 2.1.0 not found",
		});
		mocks.fetcher.mockResolvedValueOnce(remotePage);
		const state = new PageState(onlineBackend() as never);

		await expect(
			state.getPage("app-1", "page-1", "board-1", [2, 1, 0]),
		).resolves.toBe(remotePage);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/pages/page-1?board_id=board-1&version=2_1_0",
		);
		expect(
			mocks.invoke.mock.calls.some(([command]) => command === "update_page"),
		).toBe(false);
	});

	test("serves the last known page when the snapshot is unreachable offline", async () => {
		const currentPage = { id: "page-1", boardId: "board-1" };
		mocks.invoke
			.mockRejectedValueOnce({
				error:
					"Failed to load page 'page-1' from board 'board-1': version 2.1.0 not found",
			})
			.mockResolvedValueOnce(currentPage);
		const state = new PageState(offlineBackend() as never);

		await expect(
			state.getPage("app-1", "page-1", "board-1", [2, 1, 0]),
		).resolves.toBe(currentPage);
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(mocks.invoke).toHaveBeenLastCalledWith("get_page", {
			appId: "app-1",
			pageId: "page-1",
			boardId: "board-1",
			version: undefined,
		});
	});

	test("reports the version failure when nothing at all is readable", async () => {
		const versionFailure = { error: "Page not found" };
		mocks.invoke.mockRejectedValue(versionFailure);
		const state = new PageState(offlineBackend() as never);

		await expect(
			state.getPage("app-1", "page-1", "board-1", [2, 1, 0]),
		).rejects.toBe(versionFailure);
	});
});

describe("cached page freshness", () => {
	const remote = {
		appId: "app-1",
		pageId: "page-1",
		name: "Page",
		updatedAt: "2026-08-06T10:00:00Z",
	};

	test("downloads a page this device has never seen", () => {
		expect(isCachedPageOutdated(undefined, remote)).toBe(true);
	});

	test("refreshes a cached page the server changed later", () => {
		expect(
			isCachedPageOutdated(
				{ ...remote, updatedAt: "2026-08-06T09:00:00Z" },
				remote,
			),
		).toBe(true);
	});

	test("leaves an identical or newer local revision alone", () => {
		expect(isCachedPageOutdated(remote, remote)).toBe(false);
		expect(
			isCachedPageOutdated(
				{ ...remote, updatedAt: "2026-08-06T11:00:00Z" },
				remote,
			),
		).toBe(false);
	});

	test("refreshes once when the cached copy predates revision tracking", () => {
		expect(
			isCachedPageOutdated({ ...remote, updatedAt: undefined }, remote),
		).toBe(true);
	});

	test("replaces an unreadable local payload whatever the revisions claim", () => {
		expect(isCachedPageOutdated({ ...remote, unavailable: true }, remote)).toBe(
			true,
		);
		expect(
			isCachedPageOutdated(
				{ ...remote, unavailable: true },
				{
					...remote,
					updatedAt: undefined,
				},
			),
		).toBe(true);
	});

	test("never churns when the listing carries no revision", () => {
		expect(
			isCachedPageOutdated(remote, { ...remote, updatedAt: undefined }),
		).toBe(false);
		expect(
			isCachedPageOutdated(
				{ ...remote, updatedAt: "not-a-date" },
				{ ...remote, updatedAt: "also-not-a-date" },
			),
		).toBe(false);
	});
});
