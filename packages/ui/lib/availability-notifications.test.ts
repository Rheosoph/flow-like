import { describe, expect, test } from "bun:test";
import { DEFAULT_AVAILABILITY } from "./availability";
import { shouldPresentUserNotification } from "./availability-notifications";

describe("account and device focus notifications", () => {
	test("either active Focus suppresses routine alerts", () => {
		const focus = {
			...DEFAULT_AVAILABILITY,
			status: "focus" as const,
			until: 200,
		};
		expect(
			shouldPresentUserNotification(
				DEFAULT_AVAILABILITY,
				focus,
				"routine",
				100,
			),
		).toBe(false);
		expect(
			shouldPresentUserNotification(
				focus,
				DEFAULT_AVAILABILITY,
				"routine",
				100,
			),
		).toBe(false);
		expect(
			shouldPresentUserNotification(
				DEFAULT_AVAILABILITY,
				focus,
				"routine",
				200,
			),
		).toBe(true);
	});
	test("account exceptions apply to device Focus even while account status is available", () => {
		const account = {
			...DEFAULT_AVAILABILITY,
			preferences: { allow_calls: false, allow_mentions: true },
		};
		const focus = {
			...DEFAULT_AVAILABILITY,
			status: "focus" as const,
			until: 200,
		};
		expect(shouldPresentUserNotification(account, focus, "call", 100)).toBe(
			false,
		);
		expect(shouldPresentUserNotification(account, focus, "mention", 100)).toBe(
			true,
		);
	});
	test("signed-out device focus uses its own exceptions", () => {
		const focus = {
			...DEFAULT_AVAILABILITY,
			status: "focus" as const,
			until: 200,
			preferences: { allow_calls: true, allow_mentions: false },
		};
		expect(shouldPresentUserNotification(undefined, focus, "call", 100)).toBe(
			true,
		);
		expect(
			shouldPresentUserNotification(undefined, focus, "mention", 100),
		).toBe(false);
	});
});
