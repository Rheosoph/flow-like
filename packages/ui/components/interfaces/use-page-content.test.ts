import { beforeEach, describe, expect, test } from "bun:test";
import {
	boardReadinessKey,
	resetBoardReadiness,
	whenBoardReady,
} from "../../lib/board-readiness";
import {
	type IStoreRedirectState,
	deriveRouteMappings,
	isUsableRuntimeEvent,
	loadPageWithBoardSync,
	pageLoadErrorMessage,
	resolveRouteMapping,
	resolveStoreRedirect,
	runtimeBootstrapTarget,
} from "./use-page-content";

const ROUTES = [
	{ path: "/", eventId: "home" },
	{ path: "/config", eventId: "config" },
];

describe("route mapping resolution", () => {
	test("derives explicit mappings from the event catalog", () => {
		expect(
			deriveRouteMappings([
				{ id: "home", route: "/", is_default: true },
				{ id: "config", route: "/config" },
			] as never),
		).toEqual([
			{ path: "/", eventId: "home" },
			{ path: "/config", eventId: "config" },
		]);
	});

	test("derives the root mapping when a default event has no route", () => {
		expect(
			deriveRouteMappings([
				{ id: "home", is_default: true },
				{ id: "other", route: "/other" },
			] as never),
		).toEqual([
			{ path: "/other", eventId: "other" },
			{ path: "/", eventId: "home" },
		]);
	});

	test("keeps the first event for duplicate canonical paths", () => {
		expect(
			deriveRouteMappings([
				{ id: "first", route: "/config/" },
				{ id: "second", route: "config" },
				{ id: "default", is_default: true },
			] as never),
		).toEqual([
			{ path: "/config/", eventId: "first" },
			{ path: "/", eventId: "default" },
		]);
	});

	test("prefers an explicit root route over a synthesized default", () => {
		expect(
			deriveRouteMappings([
				{ id: "default", is_default: true },
				{ id: "home", route: "/" },
			] as never),
		).toEqual([{ path: "/", eventId: "home" }]);
	});

	test("matches the requested route", () => {
		expect(resolveRouteMapping(ROUTES, "/config")).toEqual({
			mapping: { path: "/config", eventId: "config" },
			missed: false,
		});
	});

	test("matches regardless of trailing or leading slash", () => {
		for (const requested of ["/config/", "config", "config/"]) {
			expect(resolveRouteMapping(ROUTES, requested).mapping?.eventId).toBe(
				"config",
			);
		}
		expect(
			resolveRouteMapping([{ path: "config", eventId: "config" }], "/config")
				.mapping?.eventId,
		).toBe("config");
	});

	test("an unknown route falls back to the default but reports the miss", () => {
		expect(resolveRouteMapping(ROUTES, "/missing")).toEqual({
			mapping: { path: "/", eventId: "home" },
			missed: true,
		});
	});

	test("the default route is not a miss", () => {
		for (const requested of ["/", "", null, undefined]) {
			expect(resolveRouteMapping(ROUTES, requested)).toEqual({
				mapping: { path: "/", eventId: "home" },
				missed: false,
			});
		}
	});

	test("reports a miss even when there is no default route to fall back to", () => {
		expect(
			resolveRouteMapping([{ path: "/other", eventId: "other" }], "/config"),
		).toEqual({ mapping: null, missed: true });
	});

	test("an empty route list yields no mapping", () => {
		expect(resolveRouteMapping([], "/config")).toEqual({
			mapping: null,
			missed: true,
		});
		expect(resolveRouteMapping([], "/")).toEqual({
			mapping: null,
			missed: false,
		});
	});
});

describe("runtime event eligibility", () => {
	const eventTypes = new Set(["generic_form"]);

	test("never treats an inactive Event as a runnable interface", () => {
		expect(
			isUsableRuntimeEvent(
				{
					id: "inactive-page",
					active: false,
					default_page_id: "page",
				} as never,
				eventTypes,
			),
		).toBe(false);
		expect(
			isUsableRuntimeEvent(
				{
					id: "inactive-form",
					active: false,
					event_type: "generic_form",
				} as never,
				eventTypes,
			),
		).toBe(false);
	});
});

describe("runtime bootstrap target", () => {
	test("uses the route when route resolution has precedence", () => {
		expect(runtimeBootstrapTarget("/dashboard", "event-1", false)).toEqual({
			route: "/dashboard",
		});
	});

	test("uses only the direct Event when the URL did not name a route", () => {
		expect(runtimeBootstrapTarget("/", "event-1", true)).toEqual({
			eventId: "event-1",
		});
	});
});

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

	test("surfaces why the board never arrived, not just that the file is missing", () => {
		// Without the cause, a stale offline flag, a failed local write and a server that no
		// longer has the board all read as the same "file not found" on the card.
		const message = pageLoadErrorMessage(
			new Error(
				"Failed to open board 'board-1' while looking up page 'page-1': not found",
				{
					cause: new Error(
						"Board board-1 could not be made available (persist)",
					),
				},
			),
		);

		expect(message).toContain("Failed to open board 'board-1'");
		expect(message).toContain("(persist)");
	});

	test("does not repeat a cause the outer message already states", () => {
		expect(
			pageLoadErrorMessage(
				new Error("board unavailable: hub unreachable", {
					cause: new Error("hub unreachable"),
				}),
			),
		).toBe("board unavailable: hub unreachable");
	});
});

describe("board sync retry diagnostics", () => {
	test("a retry that fails differently is kept as the cause", async () => {
		const boardState = {
			async getBoard() {
				throw new Error("board unavailable");
			},
		};
		let attempt = 0;
		const pageState = {
			async getPage() {
				attempt += 1;
				throw new Error(
					attempt === 1 ? "board file missing" : "hub rejected the page",
				);
			},
		};

		const error: unknown = await loadPageWithBoardSync(
			boardState as never,
			pageState as never,
			"app-1",
			"page-1",
			"board-2",
		).catch((e: unknown) => e);

		// The first failure stays the headline — it is the precise one — while the retry's
		// distinct failure is preserved instead of discarded.
		expect((error as Error).message).toBe("board file missing");
		expect(((error as Error).cause as Error).message).toBe(
			"hub rejected the page",
		);
	});
});
