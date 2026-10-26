import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { beforeEach, describe, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({ apiPost: vi.fn(), apiGet: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", () => ({
	finishAllProgressToasts: vi.fn(),
}));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({}));
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-tool-registry",
	() => ({ runGlobalChatTool: vi.fn() }),
);
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport",
	() => ({ dispatchSpecialistToolRequest: vi.fn() }),
);
vi.mock("./api-utils", () => ({
	apiPost: mocks.apiPost,
	apiGet: mocks.apiGet,
	getApiBaseUrl: () => "https://api.example.test",
}));
import { WebBoardState } from "./board-state";

const board = {
	id: "board",
	name: "Board",
	nodes: {},
	layers: {},
	comments: {},
	variables: {},
	refs: {},
} as unknown as IBoard;

const routeMissing = () =>
	new ApiResponseError({ status: 404, message: "Not Found" });

beforeEach(() => {
	mocks.apiPost.mockReset();
	mocks.apiGet.mockReset();
});

describe("web board sync fallback", () => {
	test("a hub without /sync serves the whole board and is not asked again", async () => {
		mocks.apiPost.mockRejectedValue(routeMissing());
		mocks.apiGet.mockResolvedValue(board);
		const state = new WebBoardState({ auth: {} } as never);

		expect(await state.getBoard("app", "board", [1, 2, 3])).toBe(board);
		expect(mocks.apiGet.mock.calls[0][0]).toBe(
			"apps/app/board/board?version=1_2_3",
		);

		await state.getBoard("app", "board");
		expect(mocks.apiPost).toHaveBeenCalledTimes(1);
		expect(mocks.apiGet.mock.calls[1][0]).toBe("apps/app/board/board");
	});

	test("a missing board stays an error", async () => {
		const missing = new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "Board board not found",
		});
		mocks.apiPost.mockRejectedValue(missing);
		const state = new WebBoardState({ auth: {} } as never);

		await expect(state.getBoard("app", "board")).rejects.toBe(missing);
		expect(mocks.apiGet).not.toHaveBeenCalled();
	});
});
