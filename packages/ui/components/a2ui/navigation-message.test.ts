import { describe, expect, test } from "bun:test";
import {
	type A2UINavigationMessage,
	createNavigateToMessage,
	interceptA2UINavigationMessage,
} from "./navigation-message";
import type { A2UIServerMessage } from "./types";

describe("interceptA2UINavigationMessage", () => {
	test.each([
		{
			type: "navigateTo",
			route: "/orders",
			replace: false,
			queryParams: { status: "open" },
		},
		{
			type: "setQueryParam",
			key: "order",
			value: "42",
			replace: true,
		},
	] satisfies A2UINavigationMessage[])(
		"consumes $type when an embedded owner is present",
		(message) => {
			const received: A2UINavigationMessage[] = [];
			expect(
				interceptA2UINavigationMessage(message, (next) => received.push(next)),
			).toBe(true);
			expect(received).toEqual([message]);
		},
	);

	test("leaves navigation for the host router when no interceptor is present", () => {
		const message: A2UINavigationMessage = {
			type: "navigateTo",
			route: "/orders",
			replace: false,
		};
		expect(interceptA2UINavigationMessage(message)).toBe(false);
	});

	test("does not consume non-navigation messages", () => {
		const received: A2UINavigationMessage[] = [];
		const message: A2UIServerMessage = { type: "showScreen" };
		expect(
			interceptA2UINavigationMessage(message, (next) => received.push(next)),
		).toBe(false);
		expect(received).toEqual([]);
	});
});

describe("createNavigateToMessage", () => {
	test("normalizes a direct navigate_page action for the embedded owner", () => {
		expect(createNavigateToMessage("/orders", { status: "open" })).toEqual({
			type: "navigateTo",
			route: "/orders",
			replace: false,
			queryParams: { status: "open" },
		});
	});
});
