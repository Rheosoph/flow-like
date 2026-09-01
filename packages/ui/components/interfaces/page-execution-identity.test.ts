import { describe, expect, test } from "bun:test";
import { pageExecutionIdentity } from "./page-execution-identity";

describe("Page lifecycle execution identity", () => {
	test("uses the governed Event when runtime bootstrap redacts the Board", () => {
		expect(pageExecutionIdentity(undefined, "page-event")).toBe(
			"event:page-event",
		);
	});

	test("preserves the local Board path and rejects an ungoverned missing target", () => {
		expect(pageExecutionIdentity("board-1", "page-event")).toBe(
			"board:board-1",
		);
		expect(pageExecutionIdentity(undefined, undefined)).toBeUndefined();
	});
});
