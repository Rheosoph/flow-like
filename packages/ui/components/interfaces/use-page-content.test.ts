import { describe, expect, test } from "bun:test";
import { loadPageAfterBoardSync } from "./use-page-content";

describe("page board synchronization", () => {
	test("waits for board synchronization before reading the page", async () => {
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

		const loading = loadPageAfterBoardSync(
			boardState as never,
			pageState as never,
			"app-1",
			"page-1",
			"board-1",
			[1, 2, 3],
		);

		await Promise.resolve();
		expect(calls).toEqual(["board:start"]);
		finishBoardSync?.();
		expect(await loading).toBe(page);
		expect(calls).toEqual(["board:start", "board:ready", "page"]);
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
			loadPageAfterBoardSync(
				boardState as never,
				pageState as never,
				"app-1",
				"page-1",
				"board-1",
			),
		).resolves.toBe(page);
		expect(calls).toEqual(["board", "page"]);
	});
});
