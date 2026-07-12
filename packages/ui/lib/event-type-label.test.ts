import { describe, expect, test } from "bun:test";
import { formatEventTypeLabel } from "./event-type-label";

describe("formatEventTypeLabel", () => {
	test("presents the persisted simple chat type as Chat UI", () => {
		expect(formatEventTypeLabel("simple_chat")).toBe("Chat UI");
	});

	test("title-cases event types without a custom label", () => {
		expect(formatEventTypeLabel("generic_form")).toBe("Generic Form");
	});
});
