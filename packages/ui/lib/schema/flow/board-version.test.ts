import { describe, expect, test } from "bun:test";
import {
	normalizeBoardVersion,
	resolveEventBoardVersion,
	withBoardVersion,
} from "./board-version";

describe("page event board versions", () => {
	test("forwards an explicit pin for the event board", () => {
		const version = resolveEventBoardVersion("board-a", [1, 2, 3], "board-a");
		const payload = withBoardVersion({ id: "node", payload: {} }, version);

		expect(payload).toEqual({
			id: "node",
			payload: {},
			version: [1, 2, 3],
		});
	});

	test("represents latest by omitting version", () => {
		const payload = withBoardVersion(
			{ id: "node", payload: {} },
			resolveEventBoardVersion("board-a", null, "board-a"),
		);

		expect(payload).toEqual({ id: "node", payload: {} });
		expect("version" in payload).toBe(false);
	});

	test("does not leak a page event pin to another board", () => {
		expect(
			resolveEventBoardVersion("board-a", [1, 2, 3], "board-b"),
		).toBeUndefined();
	});

	test("rejects malformed versions instead of accidentally pinning", () => {
		expect(normalizeBoardVersion([1, 2])).toBeUndefined();
		expect(normalizeBoardVersion([1, 2, -1])).toBeUndefined();
		expect(normalizeBoardVersion([1, 2, 3.5])).toBeUndefined();
	});
});
