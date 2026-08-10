import { beforeEach, describe, expect, test } from "bun:test";
import {
	boardReadinessKey,
	resetBoardReadiness,
	whenBoardReady,
} from "../../lib/board-readiness";
import {
	type IStoreRedirectState,
	loadPageWithBoardSync,
	pageLoadErrorMessage,
	resolveStoreRedirect,
} from "./use-page-content";

function redirectState(
	overrides: Partial<IStoreRedirectState> = {},
): IStoreRedirectState {
	return {
		embedded: false,
		authLoading: false,
		hasAccessToken: true,
		appInLocalProfile: true,
		localProfileCheckPending: false,
		remoteAppCheckPending: false,
		remoteAppLoaded: true,
		remoteAppFailed: false,
		eventsLoaded: true,
		eventsFailed: false,
		eventsFetching: false,
		offline: false,
		...overrides,
	};
}

describe("store redirect", () => {
	test("keeps a resolved interface when a refresh fails but data survived", () => {
		expect(
			resolveStoreRedirect(
				redirectState({ eventsFailed: true, eventsLoaded: true }),
			),
		).toEqual({ pending: false, redirect: false });
	});

	test("waits instead of ejecting while the catalog is still retrying", () => {
		expect(
			resolveStoreRedirect(
				redirectState({
					eventsFailed: true,
					eventsLoaded: false,
					eventsFetching: true,
				}),
			),
		).toEqual({ pending: false, redirect: false });
	});

	test("ejects once the catalog failed with nothing to render", () => {
		expect(
			resolveStoreRedirect(
				redirectState({ eventsFailed: true, eventsLoaded: false }),
			),
		).toEqual({ pending: false, redirect: true });
	});

	test("a locally installed app survives a failed hub lookup", () => {
		expect(
			resolveStoreRedirect(
				redirectState({
					appInLocalProfile: true,
					remoteAppLoaded: false,
					remoteAppFailed: true,
				}),
			),
		).toEqual({ pending: false, redirect: false });
	});

	test("ejects a signed-in user without local or remote access", () => {
		expect(
			resolveStoreRedirect(
				redirectState({
					appInLocalProfile: false,
					remoteAppLoaded: false,
					remoteAppFailed: true,
				}),
			),
		).toEqual({ pending: false, redirect: true });
	});

	test("ejects a signed-out user whose profiles do not contain the app", () => {
		expect(
			resolveStoreRedirect(
				redirectState({ hasAccessToken: false, appInLocalProfile: false }),
			),
		).toEqual({ pending: false, redirect: true });
	});

	test("holds every verdict while an access check is in flight", () => {
		for (const pendingFlag of [
			"authLoading",
			"localProfileCheckPending",
			"remoteAppCheckPending",
		] as const) {
			expect(
				resolveStoreRedirect(
					redirectState({
						[pendingFlag]: true,
						appInLocalProfile: false,
						remoteAppLoaded: false,
						remoteAppFailed: true,
					}),
				),
			).toEqual({ pending: true, redirect: false });
		}
	});

	test("never ejects an offline device to an unreachable store", () => {
		expect(
			resolveStoreRedirect(
				redirectState({
					offline: true,
					appInLocalProfile: false,
					remoteAppLoaded: false,
					remoteAppFailed: true,
					eventsFailed: true,
					eventsLoaded: false,
				}),
			),
		).toEqual({ pending: false, redirect: false });
	});

	test("still waits for a pending access check while offline", () => {
		expect(
			resolveStoreRedirect(
				redirectState({ offline: true, localProfileCheckPending: true }),
			),
		).toEqual({ pending: true, redirect: false });
	});

	test("never ejects an embedded interface", () => {
		expect(
			resolveStoreRedirect(
				redirectState({
					embedded: true,
					appInLocalProfile: false,
					remoteAppLoaded: false,
					remoteAppFailed: true,
					eventsFailed: true,
					eventsLoaded: false,
				}),
			),
		).toEqual({ pending: false, redirect: false });
	});
});

describe("page board synchronization", () => {
	beforeEach(() => {
		resetBoardReadiness();
	});

	test("reads the page without waiting for board synchronization", async () => {
		const calls: string[] = [];
		let finishBoardSync: (() => void) | undefined;
		const boardReady = new Promise<void>((resolve) => {
			finishBoardSync = resolve;
		});
		const page = { id: "page-1", boardId: "board-1" };
		const boardState = {
			async getBoard() {
				calls.push("board:start");
				await boardReady;
				calls.push("board:ready");
				return {};
			},
		};
		const pageState = {
			async getPage() {
				calls.push("page");
				return page;
			},
		};

		// The page resolves while the board refresh is still in flight.
		expect(
			await loadPageWithBoardSync(
				boardState as never,
				pageState as never,
				"app-1",
				"page-1",
				"board-1",
				[1, 2, 3],
			),
		).toBe(page);
		expect(calls).toEqual(["board:start", "page"]);

		// Execution still has something to wait on until the refresh finishes.
		const key = boardReadinessKey("app-1", "board-1", [1, 2, 3]);
		let ready = false;
		void whenBoardReady(key).then(() => {
			ready = true;
		});
		await Promise.resolve();
		expect(ready).toBe(false);

		finishBoardSync?.();
		await whenBoardReady(key);
		expect(calls).toEqual(["board:start", "page", "board:ready"]);
	});

	test("retries after board synchronization when the page cannot be read yet", async () => {
		const calls: string[] = [];
		const page = { id: "page-1", boardId: "board-1" };
		let attempts = 0;
		let markFirstAttemptFailed: (() => void) | undefined;
		const firstAttemptFailed = new Promise<void>((resolve) => {
			markFirstAttemptFailed = resolve;
		});
		// The refresh only lands after the first read has already failed, which is the ordering
		// a device that has not downloaded the board yet actually sees.
		const boardState = {
			async getBoard() {
				calls.push("board");
				await firstAttemptFailed;
				return {};
			},
		};
		const pageState = {
			async getPage() {
				calls.push("page");
				attempts += 1;
				if (attempts === 1) {
					markFirstAttemptFailed?.();
					throw new Error("board not on this device yet");
				}
				return page;
			},
		};

		expect(
			await loadPageWithBoardSync(
				boardState as never,
				pageState as never,
				"app-1",
				"page-1",
				"board-1",
			),
		).toBe(page);
		expect(calls).toEqual(["board", "page", "page"]);
	});

	test("reports the original failure when the retry fails too", async () => {
		const boardState = { async getBoard() {} };
		const pageState = {
			async getPage() {
				throw new Error("Page not found: page-1");
			},
		};

		await expect(
			loadPageWithBoardSync(
				boardState as never,
				pageState as never,
				"app-1",
				"page-1",
				"board-1",
			),
		).rejects.toThrow("Page not found: page-1");
	});

	test("reads the pinned version's page, not the draft board's", async () => {
		const page = { id: "page-1", boardId: "board-1" };
		const seen: unknown[] = [];
		const boardState = { async getBoard() {} };
		const pageState = {
			async getPage(...args: unknown[]) {
				seen.push(args);
				return page;
			},
		};

		await loadPageWithBoardSync(
			boardState as never,
			pageState as never,
			"app-1",
			"page-1",
			"board-1",
			[2, 1, 0],
		);

		expect(seen).toEqual([
			["app-1", "page-1", "board-1", [2, 1, 0], undefined],
		]);
	});

	test("still lets page state fall back when board synchronization fails", async () => {
		const calls: string[] = [];
		const page = { id: "page-1", boardId: "board-1" };
		const boardState = {
			async getBoard() {
				calls.push("board");
				throw new Error("board unavailable");
			},
		};
		const pageState = {
			async getPage() {
				calls.push("page");
				return page;
			},
		};

		await expect(
			loadPageWithBoardSync(
				boardState as never,
				pageState as never,
				"app-1",
				"page-1",
				"board-1",
			),
		).resolves.toBe(page);
		expect(calls).toEqual(["board", "page"]);
		// A failed refresh must not leave execution waiting forever.
		await whenBoardReady(boardReadinessKey("app-1", "board-1"));
	});
});

describe("page load failures", () => {
	test("keeps the native failure readable for the retry surface", () => {
		expect(
			pageLoadErrorMessage({
				error:
					"Failed to load page 'page-1' from board 'board-1': page page-1 not found at canonical path",
			}),
		).toContain("Failed to load page 'page-1'");
		expect(pageLoadErrorMessage(new Error("network unavailable"))).toBe(
			"network unavailable",
		);
		expect(pageLoadErrorMessage("boom")).toBe("boom");
		expect(pageLoadErrorMessage(undefined)).toBe("Unknown error");
	});
});
