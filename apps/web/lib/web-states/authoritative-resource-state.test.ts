import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	applyWidgetRename: vi.fn(),
	normalizePageForPersistence: (page: unknown) => page,
	normalizeWidgetForPersistence: (widget: unknown) => widget,
}));

vi.mock("./api-utils", () => ({
	apiDelete: vi.fn(),
	apiGet: mocks.apiGet,
	apiPost: vi.fn(),
	apiPut: vi.fn(),
}));

vi.mock("../oauth-db", () => ({
	oauthConsentStore: {},
	oauthTokenStore: {},
}));

import { WebBoardState } from "./board-state";
import { WebDatabaseState } from "./database-state";
import { WebEventState } from "./event-state";
import { WebPageState } from "./page-state";
import { WebRouteState } from "./route-state";
import { WebWidgetState } from "./widget-state";

const backend = { auth: { token: "test" } } as never;

describe("web authoritative resource reads", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test("propagates Page inventory failures instead of reporting an empty app", async () => {
		const failure = new Error("authority unavailable");
		mocks.apiGet.mockRejectedValueOnce(failure);

		await expect(
			new WebPageState(backend).getPagesAuthoritative("app-1", "board-1"),
		).rejects.toBe(failure);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"apps/app-1/pages?board_id=board-1",
			{ token: "test" },
		);
	});

	test("does not replace a missing Page version with the current Page", async () => {
		const failure = new Error("version missing");
		mocks.apiGet.mockRejectedValueOnce(failure);

		await expect(
			new WebPageState(backend).getPageAuthoritative(
				"app-1",
				"page-1",
				"board-1",
				[2, 1, 0],
			),
		).rejects.toBe(failure);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet.mock.calls[0][0]).toBe(
			"apps/app-1/pages/page-1?board_id=board-1&version=2_1_0",
		);
	});

	test("propagates Widget inventory failures instead of reporting an empty app", async () => {
		const failure = new Error("authority unavailable");
		mocks.apiGet.mockRejectedValueOnce(failure);

		await expect(
			new WebWidgetState(backend).getWidgetsAuthoritative("app-1", "de"),
		).rejects.toBe(failure);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"apps/app-1/widgets?language=de",
			{ token: "test" },
		);
	});

	test("requests the exact Widget version once", async () => {
		const widget = { id: "widget-1" };
		mocks.apiGet.mockResolvedValueOnce(widget);

		await expect(
			new WebWidgetState(backend).getWidgetAuthoritative(
				"app-1",
				"widget-1",
				[3, 2, 1],
			),
		).resolves.toBe(widget);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet.mock.calls[0][0]).toBe(
			"apps/app-1/widgets/widget-1?version=3_2_1",
		);
	});

	test("propagates database inventory failures instead of reporting no tables", async () => {
		const failure = new Error("authority unavailable");
		mocks.apiGet.mockRejectedValueOnce(failure);

		await expect(
			new WebDatabaseState(backend).listTablesAuthoritative("app-1"),
		).rejects.toBe(failure);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet).toHaveBeenCalledWith("apps/app-1/db", {
			token: "test",
		});
	});

	test("reads the exact encoded database schema once", async () => {
		const schema = { fields: [] };
		mocks.apiGet.mockResolvedValueOnce(schema);

		await expect(
			new WebDatabaseState(backend).getSchemaAuthoritative(
				"app-1",
				"support tickets",
			),
		).resolves.toBe(schema);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet.mock.calls[0][0]).toBe(
			"apps/app-1/db/support%20tickets/schema",
		);
	});

	test("propagates route authority failures instead of reporting a missing route", async () => {
		const failure = new Error("authority unavailable");
		mocks.apiGet.mockRejectedValueOnce(failure);

		await expect(
			new WebRouteState(backend).getRouteByPathAuthoritative(
				"app-1",
				"/support tickets",
			),
		).rejects.toBe(failure);
		expect(mocks.apiGet).toHaveBeenCalledOnce();
		expect(mocks.apiGet.mock.calls[0][0]).toBe(
			"apps/app-1/routes/by-path?path=%2Fsupport%20tickets",
		);
	});

	test("reads exact Board artifacts without incremental display state", async () => {
		const board = { id: "board-1", nodes: {} };
		mocks.apiGet
			.mockResolvedValueOnce([{ id: "board-1" }])
			.mockResolvedValueOnce(board)
			.mockResolvedValueOnce({ flowscript: "event entry() {}\n" });
		const state = new WebBoardState(backend);

		await expect(
			state.getBoardSummariesAuthoritative("app-1", ["node_types"]),
		).resolves.toEqual([{ id: "board-1" }]);
		await expect(
			state.getBoardAuthoritative("app-1", "board-1", [2, 0, 1]),
		).resolves.toBe(board);
		await expect(
			state.getFlowScriptAuthoritative("app-1", "board-1", [2, 0, 1]),
		).resolves.toBe("event entry() {}\n");
		expect(mocks.apiGet.mock.calls.map(([path]) => path)).toEqual([
			"apps/app-1/board/summaries?include=node_types",
			"apps/app-1/board/board-1?version=2_0_1",
			"apps/app-1/board/board-1/flowscript?version=2_0_1&anchors=true",
		]);
	});

	test("rejects an empty FlowScript body instead of rendering undefined", async () => {
		mocks.apiGet
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce({ flowscript: "fn a() {}\n" });
		const state = new WebBoardState(backend);

		await expect(
			state.getFlowScriptFile("app-1", "board-1", "main"),
		).rejects.toThrow("The hub returned no FlowScript for board board-1");
		await expect(
			state.getFlowScriptScoped("app-1", "board-1", ["node-1"]),
		).rejects.toThrow("The hub returned no FlowScript for board board-1");
		await expect(
			state.getFlowScriptFile("app-1", "board-1", "main"),
		).resolves.toEqual({ flowscript: "fn a() {}\n", scope_anchors: [] });
	});

	test("propagates authoritative Board and Event failures", async () => {
		const boardFailure = new Error("board authority unavailable");
		const eventFailure = new Error("event authority unavailable");
		mocks.apiGet
			.mockRejectedValueOnce(boardFailure)
			.mockRejectedValueOnce(eventFailure);

		await expect(
			new WebBoardState(backend).getBoardAuthoritative("app-1", "board-1"),
		).rejects.toBe(boardFailure);
		await expect(
			new WebEventState(backend).getEventsAuthoritative("app-1"),
		).rejects.toBe(eventFailure);
		expect(mocks.apiGet).toHaveBeenCalledTimes(2);
	});

	test("requests one exact authoritative Event version", async () => {
		const event = { id: "event-1", active: false };
		mocks.apiGet.mockResolvedValueOnce(event);

		await expect(
			new WebEventState(backend).getEventAuthoritative(
				"app-1",
				"event-1",
				[4, 1, 0],
			),
		).resolves.toBe(event);
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"apps/app-1/events/event-1?version=4_1_0",
			{ token: "test" },
		);
	});
});
